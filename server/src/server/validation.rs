//! Input validation for client-supplied strings. Rejects oversized or invalid data.

const MAX_APP_KEY_LEN: usize = 256;
const MAX_ROOM_CODE_LEN: usize = 32;
const MAX_CLIENT_HASH_LEN: usize = 256;
const MAX_DISPLAY_NAME_LEN: usize = 64;
const MAX_AUTH_TOKEN_LEN: usize = 256;
const MAX_PASSWORD_LEN: usize = 128;

/// Validates Hello payload; returns error message if invalid.
pub fn validate_hello(
    app_key: &str,
    room_code_len: usize,
    client_hash: &str,
    display_name: Option<&str>,
    auth_token: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    if app_key.is_empty() {
        return Err("app_key is required".into());
    }
    if app_key.len() > MAX_APP_KEY_LEN {
        return Err("app_key too long".into());
    }
    if room_code_len > MAX_ROOM_CODE_LEN {
        return Err("room_code too long".into());
    }
    if client_hash.is_empty() {
        return Err("client_hash is required".into());
    }
    if client_hash.len() > MAX_CLIENT_HASH_LEN {
        return Err("client_hash too long".into());
    }
    if let Some(d) = display_name {
        if d.len() > MAX_DISPLAY_NAME_LEN {
            return Err("display_name too long".into());
        }
    }
    if let Some(t) = auth_token {
        if t.len() > MAX_AUTH_TOKEN_LEN {
            return Err("auth_token too long".into());
        }
    }
    if let Some(p) = password {
        if p.len() > MAX_PASSWORD_LEN {
            return Err("password too long".into());
        }
    }
    Ok(())
}
