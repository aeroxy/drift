use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use tokio::sync::mpsc;
use axum::extract::ws::Message;

use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};
use crate::protocol::codec::encode_data_frame;
use crate::server::AppState;
use crate::fileops::reader::ChunkedReader;
use crate::fileops::compress;

pub async fn handle_browser_transfer(
    state: Arc<AppState>,
    id: Uuid,
    entries: Vec<TransferEntry>,
    direction: Direction,
    ws_tx: mpsc::UnboundedSender<Message>,
) {
    tracing::info!("Browser transfer request: id={}, entries={}, direction={:?}", id, entries.len(), direction);

    // Check if we have a remote connection
    let remote = state.remote.read().await;
    if remote.is_none() {
        send_error(&ws_tx, id, "No remote connection");
        return;
    }

    // Forward request to remote
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    let request_msg = ControlMessage::TransferRequest {
        id,
        entries: entries.clone(),
        direction: direction.clone(),
    };

    if let Some(ref remote_conn) = *remote {
        if remote_conn.tx.send((request_msg, response_tx)).is_err() {
            send_error(&ws_tx, id, "Failed to send to remote");
            return;
        }
    }
    drop(remote); // Release the lock

    // Wait for TransferAccepted response
    match tokio::time::timeout(std::time::Duration::from_secs(10), response_rx).await {
        Ok(Ok(ControlMessage::TransferAccepted { .. })) => {
            tracing::info!("Remote accepted transfer, starting file send");

            // Forward TransferAccepted to browser
            let _ = ws_tx.send(Message::Text(
                serde_json::to_string(&ControlMessage::TransferAccepted {
                    id,
                    resume_offsets: std::collections::HashMap::new(),
                }).unwrap().into()
            ));

            // Now send files
            if direction == Direction::Push && !entries.is_empty() {
                push_entries(&state, id, &entries, &ws_tx).await;
            }
        }
        Ok(Ok(ControlMessage::TransferError { error, .. })) => {
            send_error(&ws_tx, id, &error);
        }
        Ok(Ok(_)) => {
            send_error(&ws_tx, id, "Unexpected response from remote");
        }
        Ok(Err(_)) => {
            send_error(&ws_tx, id, "Remote response channel closed");
        }
        Err(_) => {
            send_error(&ws_tx, id, "Remote response timeout");
        }
    }
}

/// Push all entries (files and folders) to remote
async fn push_entries(
    state: &AppState,
    id: Uuid,
    entries: &[TransferEntry],
    ws_tx: &mpsc::UnboundedSender<Message>,
) {
    // Collect files to transfer (compressing directories into temp archives)
    let mut files_to_send: Vec<(String, PathBuf, u64, Option<PathBuf>)> = Vec::new();
    // Each tuple: (display_name, file_path, size, cleanup_path)

    for entry in entries {
        if entry.is_dir {
            // Compress directory to temp archive
            match compress::compress_directory(&state.config.root_dir, &entry.relative_path) {
                Ok((archive_path, archive_size)) => {
                    files_to_send.push((
                        entry.relative_path.clone(),
                        archive_path.clone(),
                        archive_size,
                        Some(archive_path),
                    ));
                }
                Err(e) => {
                    send_error(ws_tx, id, &format!("Failed to compress {}: {}", entry.relative_path, e));
                    cleanup_archives(&files_to_send);
                    return;
                }
            }
        } else {
            let file_path = state.config.root_dir.join(&entry.relative_path);
            files_to_send.push((
                entry.relative_path.clone(),
                file_path,
                entry.size,
                None,
            ));
        }
    }

    // Calculate total size
    let total_size: u64 = files_to_send.iter().map(|(_, _, s, _)| s).sum();
    let mut total_sent: u64 = 0;

    // Get channels to remote
    let channels = {
        let remote = state.remote.read().await;
        remote.as_ref().map(|r| (r.binary_tx.clone(), r.outgoing_tx.clone()))
    };

    let Some((binary_tx, outgoing_tx)) = channels else {
        send_error(ws_tx, id, "Remote connection lost");
        cleanup_archives(&files_to_send);
        return;
    };

    // Send each file
    for (display_name, file_path, _file_size, _cleanup) in &files_to_send {
        match ChunkedReader::open(file_path, 0).await {
            Ok(mut reader) => {
                tracing::info!("Sending: {} ({} bytes)", display_name, reader.total_size());

                while let Ok(Some((_offset, chunk))) = reader.read_chunk().await {
                    // Encode binary frame
                    let frame = encode_data_frame(id, total_sent, &chunk);

                    // Send binary frame to remote
                    if binary_tx.send(frame).is_err() {
                        send_error(ws_tx, id, "Connection to remote lost");
                        cleanup_archives(&files_to_send);
                        return;
                    }

                    total_sent += chunk.len() as u64;

                    // Send progress to browser
                    let _ = ws_tx.send(Message::Text(
                        serde_json::to_string(&ControlMessage::TransferProgress {
                            id,
                            path: display_name.clone(),
                            bytes_done: total_sent,
                            bytes_total: total_size,
                        }).unwrap().into()
                    ));
                }
            }
            Err(e) => {
                send_error(ws_tx, id, &format!("Failed to open {}: {}", display_name, e));
                cleanup_archives(&files_to_send);
                return;
            }
        }
    }

    // Send TransferComplete to remote server (one-way notification)
    let _ = outgoing_tx.send(ControlMessage::TransferComplete { id });

    // Send complete to browser
    let _ = ws_tx.send(Message::Text(
        serde_json::to_string(&ControlMessage::TransferComplete { id }).unwrap().into()
    ));
    tracing::info!("Transfer complete: {} ({} bytes)", id, total_sent);

    // Clean up temp archives
    cleanup_archives(&files_to_send);
}

fn send_error(ws_tx: &mpsc::UnboundedSender<Message>, id: Uuid, error: &str) {
    let _ = ws_tx.send(Message::Text(
        serde_json::to_string(&ControlMessage::TransferError {
            id,
            error: error.to_string(),
        }).unwrap().into()
    ));
}

fn cleanup_archives(files: &[(String, PathBuf, u64, Option<PathBuf>)]) {
    for (_, _, _, cleanup) in files {
        if let Some(path) = cleanup {
            compress::cleanup_archive(path);
        }
    }
}
