use std::sync::Arc;
use uuid::Uuid;
use tokio::sync::mpsc;
use axum::extract::ws::Message;

use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};
use crate::server::AppState;
use crate::fileops::reader::ChunkedReader;

#[allow(dead_code)]
pub async fn handle_transfer_request(
    state: Arc<AppState>,
    id: Uuid,
    entries: Vec<TransferEntry>,
    direction: Direction,
    ws_tx: mpsc::UnboundedSender<Message>,
) {
    tracing::info!("Starting transfer: id={}, entries={}, direction={:?}", id, entries.len(), direction);

    // For now, only handle single file transfers (first entry)
    if entries.is_empty() {
        let _ = ws_tx.send(Message::Text(
            serde_json::to_string(&ControlMessage::TransferError {
                id,
                error: "No files to transfer".to_string(),
            }).unwrap().into()
        ));
        return;
    }

    if entries.len() > 1 {
        let _ = ws_tx.send(Message::Text(
            serde_json::to_string(&ControlMessage::TransferError {
                id,
                error: "Multiple file transfer not yet implemented".to_string(),
            }).unwrap().into()
        ));
        return;
    }

    let entry = &entries[0];
    if entry.is_dir {
        let _ = ws_tx.send(Message::Text(
            serde_json::to_string(&ControlMessage::TransferError {
                id,
                error: "Directory transfer not yet implemented".to_string(),
            }).unwrap().into()
        ));
        return;
    }

    match direction {
        Direction::Push => {
            // Send TransferAccepted first
            let _ = ws_tx.send(Message::Text(
                serde_json::to_string(&ControlMessage::TransferAccepted {
                    id,
                    resume_offsets: std::collections::HashMap::new(),
                }).unwrap().into()
            ));

            // Read local file and send to remote
            let file_path = state.config.root_dir.join(&entry.relative_path);
            match ChunkedReader::open(&file_path, 0).await {
                Ok(mut reader) => {
                    tracing::info!("Reading file: {:?} ({} bytes)", file_path, reader.total_size());

                    while let Ok(Some((offset, chunk))) = reader.read_chunk().await {
                        // Send progress
                        let _ = ws_tx.send(Message::Text(
                            serde_json::to_string(&ControlMessage::TransferProgress {
                                id,
                                path: entry.relative_path.clone(),
                                bytes_done: offset + chunk.len() as u64,
                                bytes_total: reader.total_size(),
                            }).unwrap().into()
                        ));

                        // In a real implementation, we would send binary frames here
                        // For now, just simulate progress
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }

                    // Send complete
                    let _ = ws_tx.send(Message::Text(
                        serde_json::to_string(&ControlMessage::TransferComplete { id, total_bytes: 0 }).unwrap().into()
                    ));
                    tracing::info!("Transfer complete: {}", id);
                }
                Err(e) => {
                    let _ = ws_tx.send(Message::Text(
                        serde_json::to_string(&ControlMessage::TransferError {
                            id,
                            error: format!("Failed to open file: {}", e),
                        }).unwrap().into()
                    ));
                }
            }
        }
        Direction::Pull => {
            let _ = ws_tx.send(Message::Text(
                serde_json::to_string(&ControlMessage::TransferError {
                    id,
                    error: "Pull transfer not yet implemented".to_string(),
                }).unwrap().into()
            ));
        }
    }
}
