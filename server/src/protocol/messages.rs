//! Message types for client <-> server. Serialization via bincode (serde).
//!
//! Deserialization uses `bincode::Options::with_limit` to cap allocation size,
//! preventing a crafted payload from triggering multi-GB allocations.

use std::io;

use bincode::Options;
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::protocol::frame::MAX_FRAME_LEN;
use crate::room::{PeerId, RoomCode, RoomSettings};

// ---------- Client -> Server ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello {
        app_key: String,
        room_code: RoomCode,
        client_hash: String,
        display_name: Option<String>,
        /// Required when server sets ROOMIE_APP_SECRET.
        #[serde(default)]
        auth_token: Option<String>,
        /// Required when joining a room that has a password set.
        #[serde(default)]
        password: Option<String>,
    },
    Move(Vec<u8>),
    /// Input blob (e.g. paddle position); forwarded to host only as PeerInput.
    Input(Vec<u8>),
    RoomControl(RoomControl),
    RequestState,
    Ping,
    /// List public, non-full rooms for an app (no session required).
    ListRooms { app_key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomControl {
    CreateRoom { settings: RoomSettings },
    UpdateSettings(RoomSettings),
    SetPublic(bool),
    AcceptPeer(PeerId),
    DenyPeer(PeerId),
    Kick(PeerId),
    Ban(PeerId),
    TransferHost(PeerId),
    Leave,
}

// ---------- Server -> Client ----------

/// One entry in the public room list (no password; safe to send to browsers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomListEntry {
    pub room_code: RoomCode,
    pub player_count: u32,
    pub settings: RoomSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Joined {
        room_code: RoomCode,
        peer_id: PeerId,
        host_peer_id: PeerId,
        /// Assigned paddle/slot. Agnostic to host; first available when you join.
        side: u8,
        settings: RoomSettings,
        initial_state: Option<Bytes>,
    },
    /// Response to ListRooms: public, non-full rooms for the app.
    RoomList {
        rooms: Vec<RoomListEntry>,
    },
    PeerJoined {
        peer_id: PeerId,
        client_hash: String,
        display_name: Option<String>,
    },
    PeerLeft {
        peer_id: PeerId,
    },
    /// Input from a peer; sent to host only so host can incorporate into state.
    PeerInput {
        peer_id: PeerId,
        blob: Vec<u8>,
    },
    StateUpdate(Bytes),
    MoveAccepted,
    MoveRejected {
        reason: String,
    },
    RoomEvent(RoomEvent),
    Pong,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomEvent {
    HostTransferred(PeerId),
    Kicked,
    Banned,
    SettingsChanged(RoomSettings),
}

// ---------- Encode / Decode (bincode with allocation limit) ----------

/// Bincode options matching the legacy `bincode::serialize` wire format (varint, LE,
/// allow trailing bytes) but with an allocation cap equal to the frame size limit.
fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_varint_encoding()
        .allow_trailing_bytes()
        .with_limit(MAX_FRAME_LEN as u64)
}

/// Common trait for messages that can be encoded/decoded using bincode.
pub trait ProtocolMessage: Serialize + DeserializeOwned {
    fn encode(&self) -> io::Result<Vec<u8>> {
        bincode_opts()
            .serialize(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn decode(payload: &[u8]) -> io::Result<Self> {
        bincode_opts()
            .deserialize(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl ProtocolMessage for ClientMessage {}
impl ProtocolMessage for ServerMessage {}
