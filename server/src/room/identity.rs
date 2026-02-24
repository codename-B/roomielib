//! Client identity: machine hash, optional display name; ban set.

use serde::{Deserialize, Serialize};

/// Opaque client identity from machine-derived hash (sent by client in Hello).
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientHash(pub String);

impl ClientHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
