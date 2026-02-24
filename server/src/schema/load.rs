//! Load per-app schema to get default room settings (min_players, max_players).
//!
//! Schema path: env ROOMIE_SCHEMA_DIR, or "schema" relative to current directory.

use std::path::PathBuf;
use std::sync::OnceLock;

use dashmap::DashMap;

use crate::room::room::RoomSettings;

static CACHE: OnceLock<DashMap<String, RoomSettings>> = OnceLock::new();

fn get_cache() -> &'static DashMap<String, RoomSettings> {
    CACHE.get_or_init(DashMap::new)
}

/// Minimal schema fragment we need for room defaults.
#[derive(serde::Deserialize, Default)]
struct SchemaRoomDefaults {
    #[serde(default)]
    min_players: u32,
    #[serde(default)]
    max_players: u32,
}

fn schema_dir() -> Option<PathBuf> {
    let is_schema_dir = |p: &std::path::Path| p.is_dir();
    if let Some(dir) = std::env::var_os("ROOMIE_SCHEMA_DIR") {
        let p = PathBuf::from(dir);
        if is_schema_dir(&p) {
            return Some(p);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("schema");
        if is_schema_dir(&p) {
            return Some(p);
        }
    }
    // When run from target/debug/ or target/release/, repo root is ../..
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for rel in ["schema", "../schema", "../../schema"] {
                let p = parent.join(rel);
                if is_schema_dir(&p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Returns default room settings for the given app by loading `schema/{app_key}.json`
/// and reading `min_players` and `max_players`. If the file is missing or invalid,
/// returns `RoomSettings::default()` (or app-specific fallbacks for known apps).
pub fn default_room_settings_for_app(app_key: &str) -> RoomSettings {
    if let Some(settings) = get_cache().get(app_key) {
        return settings.clone();
    }
    if is_builtin_app(app_key) {
        let settings = app_fallback(app_key);
        get_cache().insert(app_key.to_string(), settings.clone());
        return settings;
    }
    if let Some(settings) = load_settings_from_disk(app_key) {
        get_cache().insert(app_key.to_string(), settings.clone());
        return settings;
    }
    // Not a registered app; return fallback without caching to avoid memory leak from invalid keys.
    app_fallback(app_key)
}

fn load_settings_from_disk(app_key: &str) -> Option<RoomSettings> {
    let dir = schema_dir()?;
    let path = dir.join(format!("{}.json", app_key));
    let contents = std::fs::read_to_string(&path).ok()?;
    let defaults: SchemaRoomDefaults = serde_json::from_str(&contents).ok()?;
    Some(RoomSettings {
        min_players: defaults.min_players,
        max_players: if defaults.max_players > 0 {
            defaults.max_players
        } else {
            RoomSettings::default().max_players
        },
        ..RoomSettings::default()
    })
}

/// Returns true if the app_key is a registered game type (has a schema file or is built-in).
/// Only registered apps can create or join rooms.
pub fn is_registered_app(app_key: &str) -> bool {
    if get_cache().contains_key(app_key) {
        return true;
    }
    if is_builtin_app(app_key) {
        // Cache built-in app settings too for speed.
        let settings = app_fallback(app_key);
        get_cache().insert(app_key.to_string(), settings);
        return true;
    }
    if let Some(settings) = load_settings_from_disk(app_key) {
        get_cache().insert(app_key.to_string(), settings);
        return true;
    }
    false
}

fn is_builtin_app(app_key: &str) -> bool {
    matches!(app_key, "pong")
}

/// Fallback when schema file is not found; known apps get correct min/max so start is enforced.
fn app_fallback(app_key: &str) -> RoomSettings {
    let (min_players, max_players) = if is_builtin_app(app_key) {
        (2, 2) // pong
    } else {
        (0, 0)
    };
    RoomSettings {
        min_players,
        max_players: if max_players > 0 { max_players } else { 0 },
        ..RoomSettings::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn bench_load_settings() {
        let app_key = "pong";
        // Ensure the file exists for the test. "pong" is builtin and has fallback.
        let iterations = 1000;
        let start = Instant::now();
        for _ in 0..iterations {
            let settings = default_room_settings_for_app(app_key);
            assert_eq!(settings.min_players, 2);
        }
        let duration = start.elapsed();
        println!("\nBENCHMARK: Took {:?} for {} iterations ({:?} per call)",
            duration, iterations, duration / iterations);
    }
}
