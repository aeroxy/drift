use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

fn default_destination() -> String {
    ".".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlMessage {
    // Handshake
    KeyExchange { public_key: String },
    AuthChallenge { nonce: String },
    AuthResponse { proof: String },
    HandshakeComplete,

    // Browsing
    BrowseRequest { path: String },
    BrowseResponse {
        hostname: String,
        cwd: String,
        entries: Vec<FileEntry>,
    },

    // Info
    InfoRequest,
    InfoResponse {
        hostname: String,
        root_dir: String,
    },

    // Transfers
    TransferRequest {
        id: Uuid,
        entries: Vec<TransferEntry>,
        direction: Direction,
        #[serde(default = "default_destination")]
        destination_path: String,
    },
    TransferAccepted {
        id: Uuid,
        resume_offsets: HashMap<String, u64>,
    },
    TransferProgress {
        id: Uuid,
        path: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    TransferComplete {
        id: Uuid,
        total_bytes: u64,
    },
    TransferFinalized {
        id: Uuid,
    },
    TransferError {
        id: Uuid,
        error: String,
    },

    // System
    ConnectionStatus {
        has_remote: bool,
    },
    Ping,
    Pong,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    #[cfg(unix)]
    pub permissions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferEntry {
    pub relative_path: String,
    pub size: u64,
    pub is_dir: bool,
    #[cfg(unix)]
    pub permissions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Direction {
    Push,
    Pull,
}

impl ControlMessage {
    /// Returns true if this message expects a response
    pub fn is_request(&self) -> bool {
        matches!(
            self,
            ControlMessage::BrowseRequest { .. }
                | ControlMessage::InfoRequest
                | ControlMessage::TransferRequest { .. }
                | ControlMessage::Ping
        )
    }
}
