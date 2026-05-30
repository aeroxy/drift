use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Current protocol version. Increment when making backward-incompatible changes.
/// Version 1: Original format (no file_index in frames)
/// Version 2: Added file_index to data frames for multi-file transfers
/// Version 3: Correlated request/response ids for browse/info/ping
pub const CURRENT_PROTOCOL_VERSION: u32 = 3;
pub const MIN_MULTI_FILE_VERSION: u32 = 2;
pub const MIN_REQUEST_ID_VERSION: u32 = 3;

fn default_destination() -> String {
    ".".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlMessage {
    // Handshake
    KeyExchange {
        public_key: String,
        #[serde(default)]
        protocol_version: Option<u32>,
    },
    AuthChallenge {
        nonce: String,
    },
    AuthResponse {
        proof: String,
    },
    HandshakeComplete,

    // Browsing
    BrowseRequest {
        #[serde(default)]
        request_id: Option<Uuid>,
        path: String,
    },
    BrowseResponse {
        #[serde(default)]
        request_id: Option<Uuid>,
        hostname: String,
        cwd: String,
        entries: Vec<FileEntry>,
    },

    // Info
    InfoRequest {
        #[serde(default)]
        request_id: Option<Uuid>,
    },
    InfoResponse {
        #[serde(default)]
        request_id: Option<Uuid>,
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
    Ping {
        #[serde(default)]
        request_id: Option<Uuid>,
    },
    Pong {
        #[serde(default)]
        request_id: Option<Uuid>,
    },
    Error {
        #[serde(default)]
        request_id: Option<Uuid>,
        message: String,
    },
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
                | ControlMessage::InfoRequest { .. }
                | ControlMessage::TransferRequest { .. }
                | ControlMessage::Ping { .. }
        )
    }

    /// Correlation id used to track an outgoing request.
    pub fn request_id(&self) -> Option<Uuid> {
        match self {
            ControlMessage::BrowseRequest { request_id, .. }
            | ControlMessage::InfoRequest { request_id }
            | ControlMessage::Ping { request_id } => *request_id,
            ControlMessage::TransferRequest { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Correlation id used to route a response to its waiting caller.
    pub fn response_id(&self) -> Option<Uuid> {
        match self {
            ControlMessage::BrowseResponse { request_id, .. }
            | ControlMessage::InfoResponse { request_id, .. }
            | ControlMessage::Pong { request_id }
            | ControlMessage::Error { request_id, .. } => *request_id,
            ControlMessage::TransferAccepted { id, .. }
            | ControlMessage::TransferError { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Inject a correlation id into request variants that do not already have one.
    pub fn with_request_id(self, request_id: Uuid) -> Self {
        match self {
            ControlMessage::BrowseRequest {
                request_id: existing,
                path,
            } => ControlMessage::BrowseRequest {
                request_id: existing.or(Some(request_id)),
                path,
            },
            ControlMessage::InfoRequest {
                request_id: existing,
            } => ControlMessage::InfoRequest {
                request_id: existing.or(Some(request_id)),
            },
            ControlMessage::Ping {
                request_id: existing,
            } => ControlMessage::Ping {
                request_id: existing.or(Some(request_id)),
            },
            other => other,
        }
    }
}
