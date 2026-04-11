use std::path::Path;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::crypto::stream::CryptoStream;
use crate::fileops::decompress;
use crate::fileops::writer::ChunkedWriter;
use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};

use super::{perform_client_handshake, recv_encrypted_frame, DecryptedFrame, WsRead, WsWrite};
use super::send::{send_encrypted_control, recv_encrypted_control, format_bytes};

/// Connect to a remote drift server and pull a file or folder.
pub async fn pull_remote(
    target: &str,
    remote_path: &str,
    output_dir: Option<&Path>,
    password: &Option<String>,
) -> anyhow::Result<()> {
    let url = format!("ws://{}/ws", target);
    tracing::info!("Connecting to {}", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (crypto, fp) = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Encrypted connection established (fingerprint: {})", fp);

    // Discover file metadata by browsing the parent directory
    let (parent, file_name) = split_remote_path(remote_path);

    send_encrypted_control(&crypto, &mut ws_write, &ControlMessage::BrowseRequest {
        path: parent.to_string(),
    }).await?;

    let response = recv_encrypted_control(&crypto, &mut ws_read).await?;

    let entry_info = match response {
        ControlMessage::BrowseResponse { entries, .. } => {
            entries.into_iter()
                .find(|e| e.name == file_name)
                .ok_or_else(|| anyhow::anyhow!("'{}' not found on remote", remote_path))?
        }
        ControlMessage::Error { message } => {
            anyhow::bail!("Browse failed: {}", message);
        }
        other => {
            anyhow::bail!("Unexpected response: {:?}", other);
        }
    };

    let is_dir = entry_info.is_dir;
    let expected_size = entry_info.size;

    tracing::info!("Found: {} ({}, {})",
        file_name,
        if is_dir { "directory" } else { "file" },
        format_bytes(expected_size),
    );

    // Build transfer request
    let transfer_id = Uuid::new_v4();
    let entry = TransferEntry {
        relative_path: remote_path.to_string(),
        size: expected_size,
        is_dir,
        #[cfg(unix)]
        permissions: entry_info.permissions,
    };

    let destination = output_dir.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| ".".to_string());

    send_encrypted_control(&crypto, &mut ws_write, &ControlMessage::TransferRequest {
        id: transfer_id,
        entries: vec![entry],
        direction: Direction::Pull,
        destination_path: destination,
    }).await?;

    // Wait for acceptance
    loop {
        let msg = recv_encrypted_control(&crypto, &mut ws_read).await?;
        match msg {
            ControlMessage::TransferAccepted { id, .. } if id == transfer_id => {
                tracing::info!("Transfer accepted");
                break;
            }
            ControlMessage::TransferError { error, .. } => {
                anyhow::bail!("Transfer rejected: {}", error);
            }
            _ => continue,
        }
    }

    // Determine output paths
    let output = output_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));

    let (write_path, archive_cleanup) = if is_dir {
        let drift_dir = output.join(".drift");
        std::fs::create_dir_all(&drift_dir)?;
        let archive_path = drift_dir.join(format!("{}.tar.gz", transfer_id));
        (archive_path.clone(), Some(archive_path))
    } else {
        (output.join(&file_name), None)
    };

    // Receive data
    tracing::info!("Pulling {} ...", file_name);
    receive_transfer(
        &crypto, &mut ws_write, &mut ws_read,
        transfer_id, &write_path,
    ).await?;

    // Decompress if directory
    if is_dir {
        if let Some(ref archive_path) = archive_cleanup {
            tracing::info!("Extracting directory...");
            decompress::decompress_archive(archive_path, &output)?;
        }
    }

    tracing::info!("Pull complete: {} → {}", file_name, output.display());

    let _ = ws_write.send(Message::Close(None)).await;
    Ok(())
}

/// Receive all data frames for a transfer, write to disk, and send TransferFinalized.
async fn receive_transfer(
    crypto: &CryptoStream,
    ws_write: &mut WsWrite,
    ws_read: &mut WsRead,
    transfer_id: Uuid,
    write_path: &Path,
) -> anyhow::Result<()> {
    let mut writer = ChunkedWriter::create(write_path).await?;
    let mut received: u64 = 0;
    let mut last_percent: u64 = 0;

    loop {
        match recv_encrypted_frame(crypto, ws_read).await? {
            DecryptedFrame::Data { transfer_id: id, chunk, .. } if id == transfer_id => {
                received += chunk.len() as u64;
                writer.write_chunk(&chunk).await?;

                // Log progress at 10% intervals if we have a size hint
                let percent = received / (1024 * 1024); // per-MB marker
                if percent > last_percent {
                    tracing::info!("  received {}", format_bytes(received));
                    last_percent = percent;
                }
            }
            DecryptedFrame::Control(ControlMessage::TransferComplete { id, total_bytes }) if id == transfer_id => {
                writer.finalize().await?;
                tracing::info!("  {} received", format_bytes(total_bytes));

                // Acknowledge
                send_encrypted_control(crypto, ws_write, &ControlMessage::TransferFinalized {
                    id: transfer_id,
                }).await?;

                return Ok(());
            }
            DecryptedFrame::Control(ControlMessage::TransferError { id, error }) if id == transfer_id => {
                anyhow::bail!("Transfer error: {}", error);
            }
            _ => continue, // skip unrelated frames
        }
    }
}

/// Split a remote path into (parent, filename).
/// "foo/bar.txt" → ("foo", "bar.txt")
/// "bar.txt" → (".", "bar.txt")
fn split_remote_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(pos) => (&path[..pos], &path[pos + 1..]),
        None => (".", path),
    }
}
