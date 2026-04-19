use std::path::Path;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::crypto::stream::CryptoStream;
use crate::fileops::compress;
use crate::fileops::reader::ChunkedReader;
use crate::protocol::codec::{
    encode_data_frame, encode_control_frame, decode_frame_type,
    FRAME_TYPE_CONTROL,
};
use crate::protocol::messages::{ControlMessage, Direction, TransferEntry};

use super::perform_client_handshake;

/// Send a file or folder to a remote drift server without starting a web panel.
/// Connects, handshakes, transfers, and exits.
pub async fn send_file(
    target: &str,
    file_path: &Path,
    password: &Option<String>,
    allow_insecure_tls: bool,
) -> anyhow::Result<()> {
    let file_path = file_path.canonicalize()?;
    let file_name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?
        .to_string_lossy()
        .to_string();

    let is_dir = file_path.is_dir();
    let metadata = std::fs::metadata(&file_path)?;

    tracing::info!("Preparing to send: {}", file_path.display());

    let (actual_path, actual_size, cleanup_path) = if is_dir {
        let parent = file_path.parent().unwrap_or(Path::new("."));
        let (archive_path, archive_size) = compress::compress_directory(parent, &file_name)?;
        let cleanup = archive_path.clone();
        (archive_path, archive_size, Some(cleanup))
    } else {
        (file_path.clone(), metadata.len(), None)
    };

    let (ws_stream, _) = super::open_ws(target, allow_insecure_tls).await?;
    tracing::info!("Connected");

    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (crypto, fp) = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Encrypted connection established (fingerprint: {})", fp);

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
        destination_path: ".".to_string(),
    };

    send_encrypted_control(&crypto, &mut ws_write, &request).await?;
    tracing::info!("Transfer request sent, waiting for acceptance...");

    loop {
        let msg = recv_encrypted_control(&crypto, &mut ws_read).await?;
        match msg {
            ControlMessage::TransferAccepted { id, .. } if id == transfer_id => {
                tracing::info!("Transfer accepted");
                break;
            }
            ControlMessage::TransferError { error, .. } => {
                if let Some(ref p) = cleanup_path { compress::cleanup_archive(p); }
                anyhow::bail!("Transfer rejected: {}", error);
            }
            _ => continue,
        }
    }

    let mut reader = ChunkedReader::open(&actual_path, 0).await?;
    let total = reader.total_size();
    let mut sent: u64 = 0;
    let mut last_percent: u64 = 0;

    tracing::info!("Sending {} ({} bytes)...", file_name, total);

    while let Some((_offset, chunk)) = reader.read_chunk().await? {
        let frame = encode_data_frame(transfer_id, sent, &chunk);
        let ciphertext = crypto.encrypt(&frame)?;
        ws_write.send(Message::Binary(ciphertext.into())).await?;

        sent += chunk.len() as u64;

        let percent = if total > 0 { sent * 100 / total } else { 100 };
        if percent / 10 > last_percent / 10 {
            tracing::info!("  {}% ({}/{})", percent, format_bytes(sent), format_bytes(total));
            last_percent = percent;
        }
    }

    send_encrypted_control(&crypto, &mut ws_write, &ControlMessage::TransferComplete {
        id: transfer_id,
        total_bytes: sent,
    }).await?;

    tracing::info!("Transfer complete: {} sent to {}", file_name, target);

    if let Some(ref p) = cleanup_path { compress::cleanup_archive(p); }
    let _ = ws_write.send(Message::Close(None)).await;

    Ok(())
}

/// Encode and send a control message as an encrypted binary frame.
pub(crate) async fn send_encrypted_control(
    crypto: &CryptoStream,
    ws_write: &mut super::WsWrite,
    msg: &ControlMessage,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(msg)?;
    let frame = encode_control_frame(json.as_bytes());
    let ciphertext = crypto.encrypt(&frame)?;
    ws_write.send(Message::Binary(ciphertext.into())).await?;
    Ok(())
}

/// Receive one encrypted binary frame and parse it as a control message.
pub(crate) async fn recv_encrypted_control(
    crypto: &CryptoStream,
    ws_read: &mut super::WsRead,
) -> anyhow::Result<ControlMessage> {
    loop {
        match ws_read.next().await {
            Some(Ok(Message::Binary(encrypted))) => {
                let plaintext = crypto.decrypt(&encrypted)?;
                let (frame_type, payload) = decode_frame_type(&plaintext)?;
                if frame_type != FRAME_TYPE_CONTROL {
                    tracing::warn!("Expected control frame, got type {:#x} — skipping", frame_type);
                    continue;
                }
                let msg: ControlMessage = serde_json::from_slice(payload)?;
                return Ok(msg);
            }
            Some(Ok(Message::Close(_))) => anyhow::bail!("Connection closed by remote"),
            Some(Err(e)) => anyhow::bail!("WebSocket error: {}", e),
            None => anyhow::bail!("Connection closed"),
            _ => continue,
        }
    }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
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
