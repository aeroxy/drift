use std::collections::HashMap;
use uuid::Uuid;

use super::messages::TransferEntry;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TransferState {
    pub id: Uuid,
    pub entries: Vec<TransferEntry>,
    pub current_file: usize,
    pub current_offset: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub status: TransferStatus,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TransferStatus {
    Pending,
    InProgress,
    Paused,
    Complete,
    Failed(String),
}

#[allow(dead_code)]
impl TransferState {
    pub fn new(id: Uuid, entries: Vec<TransferEntry>) -> Self {
        let bytes_total = entries.iter().map(|e| e.size).sum();
        Self {
            id,
            entries,
            current_file: 0,
            current_offset: 0,
            bytes_total,
            bytes_done: 0,
            status: TransferStatus::Pending,
        }
    }

    pub fn apply_resume_offsets(&mut self, offsets: &HashMap<String, u64>) {
        for (path, &offset) in offsets {
            if let Some(entry) = self.entries.iter().find(|e| &e.relative_path == path) {
                self.bytes_done += offset.min(entry.size);
            }
        }
    }
}
