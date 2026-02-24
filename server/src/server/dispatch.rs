//! Message dispatch: route client messages to room handlers.
//!
//! Fully synchronous — all locking is via std::sync::Mutex and DashMap.
//! No lock is held across an await point, so Tokio worker threads are never blocked
//! longer than the critical section (microseconds).

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::protocol::messages::{ClientMessage, RoomControl, RoomEvent, ServerMessage};
use crate::room::identity::ClientHash;
use crate::room::pool::RoomPool;
use crate::room::room::ClientInfo;
use crate::schema::load;

use super::validation;
use super::{
    broadcast_and_prune, broadcast_state_update_and_prune, leave_room, lookup_room, AppRegistry,
    Session,
};

/// Dispatch client message; returns list of ServerMessages to send to this client only.
/// Broadcasts to other peers are done inside by sending on their channels.
pub(crate) fn dispatch(
    registry: &AppRegistry,
    session: &mut Option<Session>,
    my_tx: &mpsc::Sender<ServerMessage>,
    msg: ClientMessage,
) -> std::io::Result<Vec<ServerMessage>> {
    let mut to_send = Vec::new();
    match msg {
        ClientMessage::Hello {
            app_key,
            room_code,
            client_hash,
            display_name,
            auth_token,
            password,
        } => {
            if let Some(old) = session.take() {
                leave_room(registry, &old.app_key, &old.room_code, old.peer_id);
            }
            if let Err(e) = validation::validate_hello(
                &app_key,
                room_code.as_str().len(),
                &client_hash,
                display_name.as_deref(),
                auth_token.as_deref(),
                password.as_deref(),
            ) {
                to_send.push(ServerMessage::Error {
                    message: e.into(),
                });
                return Ok(to_send);
            }
            if let Ok(secret) = std::env::var("ROOMIE_APP_SECRET") {
                if !secret.is_empty() {
                    let token = auth_token.as_deref().unwrap_or("");
                    if token != secret {
                        to_send.push(ServerMessage::Error {
                            message: "authentication required".into(),
                        });
                        return Ok(to_send);
                    }
                }
            }
            if !load::is_registered_app(&app_key) {
                to_send.push(ServerMessage::Error {
                    message: "unregistered game type".into(),
                });
                return Ok(to_send);
            }
            let hash = ClientHash(client_hash.clone());
            let is_new_room = room_code.as_str().is_empty();

            // Ensure pool exists (momentary DashMap shard write-lock, released at semicolon).
            registry
                .entry(app_key.clone())
                .or_insert_with(|| RoomPool::new(app_key.clone()));

            let pool_ref = match registry.get(&app_key) {
                Some(p) => p,
                None => {
                    to_send.push(ServerMessage::Error {
                        message: "internal error".into(),
                    });
                    return Ok(to_send);
                }
            };

            // Get or create the room Arc (DashMap shard lock only).
            let (code, room_arc) = if is_new_room {
                let default_settings = load::default_room_settings_for_app(&app_key);
                let _min_p = default_settings.min_players;
                let _max_p = default_settings.max_players;
                match pool_ref.create_room(default_settings, "") {
                    Some(result) => {
                        crate::debug_println!(
                            "room created app={} room={:?} min_players={} max_players={}",
                            app_key,
                            result.0,
                            _min_p,
                            _max_p
                        );
                        result
                    }
                    None => {
                        to_send.push(ServerMessage::Error {
                            message: "failed to create room".into(),
                        });
                        return Ok(to_send);
                    }
                }
            } else {
                match pool_ref.get_room(&room_code) {
                    Some(arc) => (room_code.clone(), arc),
                    None => {
                        crate::debug_println!(
                            "join failed: room_code={:?} app_key={}",
                            room_code.as_str(),
                            app_key
                        );
                        to_send.push(ServerMessage::Error {
                            message: "room not found".into(),
                        });
                        return Ok(to_send);
                    }
                }
            };
            drop(pool_ref);

            // Lock this specific room — no global lock held.
            let mut room = room_arc.lock();

            if room.is_banned(&hash) {
                to_send.push(ServerMessage::Error {
                    message: "banned from this room".into(),
                });
                return Ok(to_send);
            }
            if room.is_full() {
                to_send.push(ServerMessage::Error {
                    message: "room full".into(),
                });
                return Ok(to_send);
            }
            if let Some(required) = &room.settings.password {
                let provided = password.as_deref().unwrap_or("");
                if provided != required.as_str() {
                    to_send.push(ServerMessage::Error {
                        message: "invalid or missing room password".into(),
                    });
                    return Ok(to_send);
                }
            }
            let peer_id = match room.next_peer_id() {
                Some(id) => id,
                None => {
                    to_send.push(ServerMessage::Error {
                        message: "room full".into(),
                    });
                    return Ok(to_send);
                }
            };
            let side = room.next_available_side();
            let info = ClientInfo {
                client_hash: hash.clone(),
                display_name: display_name.clone(),
                tx: my_tx.clone(),
                side,
            };
            let host_id = if room.clients.is_empty() && room.pending.is_empty() {
                room.host_peer_id = peer_id;
                room.clients.insert(peer_id, info);
                peer_id
            } else if room.settings.accept_required {
                room.pending.insert(peer_id, info);
                room.host_peer_id
            } else {
                room.clients.insert(peer_id, info);
                room.host_peer_id
            };
            let initial_state = if room.state.is_empty() {
                None
            } else {
                Some(room.state.clone())
            };
            crate::debug_println!(
                "client joined app={} room={:?} peer_id={} host_peer_id={} side={}",
                app_key,
                code,
                peer_id,
                host_id,
                side
            );
            let mut settings = room.settings.clone();
            settings.password = None;
            let joined = ServerMessage::Joined {
                room_code: code.clone(),
                peer_id,
                host_peer_id: host_id,
                side,
                settings,
                initial_state,
            };
            if !room.settings.accept_required || room.clients.contains_key(&peer_id) {
                for (pid, client_info) in &room.clients {
                    if *pid != peer_id {
                        let _ = client_info.tx.try_send(ServerMessage::PeerJoined {
                            peer_id,
                            client_hash: client_hash.clone(),
                            display_name: display_name.clone(),
                        });
                    }
                }
            }
            drop(room);
            *session = Some(Session {
                app_key,
                room_code: code,
                peer_id,
            });
            to_send.push(joined);
        }
        ClientMessage::Move(blob) => {
            let s = match session.as_ref() {
                Some(s) => s,
                None => {
                    to_send.push(ServerMessage::Error {
                        message: "not in a room".into(),
                    });
                    return Ok(to_send);
                }
            };
            let room_arc = match lookup_room(registry, &s.app_key, &s.room_code) {
                Some(arc) => arc,
                None => return Ok(to_send),
            };
            let mut room = room_arc.lock();
            if room.host_peer_id != s.peer_id {
                to_send.push(ServerMessage::MoveRejected {
                    reason: "only host can send moves".into(),
                });
                return Ok(to_send);
            }
            let min_players = room.settings.min_players;
            let accepted_count = room.clients.len() as u32;
            if min_players > 0 && accepted_count < min_players {
                crate::debug_println!(
                    "move rejected: {} players, need {}",
                    accepted_count,
                    min_players
                );
                to_send.push(ServerMessage::MoveRejected {
                    reason: format!("minimum {} players required to start", min_players),
                });
                return Ok(to_send);
            }
            room.state = Bytes::from(blob);
            broadcast_state_update_and_prune(&mut *room);
            to_send.push(ServerMessage::MoveAccepted);
        }
        ClientMessage::Input(blob) => {
            let s = match session.as_ref() {
                Some(s) => s,
                None => {
                    to_send.push(ServerMessage::Error {
                        message: "not in a room".into(),
                    });
                    return Ok(to_send);
                }
            };
            let room_arc = match lookup_room(registry, &s.app_key, &s.room_code) {
                Some(arc) => arc,
                None => return Ok(to_send),
            };
            let room = room_arc.lock();
            if room.host_peer_id != s.peer_id {
                if let Some(host_info) = room.clients.get(&room.host_peer_id) {
                    let _ = host_info.tx.try_send(ServerMessage::PeerInput {
                        peer_id: s.peer_id,
                        blob,
                    });
                }
            }
        }
        ClientMessage::RoomControl(rc) => {
            let s = match session.as_ref() {
                Some(s) => s,
                None => {
                    to_send.push(ServerMessage::Error {
                        message: "not in a room".into(),
                    });
                    return Ok(to_send);
                }
            };

            // Handle Leave and CreateRoom before acquiring the room lock.
            match &rc {
                RoomControl::Leave => {
                    let app_key = s.app_key.clone();
                    let room_code = s.room_code.clone();
                    let peer_id = s.peer_id;
                    *session = None;
                    leave_room(registry, &app_key, &room_code, peer_id);
                    return Ok(to_send);
                }
                RoomControl::CreateRoom { .. } => {
                    to_send.push(ServerMessage::Error {
                        message: "already in a room".into(),
                    });
                    return Ok(to_send);
                }
                _ => {}
            }

            let room_arc = match lookup_room(registry, &s.app_key, &s.room_code) {
                Some(arc) => arc,
                None => return Ok(to_send),
            };
            let mut room = room_arc.lock();
            let is_host = room.host_peer_id == s.peer_id;
            match rc {
                RoomControl::Leave | RoomControl::CreateRoom { .. } => unreachable!(),
                RoomControl::UpdateSettings(new_settings) if is_host => {
                    room.settings = new_settings.clone();
                    let mut safe_settings = new_settings;
                    safe_settings.password = None;
                    broadcast_and_prune(
                        &mut *room,
                        &ServerMessage::RoomEvent(RoomEvent::SettingsChanged(safe_settings)),
                    );
                }
                RoomControl::SetPublic(b) if is_host => {
                    room.settings.is_public = b;
                }
                RoomControl::AcceptPeer(pid) if is_host => {
                    if let Some(info) = room.pending.remove(&pid) {
                        let client_hash = info.client_hash.0.clone();
                        let display_name = info.display_name.clone();
                        room.clients.insert(pid, info);
                        broadcast_and_prune(
                            &mut *room,
                            &ServerMessage::PeerJoined {
                                peer_id: pid,
                                client_hash,
                                display_name,
                            },
                        );
                    }
                }
                RoomControl::DenyPeer(pid) if is_host => {
                    if let Some(info) = room.pending.remove(&pid) {
                        let _ = info.tx.try_send(ServerMessage::RoomEvent(RoomEvent::Kicked));
                    }
                }
                RoomControl::Kick(pid) if is_host => {
                    if let Some(info) = room.clients.remove(&pid) {
                        let _ = info.tx.try_send(ServerMessage::RoomEvent(RoomEvent::Kicked));
                    }
                    broadcast_and_prune(&mut *room, &ServerMessage::PeerLeft { peer_id: pid });
                }
                RoomControl::Ban(pid) if is_host => {
                    if let Some(info) = room.clients.remove(&pid) {
                        room.banned.insert(info.client_hash.clone());
                        let _ = info.tx.try_send(ServerMessage::RoomEvent(RoomEvent::Banned));
                    }
                    broadcast_and_prune(&mut *room, &ServerMessage::PeerLeft { peer_id: pid });
                }
                RoomControl::TransferHost(pid) if is_host => {
                    if room.clients.contains_key(&pid) {
                        room.host_peer_id = pid;
                        broadcast_and_prune(
                            &mut *room,
                            &ServerMessage::RoomEvent(RoomEvent::HostTransferred(pid)),
                        );
                    }
                }
                _ => {
                    to_send.push(ServerMessage::Error {
                        message: "forbidden or invalid".into(),
                    });
                    return Ok(to_send);
                }
            }
        }
        ClientMessage::RequestState => {
            let s = match session.as_ref() {
                Some(s) => s,
                None => return Ok(to_send),
            };
            let room_arc = match lookup_room(registry, &s.app_key, &s.room_code) {
                Some(arc) => arc,
                None => return Ok(to_send),
            };
            let room = room_arc.lock();
            if !room.state.is_empty() {
                to_send.push(ServerMessage::StateUpdate(room.state.clone()));
            }
        }
        ClientMessage::Ping => {
            to_send.push(ServerMessage::Pong);
        }
        ClientMessage::ListRooms { app_key } => {
            if !load::is_registered_app(&app_key) {
                to_send.push(ServerMessage::Error {
                    message: "unregistered game type".into(),
                });
                return Ok(to_send);
            }
            let pool_ref = match registry.get(&app_key) {
                Some(p) => p,
                None => {
                    to_send.push(ServerMessage::RoomList { rooms: vec![] });
                    return Ok(to_send);
                }
            };
            let rooms = pool_ref.list_public_rooms();
            to_send.push(ServerMessage::RoomList { rooms });
        }
    }
    Ok(to_send)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::room::RoomSettings;
    use std::sync::Arc;
    use dashmap::DashMap;

    #[tokio::test]
    async fn test_password_leak_in_joined() {
        let registry: AppRegistry = Arc::new(DashMap::new());
        let app_key = "pong".to_string();

        // Setup room pool
        let pool = crate::room::pool::RoomPool::new(app_key.clone());
        // Insert pool into registry
        registry.insert(app_key.clone(), pool);

        // Create a room with password
        let mut settings = RoomSettings::default();
        settings.password = Some("supersecret".to_string());
        settings.max_players = 4;

        let pool_ref = registry.get(&app_key).unwrap();
        // Since create_room uses Random, we'll let it generate code.
        let (code, _room) = pool_ref.create_room(settings, "ROOM1").unwrap();
        drop(pool_ref); // release lock

        // Prepare client message
        let (tx, _rx) = mpsc::channel(10);
        let mut session = None;

        let msg = ClientMessage::Hello {
            app_key: app_key.clone(),
            room_code: code.clone(),
            client_hash: "hash1".to_string(),
            display_name: Some("user1".to_string()),
            auth_token: None,
            password: Some("supersecret".to_string()),
        };

        let result = dispatch(&registry, &mut session, &tx, msg).unwrap();

        // Check result
        match &result[0] {
            ServerMessage::Joined { settings, .. } => {
                assert!(settings.password.is_none(), "Password leaked in Joined message! Got: {:?}", settings.password);
            },
            _ => panic!("Expected Joined message"),
        }
    }
}
