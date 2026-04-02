use std::path::Path;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::crypto::stream::CryptoStream;
use crate::fileops::compress;
use crate::fileops::reader::ChunkedReader;
use crate::protocol::codec::encode_data_frame;
use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};

use super::perform_client_handshake;

/// Send a file or folder to a remote drift server without starting a web panel.
/// Connects, handshakes, transfers, and exits.
pub async fn send_file(
    target: &str,
    file_path: &Path,
    password: &Option<String>,
) -> anyhow::Result<()> {
    // Resolve file path
    let file_path = file_path.canonicalize()?;
    let file_name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?
        .to_string_lossy()
        .to_string();

    let is_dir = file_path.is_dir();
    let metadata = std::fs::metadata(&file_path)?;

    tracing::info!("Preparing to send: {}", file_path.display());

    // If directory, compress first
    let (actual_path, actual_size, cleanup_path) = if is_dir {
        let parent = file_path.parent().unwrap_or(Path::new("."));
        let (archive_path, archive_size) = compress::compress_directory(parent, &file_name)?;
        let cleanup = archive_path.clone();
        (archive_path, archive_size, Some(cleanup))
    } else {
        (file_path.clone(), metadata.len(), None)
    };

    // Connect to remote
    let url = format!("ws://{}/ws", target);
    tracing::info!("Connecting to {}", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
    tracing::info!("Connected");

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Handshake
    let crypto = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Encrypted connection established");

    // Build transfer request
    let transfer_id = Uuid::new_v4();
    let entry = TransferEntry {
        relative_path: file_name.clone(),
        size: actual_size,
        is_dir,
        #[cfg(unix)]
        permissions: {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        },
    };

    let request = ControlMessage::TransferRequest {
        id: transfer_id,
        entries: vec![entry],
        direction: Direction::Push,
    };

    // Send request
    send_encrypted_msg(&crypto, &mut ws_write, &request).await?;
    tracing::info!("Transfer request sent, waiting for acceptance...");

    // Wait for TransferAccepted
    loop {
        let msg = recv_encrypted_msg(&crypto, &mut ws_read).await?;
        match msg {
            ControlMessage::TransferAccepted { id, .. } if id == transfer_id => {
                tracing::info!("Transfer accepted");
                break;
            }
            ControlMessage::TransferError { error, .. } => {
                if let Some(ref p) = cleanup_path {
                    compress::cleanup_archive(p);
                }
                anyhow::bail!("Transfer rejected: {}", error);
            }
            _ => {
                // Ignore other messages (InfoRequest, etc) during handshake
                continue;
            }
        }
    }

    // Send file data
    let mut reader = ChunkedReader::open(&actual_path, 0).await?;
    let total = reader.total_size();
    let mut sent: u64 = 0;
    let mut last_percent: u64 = 0;

    tracing::info!("Sending {} ({} bytes)...", file_name, total);

    while let Some((offset, chunk)) = reader.read_chunk().await? {
        let frame = encode_data_frame(transfer_id, offset, &chunk);

        // Encrypt and send binary frame
        let ciphertext = crypto.encrypt(&frame)?;
        ws_write.send(Message::Binary(ciphertext.into())).await?;

        sent += chunk.len() as u64;

        // Print progress every 10%
        let percent = if total > 0 { sent * 100 / total } else { 100 };
        if percent / 10 > last_percent / 10 {
            tracing::info!("  {}% ({}/{})", percent, format_bytes(sent), format_bytes(total));
            last_percent = percent;
        }
    }

    // Send TransferComplete
    send_encrypted_msg(&crypto, &mut ws_write, &ControlMessage::TransferComplete {
        id: transfer_id,
    }).await?;

    tracing::info!("Transfer complete: {} sent to {}", file_name, target);

    // Clean up temp archive
    if let Some(ref p) = cleanup_path {
        compress::cleanup_archive(p);
    }

    // Close connection
    let _ = ws_write.send(Message::Close(None)).await;

    Ok(())
}

async fn send_encrypted_msg(
    crypto: &CryptoStream,
    ws_write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    msg: &ControlMessage,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(msg)?;
    let ciphertext = crypto.encrypt(json.as_bytes())?;
    let encoded = BASE64.encode(&ciphertext);
    ws_write.send(Message::Text(encoded.into())).await?;
    Ok(())
}

async fn recv_encrypted_msg(
    crypto: &CryptoStream,
    ws_read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> anyhow::Result<ControlMessage> {
    loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                let ciphertext = BASE64.decode(text.as_bytes())?;
                let plaintext = crypto.decrypt(&ciphertext)?;
                let msg: ControlMessage = serde_json::from_str(std::str::from_utf8(&plaintext)?)?;
                return Ok(msg);
            }
            Some(Ok(Message::Close(_))) => {
                anyhow::bail!("Connection closed by remote");
            }
            Some(Err(e)) => {
                anyhow::bail!("WebSocket error: {}", e);
            }
            None => {
                anyhow::bail!("Connection closed");
            }
            _ => continue, // Skip binary/ping/pong
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}
