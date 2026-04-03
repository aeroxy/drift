use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use tokio::sync::mpsc;
use axum::extract::ws::Message;

use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};
use crate::protocol::codec::{encode_data_frame, encode_control_frame};
use crate::server::{AppState, FrameChannel};
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

    let remote = state.remote.read().await;
    if remote.is_none() {
        send_error(&ws_tx, id, "No remote connection");
        return;
    }

    // For Pull: register the local receiver BEFORE forwarding the request.
    // Binary frames from the remote can arrive before TransferAccepted is processed,
    // so the receiver must be ready or chunks would be silently dropped (and lost).
    let pull_done_rx = if direction == Direction::Pull {
        Some(state.transfer_receiver.start_transfer_with_notify(id, entries.clone()).await)
    } else {
        None
    };

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
    drop(remote);

    // 60s timeout: TransferAccepted may be delayed if data from a prior transfer
    // is still draining through the shared frame channel.
    match tokio::time::timeout(std::time::Duration::from_secs(60), response_rx).await {
        Ok(Ok(ControlMessage::TransferAccepted { .. })) => {
            tracing::info!("Remote accepted transfer");

            let _ = ws_tx.send(Message::Text(
                serde_json::to_string(&ControlMessage::TransferAccepted {
                    id,
                    resume_offsets: std::collections::HashMap::new(),
                }).unwrap().into()
            ));

            match direction {
                Direction::Push => {
                    if !entries.is_empty() {
                        push_entries(&state, id, &entries, &ws_tx).await;
                    }
                }
                Direction::Pull => {
                    let done_rx = pull_done_rx.expect("pull_done_rx set above for Pull");

                    match tokio::time::timeout(std::time::Duration::from_secs(1800), done_rx).await {
                        Ok(Ok(())) => {
                            tracing::info!("Pull transfer complete: {}", id);
                            let _ = ws_tx.send(Message::Text(
                                serde_json::to_string(&ControlMessage::TransferComplete { id })
                                    .unwrap().into()
                            ));
                        }
                        Ok(Err(_)) => send_error(&ws_tx, id, "Pull transfer channel closed unexpectedly"),
                        Err(_) => send_error(&ws_tx, id, "Pull transfer timed out"),
                    }
                }
            }
        }
        Ok(Ok(ControlMessage::TransferError { error, .. })) => send_error(&ws_tx, id, &error),
        Ok(Ok(_)) => send_error(&ws_tx, id, "Unexpected response from remote"),
        Ok(Err(_)) => send_error(&ws_tx, id, "Remote response channel closed"),
        Err(_) => send_error(&ws_tx, id, "Remote response timeout"),
    }
}

/// Read files from `root_dir` and stream them to the remote via the unified frame channel.
/// Used by both Push (local → remote) and Pull (remote reads and sends back to requester).
pub async fn send_entries(
    root_dir: &std::path::Path,
    id: Uuid,
    entries: &[TransferEntry],
    frame_tx: &FrameChannel,
) {
    let mut files_to_send: Vec<(String, PathBuf, u64, Option<PathBuf>)> = Vec::new();

    for entry in entries {
        if entry.is_dir {
            match compress::compress_directory(root_dir, &entry.relative_path) {
                Ok((archive_path, archive_size)) => {
                    files_to_send.push((
                        entry.relative_path.clone(),
                        archive_path.clone(),
                        archive_size,
                        Some(archive_path),
                    ));
                }
                Err(e) => {
                    send_control(frame_tx, &ControlMessage::TransferError {
                        id,
                        error: format!("Failed to compress {}: {}", entry.relative_path, e),
                    });
                    cleanup_archives(&files_to_send);
                    return;
                }
            }
        } else {
            let file_path = root_dir.join(&entry.relative_path);
            files_to_send.push((entry.relative_path.clone(), file_path, entry.size, None));
        }
    }

    let mut total_sent: u64 = 0;

    for (display_name, file_path, _file_size, _cleanup) in &files_to_send {
        match ChunkedReader::open(file_path, 0).await {
            Ok(mut reader) => {
                tracing::info!("Sending: {} ({} bytes)", display_name, reader.total_size());

                while let Ok(Some((_offset, chunk))) = reader.read_chunk().await {
                    let frame = encode_data_frame(id, total_sent, &chunk);
                    if frame_tx.send(frame).is_err() {
                        send_control(frame_tx, &ControlMessage::TransferError {
                            id,
                            error: "Connection lost while sending".to_string(),
                        });
                        cleanup_archives(&files_to_send);
                        return;
                    }
                    total_sent += chunk.len() as u64;
                }
            }
            Err(e) => {
                send_control(frame_tx, &ControlMessage::TransferError {
                    id,
                    error: format!("Failed to open {}: {}", display_name, e),
                });
                cleanup_archives(&files_to_send);
                return;
            }
        }
    }

    send_control(frame_tx, &ControlMessage::TransferComplete { id });
    tracing::info!("send_entries complete: {} ({} bytes)", id, total_sent);

    cleanup_archives(&files_to_send);
}

/// Push entries to remote — reads local files and streams through the unified frame channel.
async fn push_entries(
    state: &AppState,
    id: Uuid,
    entries: &[TransferEntry],
    ws_tx: &mpsc::UnboundedSender<Message>,
) {
    let frame_tx = {
        let remote = state.remote.read().await;
        remote.as_ref().map(|r| r.frame_tx.clone())
    };

    let Some(frame_tx) = frame_tx else {
        send_error(ws_tx, id, "Remote connection lost");
        return;
    };

    let mut files_to_send: Vec<(String, PathBuf, u64, Option<PathBuf>)> = Vec::new();

    for entry in entries {
        if entry.is_dir {
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
            files_to_send.push((entry.relative_path.clone(), file_path, entry.size, None));
        }
    }

    let total_size: u64 = files_to_send.iter().map(|(_, _, s, _)| s).sum();
    let mut total_sent: u64 = 0;

    for (display_name, file_path, _file_size, _cleanup) in &files_to_send {
        match ChunkedReader::open(file_path, 0).await {
            Ok(mut reader) => {
                tracing::info!("Sending: {} ({} bytes)", display_name, reader.total_size());

                while let Ok(Some((_offset, chunk))) = reader.read_chunk().await {
                    let frame = encode_data_frame(id, total_sent, &chunk);
                    if frame_tx.send(frame).is_err() {
                        send_error(ws_tx, id, "Connection to remote lost");
                        cleanup_archives(&files_to_send);
                        return;
                    }
                    total_sent += chunk.len() as u64;

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

    send_control(&frame_tx, &ControlMessage::TransferComplete { id });

    let _ = ws_tx.send(Message::Text(
        serde_json::to_string(&ControlMessage::TransferComplete { id }).unwrap().into()
    ));
    tracing::info!("Push complete: {} ({} bytes)", id, total_sent);

    cleanup_archives(&files_to_send);
}

fn send_control(frame_tx: &FrameChannel, msg: &ControlMessage) {
    let json = serde_json::to_string(msg).unwrap();
    let _ = frame_tx.send(encode_control_frame(json.as_bytes()));
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
