//! TCP and WebSocket listeners; per-connection handler; dispatch to app/room.
//!
//! Locking strategy: the `AppRegistry` is a `DashMap<String, RoomPool>` (sharded concurrent
//! map). Each `RoomPool` holds a `DashMap<RoomCode, Arc<Mutex<Room>>>`. Operations only lock
//! the individual room they touch — no global serialization point.

mod dispatch;
mod http;
mod validation;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

use crate::protocol::frame;
use crate::protocol::messages::{ClientMessage, ProtocolMessage, RoomEvent, ServerMessage};
use crate::room::pool::{PeerId, RoomPool};
use crate::room::room::{Room, RoomCode};

/// Idle connection timeout; closes connections that send no frames within this period.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Bounded channel capacity per connection. Provides backpressure: if a client's outbound
/// queue fills up, new messages are dropped rather than consuming unbounded memory.
const CHANNEL_CAPACITY: usize = 256;

/// Maximum concurrent connections before the server rejects new ones.
const MAX_CONNECTIONS: usize = 110_000;

/// Per-connection rate limit: max messages per 1-second window.
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
const RATE_LIMIT_MAX: u32 = 200;

/// Shared app registry: app_key -> RoomPool.
pub type AppRegistry = Arc<DashMap<String, RoomPool>>;

struct ConnectionGuard(Arc<AtomicUsize>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Bind and run the server. Accepts TCP on `tcp_bind`; if `ws_bind` is Some, also accepts
/// WebSocket connections on that address. If `http_bind` is Some, serves JSON stats on HTTP.
pub async fn run(
    tcp_bind: &str,
    ws_bind: Option<&str>,
    http_bind: Option<&str>,
) -> std::io::Result<()> {
    let tcp_listener = TcpListener::bind(tcp_bind).await?;
    eprintln!("roomie TCP listening on {}", tcp_bind);

    let registry: AppRegistry = Arc::new(DashMap::new());
    let conn_count = Arc::new(AtomicUsize::new(0));

    if let Some(addr) = http_bind {
        let addr = addr.to_string();
        let registry_http = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Err(e) = http::run_http(&addr, registry_http).await {
                eprintln!("HTTP server error: {e}");
            }
        });
    }

    if let Some(addr) = ws_bind {
        let ws_listener = TcpListener::bind(addr).await?;
        eprintln!("roomie WebSocket listening on {}", addr);
        let registry_ws = Arc::clone(&registry);
        let conn_count_ws = Arc::clone(&conn_count);
        tokio::spawn(async move {
            loop {
                let (stream, _addr) = match ws_listener.accept().await {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("WS accept error: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let prev = conn_count_ws.fetch_add(1, Ordering::Relaxed);
                if prev >= MAX_CONNECTIONS {
                    conn_count_ws.fetch_sub(1, Ordering::Relaxed);
                    drop(stream);
                    continue;
                }
                crate::debug_println!("WebSocket connection from {}", _addr);
                let registry = Arc::clone(&registry_ws);
                let guard = ConnectionGuard(Arc::clone(&conn_count_ws));
                tokio::spawn(async move {
                    let _guard = guard;
                    if let Err(_e) = handle_ws_connection(stream, registry).await {
                        crate::debug_println!("WebSocket connection error: {}", _e);
                    }
                });
            }
        });
    }

    loop {
        tokio::select! {
            accept_res = tcp_listener.accept() => {
                let (stream, _addr) = match accept_res {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("TCP accept error: {e} — retrying");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                let prev = conn_count.fetch_add(1, Ordering::Relaxed);
                if prev >= MAX_CONNECTIONS {
                    conn_count.fetch_sub(1, Ordering::Relaxed);
                    drop(stream);
                    continue;
                }
                crate::debug_println!("TCP connection from {}", _addr);
                let registry = Arc::clone(&registry);
                let guard = ConnectionGuard(Arc::clone(&conn_count));
                tokio::spawn(async move {
                    let _guard = guard;
                    if let Err(_e) = handle_connection(stream, registry).await {
                        crate::debug_println!("connection error: {}", _e);
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                crate::debug_println!("shutting down");
                break;
            }
        }
    }
    Ok(())
}

/// Per-connection state: which room we're in (if any).
pub(crate) struct Session {
    app_key: String,
    room_code: RoomCode,
    peer_id: PeerId,
}

/// Look up a room's `Arc<Mutex<Room>>` from the registry. Returns `None` if the app or
/// room doesn't exist. Only holds DashMap shard locks momentarily while cloning the Arc.
pub(crate) fn lookup_room(
    registry: &AppRegistry,
    app_key: &str,
    room_code: &RoomCode,
) -> Option<Arc<Mutex<Room>>> {
    let pool = registry.get(app_key)?;
    pool.get_room(room_code)
}

/// Broadcast to room, remove dead channels, broadcast PeerLeft and HostTransferred as needed.
pub(crate) fn broadcast_and_prune(room: &mut Room, msg: &ServerMessage) {
    let (mut dead, mut new_host) = room.broadcast_retain_connected(msg);
    while !dead.is_empty() {
        let mut next_dead = vec![];
        for pid in dead {
            let (d, h) = room.broadcast_retain_connected(&ServerMessage::PeerLeft { peer_id: pid });
            next_dead.extend(d);
            if h.is_some() {
                new_host = h;
            }
        }
        dead = next_dead;
    }
    if let Some(nh) = new_host {
        let _ = room.broadcast_retain_connected(&ServerMessage::RoomEvent(
            RoomEvent::HostTransferred(nh),
        ));
    }
}

/// Broadcast state update, remove dead channels, propagate PeerLeft/HostTransferred.
/// Uses `room.state` directly — caller must set it before calling.
pub(crate) fn broadcast_state_update_and_prune(room: &mut Room) {
    let (mut dead, mut new_host) = room.broadcast_state_update_retain_connected();
    while !dead.is_empty() {
        let mut next_dead = vec![];
        for pid in dead {
            let (d, h) = room.broadcast_retain_connected(&ServerMessage::PeerLeft { peer_id: pid });
            next_dead.extend(d);
            if h.is_some() {
                new_host = h;
            }
        }
        dead = next_dead;
    }
    if let Some(nh) = new_host {
        let _ = room.broadcast_retain_connected(&ServerMessage::RoomEvent(
            RoomEvent::HostTransferred(nh),
        ));
    }
}

/// Handle one WebSocket connection. Each Binary message = one frame (4-byte LE length + payload), same as TCP.
async fn handle_ws_connection(stream: TcpStream, registry: AppRegistry) -> std::io::Result<()> {
    let ws_stream: WebSocketStream<TcpStream> = accept_async(stream)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let (mut ws_send, mut ws_recv) = ws_stream.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(CHANNEL_CAPACITY);
    let mut session: Option<Session> = None;
    let mut result: std::io::Result<()> = Ok(());
    let mut idle_deadline = Instant::now() + CONNECTION_IDLE_TIMEOUT;
    let mut rate_count: u32 = 0;
    let mut rate_window_start = Instant::now();

    loop {
        tokio::select! {
            msg_res = ws_recv.next() => {
                let msg = match msg_res {
                    Some(Ok(Message::Binary(data))) => data,
                    Some(Ok(Message::Close(_))) | Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        result = Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                        break;
                    }
                    None => break,
                };
                if msg.len() < 4 {
                    let _ = tx.try_send(ServerMessage::Error { message: "frame too short".into() });
                    continue;
                }
                let len = u32::from_le_bytes(msg[0..4].try_into().unwrap()) as usize;
                if len > frame::MAX_FRAME_LEN || msg.len() != 4 + len {
                    let _ = tx.try_send(ServerMessage::Error { message: "invalid frame length".into() });
                    continue;
                }
                let payload = bytes::Bytes::copy_from_slice(&msg[4..]);
                let now = Instant::now();
                if now.duration_since(rate_window_start) >= RATE_LIMIT_WINDOW {
                    rate_count = 0;
                    rate_window_start = now;
                }
                rate_count += 1;
                if rate_count > RATE_LIMIT_MAX {
                    break;
                }
                let client_msg = match ClientMessage::decode(&payload) {
                    Ok(m) => m,
                    Err(_e) => {
                        crate::debug_println!("decode error (payload {} bytes)", payload.len());
                        let _ = tx.try_send(ServerMessage::Error { message: "invalid message".into() });
                        continue;
                    }
                };
                crate::debug_println!("recv: {:?}", client_msg);
                idle_deadline = Instant::now() + CONNECTION_IDLE_TIMEOUT;
                match dispatch::dispatch(&registry, &mut session, &tx, client_msg) {
                    Ok(to_send) => {
                        for s in to_send {
                            let _ = tx.try_send(s);
                        }
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            Some(out) = rx.recv() => {
                let payload: Vec<u8> = match out.encode() {
                    Ok(p) => p,
                    Err(e) => { result = Err(e); break; }
                };
                let mut frame = Vec::with_capacity(4 + payload.len());
                frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                frame.extend_from_slice(&payload);
                if let Err(e) = ws_send.send(Message::Binary(frame.into())).await {
                    result = Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, e));
                    break;
                }
            }
            _ = sleep_until(idle_deadline) => {
                break;
            }
        }
    }
    if let Some(s) = session.take() {
        crate::debug_println!("disconnect app={} room={:?} peer_id={}", s.app_key, s.room_code, s.peer_id);
        leave_room(&registry, &s.app_key, &s.room_code, s.peer_id);
    }
    result
}

/// Handle one TCP connection: framed read/write and dispatch to rooms.
async fn handle_connection(stream: TcpStream, registry: AppRegistry) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(CHANNEL_CAPACITY);
    let codec = frame::frame_codec();
    let mut framed = tokio_util::codec::Framed::new(stream, codec);
    let mut session: Option<Session> = None;
    let mut result: std::io::Result<()> = Ok(());
    let mut idle_deadline = Instant::now() + CONNECTION_IDLE_TIMEOUT;
    let mut rate_count: u32 = 0;
    let mut rate_window_start = Instant::now();

    loop {
        tokio::select! {
            frame_res = framed.next() => {
                let payload: bytes::Bytes = match frame_res {
                    Some(Ok(p)) => p.into(),
                    Some(Err(e)) => {
                        result = Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                        break;
                    }
                    None => break,
                };
                let now = Instant::now();
                if now.duration_since(rate_window_start) >= RATE_LIMIT_WINDOW {
                    rate_count = 0;
                    rate_window_start = now;
                }
                rate_count += 1;
                if rate_count > RATE_LIMIT_MAX {
                    break;
                }
                let msg = match ClientMessage::decode(&payload) {
                    Ok(m) => m,
                    Err(_e) => {
                        crate::debug_println!("decode error: {} (payload {} bytes: {:02x?})", _e, payload.len(), &payload[..payload.len().min(64)]);
                        let _ = tx.try_send(ServerMessage::Error { message: "invalid message".into() });
                        continue;
                    }
                };
                crate::debug_println!("recv: {:?}", msg);
                idle_deadline = Instant::now() + CONNECTION_IDLE_TIMEOUT;
                match dispatch::dispatch(&registry, &mut session, &tx, msg) {
                    Ok(to_send) => {
                        for s in to_send {
                            let _ = tx.try_send(s);
                        }
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            Some(out) = rx.recv() => {
                let payload: Vec<u8> = match out.encode() {
                    Ok(p) => p,
                    Err(e) => { result = Err(e); break; }
                };
                if let Err(e) = framed.send(bytes::Bytes::from(payload)).await {
                    result = Err(e);
                    break;
                }
            }
            _ = sleep_until(idle_deadline) => {
                break;
            }
        }
    }
    if let Some(s) = session.take() {
        crate::debug_println!("disconnect app={} room={:?} peer_id={}", s.app_key, s.room_code, s.peer_id);
        leave_room(&registry, &s.app_key, &s.room_code, s.peer_id);
    }
    result
}

/// Remove a peer from their room. All operations are synchronous (std::sync::Mutex +
/// DashMap), so this never holds locks across await points.
pub(crate) fn leave_room(
    registry: &AppRegistry,
    app_key: &str,
    room_code: &RoomCode,
    peer_id: PeerId,
) {
    crate::debug_println!("client left app={} room={:?} peer_id={}", app_key, room_code, peer_id);
    let room_arc = match lookup_room(registry, app_key, room_code) {
        Some(arc) => arc,
        None => return,
    };
    let is_empty = {
        let mut room = room_arc.lock();
        room.clients.remove(&peer_id);
        room.pending.remove(&peer_id);
        broadcast_and_prune(&mut *room, &ServerMessage::PeerLeft { peer_id });
        if room.host_peer_id == peer_id {
            if let Some(&new_host) = room.clients.keys().next() {
                room.host_peer_id = new_host;
                broadcast_and_prune(
                    &mut *room,
                    &ServerMessage::RoomEvent(RoomEvent::HostTransferred(new_host)),
                );
            }
        }
        room.clients.is_empty() && room.pending.is_empty()
    };
    if is_empty {
        crate::debug_println!("room destroyed app={} room={:?}", app_key, room_code);
        if let Some(pool) = registry.get(app_key) {
            pool.remove_if_empty(room_code);
        }
    }
}
