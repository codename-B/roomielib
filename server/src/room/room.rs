//! Room: host, clients, pending (accept-required), state blob, kick/ban.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::mpsc;
use unicase::UniCase;

use crate::protocol::messages::ServerMessage;
use crate::room::identity::ClientHash;

use super::pool::PeerId;

/// Unique room code within an app (e.g. 6–8 alphanumeric).
/// Uses `UniCase<String>` so comparison and hashing are case-insensitive.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct RoomCode(pub UniCase<String>);

impl RoomCode {
    pub fn new(s: String) -> Self {
        Self(UniCase::new(s))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl std::fmt::Display for RoomCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.as_ref().fmt(f)
    }
}

impl Serialize for RoomCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RoomCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let normalized: String = s
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        Ok(RoomCode(UniCase::new(normalized)))
    }
}

/// Room settings (max players, min players to start, visibility, password, accept-required).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoomSettings {
    pub max_players: u32,
    /// Minimum accepted players required before the host's moves are accepted (0 = no minimum).
    #[serde(default)]
    pub min_players: u32,
    pub is_public: bool,
    pub password: Option<String>,
    pub accept_required: bool,
    /// Game-specific options (e.g. map name, mode). Opaque to server.
    #[serde(default)]
    pub custom: Option<serde_json::Value>,
}

// ---------- Room and ClientInfo ----------

/// Per-client info in a room (accepted or pending).
#[derive(Debug)]
pub struct ClientInfo {
    pub client_hash: ClientHash,
    pub display_name: Option<String>,
    pub tx: mpsc::Sender<ServerMessage>,
    /// Assigned paddle/slot. Agnostic to host/guest; first available side when joining.
    pub side: u8,
}

/// A single game room: host, accepted/pending clients, state, settings.
pub struct Room {
    pub app_key: String,
    pub code: RoomCode,
    pub host_peer_id: PeerId,
    /// Accepted clients (and host).
    pub clients: HashMap<PeerId, ClientInfo>,
    /// Waiting for host to accept (accept_required).
    pub pending: HashMap<PeerId, ClientInfo>,
    /// Authoritative game state blob. Uses `Bytes` for O(1) refcount cloning during broadcast.
    pub state: Bytes,
    pub settings: RoomSettings,
    pub banned: HashSet<ClientHash>,
    next_peer_id: PeerId,
}

/// Returns true if `try_send` failed because the receiver was dropped (channel closed).
/// A `Full` error just means the message was dropped for a slow client — not dead.
fn is_closed<T>(r: Result<(), mpsc::error::TrySendError<T>>) -> bool {
    matches!(r, Err(mpsc::error::TrySendError::Closed(_)))
}

impl Room {
    pub fn new(app_key: String, code: RoomCode, settings: RoomSettings) -> Self {
        Self {
            app_key,
            code: code.clone(),
            host_peer_id: 0,
            clients: HashMap::new(),
            pending: HashMap::new(),
            state: Bytes::new(),
            settings,
            banned: HashSet::new(),
            next_peer_id: 1,
        }
    }

    /// Allocate next peer id in this room. Returns None if saturated (u32::MAX) with no free id.
    pub fn next_peer_id(&mut self) -> Option<PeerId> {
        let id = self.next_peer_id;
        self.next_peer_id = self.next_peer_id.saturating_add(1);
        let in_use = |pid: PeerId| self.clients.contains_key(&pid) || self.pending.contains_key(&pid);
        if in_use(id) {
            let start = self.next_peer_id.max(1);
            for candidate in (start..=PeerId::MAX).chain(1..start) {
                if !in_use(candidate) {
                    self.next_peer_id = candidate.saturating_add(1);
                    return Some(candidate);
                }
            }
            return None;
        }
        Some(id)
    }

    /// First available paddle/slot in 0..max_players.
    pub fn next_available_side(&self) -> u8 {
        let used: std::collections::HashSet<u8> = self
            .clients
            .values()
            .map(|c| c.side)
            .chain(self.pending.values().map(|c| c.side))
            .collect();
        let max_side = if self.settings.max_players == 0 {
            255u8
        } else {
            self.settings.max_players.min(255) as u8
        };
        for side in 0..=max_side {
            if !used.contains(&side) {
                return side;
            }
        }
        0
    }

    pub fn is_full(&self) -> bool {
        let max = self.settings.max_players;
        if max == 0 {
            return false;
        }
        (self.clients.len() + self.pending.len()) as u32 >= max
    }

    pub fn is_banned(&self, hash: &ClientHash) -> bool {
        self.banned.contains(hash)
    }

    /// Broadcast to accepted clients; remove those whose channel is closed; fix host if removed.
    /// Returns (dead peer ids, new host if current host was removed).
    pub fn broadcast_retain_connected(&mut self, msg: &ServerMessage) -> (Vec<PeerId>, Option<PeerId>) {
        let mut dead = Vec::new();
        let mut new_host = None;
        for (pid, info) in self.clients.iter() {
            if is_closed(info.tx.try_send(msg.clone())) {
                dead.push(*pid);
            }
        }
        for pid in &dead {
            self.clients.remove(pid);
            if self.host_peer_id == *pid {
                new_host = self.clients.keys().next().copied();
                self.host_peer_id = new_host.unwrap_or(0);
            }
        }
        (dead, new_host)
    }

    /// Broadcast StateUpdate to all clients and pending; remove dead, fix host.
    /// Uses `self.state` directly — caller must update `self.state` before calling.
    pub fn broadcast_state_update_retain_connected(&mut self) -> (Vec<PeerId>, Option<PeerId>) {
        let msg = ServerMessage::StateUpdate(self.state.clone());
        let mut dead = Vec::new();
        let mut new_host = None;
        for (pid, info) in self.clients.iter() {
            if is_closed(info.tx.try_send(msg.clone())) {
                dead.push(*pid);
            }
        }
        for (pid, info) in self.pending.iter() {
            if is_closed(info.tx.try_send(msg.clone())) {
                dead.push(*pid);
            }
        }
        for pid in &dead {
            self.clients.remove(pid);
            self.pending.remove(pid);
            if self.host_peer_id == *pid {
                new_host = self.clients.keys().next().copied();
                self.host_peer_id = new_host.unwrap_or(0);
            }
        }
        (dead, new_host)
    }

    /// Send to one peer (accepted or pending). Removes peer if channel is dead.
    /// Returns false if the peer was removed (dead channel).
    pub fn send_to_retain_connected(&mut self, peer_id: PeerId, msg: &ServerMessage) -> bool {
        let dead = if let Some(info) = self.clients.get(&peer_id) {
            is_closed(info.tx.try_send(msg.clone()))
        } else if let Some(info) = self.pending.get(&peer_id) {
            is_closed(info.tx.try_send(msg.clone()))
        } else {
            return true;
        };
        if dead {
            self.clients.remove(&peer_id);
            self.pending.remove(&peer_id);
            if self.host_peer_id == peer_id {
                self.host_peer_id = self.clients.keys().next().copied().unwrap_or(0);
            }
            return false;
        }
        true
    }

}
