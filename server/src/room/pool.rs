//! RoomPool: maps room_code -> Room for one app_key. Generates unique room codes.
//!
//! Each room is wrapped in `Arc<Mutex<Room>>` so operations on different rooms
//! never contend with each other (per-room locking instead of global lock).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use rand::Rng;

use crate::protocol::messages::RoomListEntry;
use crate::room::room::{Room, RoomCode, RoomSettings};

/// Server-assigned peer id for a connected client in a room.
pub type PeerId = u32;

const ROOM_CODE_LEN: usize = 6;
const ROOM_CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no I,O,0,1
const ROOM_CODE_GEN_RETRIES: u32 = 20;

/// One app's set of rooms (room_code -> Room).
pub struct RoomPool {
    pub app_key: String,
    pub rooms: DashMap<RoomCode, Arc<Mutex<Room>>>,
}

impl RoomPool {
    pub fn new(app_key: String) -> Self {
        Self {
            app_key,
            rooms: DashMap::new(),
        }
    }

    /// Look up a room and return a cloned `Arc` (cheap refcount bump).
    /// The DashMap shard lock is released before the caller locks the room.
    pub fn get_room(&self, code: &RoomCode) -> Option<Arc<Mutex<Room>>> {
        self.rooms.get(code).map(|r| r.value().clone())
    }

    /// Atomically insert a new room only if the code is not already taken.
    fn try_insert_new_room(
        &self,
        code: RoomCode,
        settings: RoomSettings,
    ) -> Option<(RoomCode, Arc<Mutex<Room>>)> {
        use dashmap::mapref::entry::Entry;
        match self.rooms.entry(code.clone()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                let room = Arc::new(Mutex::new(Room::new(
                    self.app_key.clone(),
                    code.clone(),
                    settings,
                )));
                entry.insert(room.clone());
                Some((code, room))
            }
        }
    }

    /// Create a new room with an atomic check-and-insert (no TOCTOU race).
    /// Returns `None` only if a `requested_code` collides with an existing room.
    pub fn create_room(
        &self,
        settings: RoomSettings,
        requested_code: &str,
    ) -> Option<(RoomCode, Arc<Mutex<Room>>)> {
        if !requested_code.is_empty() {
            let code = RoomCode::new(requested_code.to_string());
            return self.try_insert_new_room(code, settings);
        }
        let mut rng = rand::thread_rng();
        for _ in 0..ROOM_CODE_GEN_RETRIES {
            let code_str: String = (0..ROOM_CODE_LEN)
                .map(|_| {
                    let i = rng.gen_range(0..ROOM_CODE_CHARS.len());
                    ROOM_CODE_CHARS[i] as char
                })
                .collect();
            if let Some(result) = self.try_insert_new_room(RoomCode::new(code_str), settings.clone()) {
                return Some(result);
            }
        }
        static FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.try_insert_new_room(RoomCode::new(format!("R{}", n)), settings)
    }

    /// Atomically remove a room only if it is empty. Uses `try_lock` inside the DashMap
    /// shard lock to avoid TOCTOU races — if another task is mid-join, the lock fails
    /// and the room is left in place.
    pub fn remove_if_empty(&self, code: &RoomCode) -> bool {
        self.rooms
            .remove_if(code, |_, room_arc| {
                match room_arc.try_lock() {
                    Some(room) => room.clients.is_empty() && room.pending.is_empty(),
                    None => false,
                }
            })
            .is_some()
    }

    /// Total number of rooms and total players (accepted + pending) across all rooms.
    pub fn room_and_player_counts(&self) -> (usize, usize) {
        let mut players = 0usize;
        for ref_multi in self.rooms.iter() {
            let room = ref_multi.value().lock();
            players += room.clients.len() + room.pending.len();
        }
        (self.rooms.len(), players)
    }

    /// List public, non-full rooms. Returns room code, player count, and settings (password cleared).
    pub fn list_public_rooms(&self) -> Vec<RoomListEntry> {
        let mut out = Vec::new();
        for ref_multi in self.rooms.iter() {
            let room = ref_multi.value().lock();
            if !room.settings.is_public || room.is_full() {
                continue;
            }
            let player_count = (room.clients.len() + room.pending.len()) as u32;
            let mut settings = room.settings.clone();
            settings.password = None; // never expose password in list
            out.push(RoomListEntry {
                room_code: room.code.clone(),
                player_count,
                settings,
            });
        }
        out
    }
}
