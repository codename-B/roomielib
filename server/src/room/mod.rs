//! Room pool per app, room state, host, clients, settings, kick/ban.

pub mod identity;
pub mod pool;
pub mod room;

// Re-export types used by protocol and server.
pub use identity::ClientHash;
pub use room::{RoomCode, RoomSettings};
pub use pool::PeerId;
