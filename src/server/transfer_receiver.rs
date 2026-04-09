use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

use crate::fileops::writer::ChunkedWriter;
use crate::fileops::decompress;
use crate::protocol::messages::TransferEntry;

pub struct ActiveTransfer {
    pub entries: Vec<TransferEntry>,
    pub current_writer: Option<ChunkedWriter>,
    pub bytes_written: u64,
    pub has_dirs: bool,
    pub destination_path: String,
    /// Set when TransferComplete arrives, triggering auto-finalize in receive_chunk.
    expected_total: Option<u64>,
    completion_tx: Option<oneshot::Sender<()>>,
}

pub struct TransferReceiver {
    root_dir: PathBuf,
    active_transfers: Arc<Mutex<HashMap<Uuid, ActiveTransfer>>>,
}

impl TransferReceiver {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            active_transfers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start_transfer(&self, id: Uuid, entries: Vec<TransferEntry>, destination_path: String) {
        tracing::info!("Starting to receive transfer: {} ({} entries) to {}", id, entries.len(), destination_path);
        let has_dirs = entries.iter().any(|e| e.is_dir);

        let mut active = self.active_transfers.lock().await;
        active.insert(id, ActiveTransfer {
            entries,
            current_writer: None,
            bytes_written: 0,
            has_dirs,
            destination_path,
            expected_total: None,
            completion_tx: None,
        });
    }

    /// Like `start_transfer` but returns a receiver that fires once `finalize_transfer` completes.
    /// Used by Pull transfers so the browser-side handler can wait for the download to finish.
    pub async fn start_transfer_with_notify(&self, id: Uuid, entries: Vec<TransferEntry>, destination_path: String) -> oneshot::Receiver<()> {
        tracing::info!("Starting to receive transfer (with notify): {} ({} entries) to {}", id, entries.len(), destination_path);
        let has_dirs = entries.iter().any(|e| e.is_dir);
        let (tx, rx) = oneshot::channel();

        let mut active = self.active_transfers.lock().await;
        active.insert(id, ActiveTransfer {
            entries,
            current_writer: None,
            bytes_written: 0,
            has_dirs,
            destination_path,
            expected_total: None,
            completion_tx: Some(tx),
        });

        rx
    }

    /// Write a chunk into the active transfer.
    /// Returns Ok(true) if the transfer was auto-finalized (all expected bytes received),
    /// Ok(false) if still in progress, or Err on write failure.
    pub async fn receive_chunk(&self, id: Uuid, _offset: u64, data: &[u8]) -> Result<bool, String> {
        let mut active = self.active_transfers.lock().await;

        // Unknown transfer — silently drop the chunk. This can happen during Pull setup
        // when the remote starts sending binary frames before TransferAccepted is processed.
        let Some(transfer) = active.get_mut(&id) else {
            tracing::warn!("Dropping chunk for unknown transfer {}", id);
            return Ok(false);
        };

        // Initialize writer if needed
        if transfer.current_writer.is_none() {
            let file_path = if transfer.has_dirs {
                // Write to .drift/ temp directory as archive
                let drift_dir = self.root_dir.join(".drift");
                std::fs::create_dir_all(&drift_dir)
                    .map_err(|e| format!("Failed to create .drift dir: {}", e))?;
                drift_dir.join(format!("{}.tar.gz", id))
            } else {
                // Single file: write to destination directory using only the filename,
                // not the full relative_path (which includes subdirectories from the sender side).
                let entry = &transfer.entries[0];
                let file_name = std::path::Path::new(&entry.relative_path)
                    .file_name()
                    .ok_or_else(|| format!("Invalid path: {}", entry.relative_path))?;
                let dest_path = self.root_dir.join(&transfer.destination_path).join(file_name);

                // Validate that the path is within root_dir (path traversal protection)
                let root_canonical = self.root_dir.canonicalize()
                    .map_err(|e| format!("Invalid root: {}", e))?;
                if let Some(parent) = dest_path.parent() {
                    if parent.exists() {
                        let parent_canonical = parent.canonicalize()
                            .map_err(|e| format!("Invalid parent path: {}", e))?;
                        if !parent_canonical.starts_with(&root_canonical) {
                            return Err("Path traversal attempt blocked".to_string());
                        }
                    }
                }

                dest_path
            };

            tracing::info!("Creating writer for: {:?}", file_path);
            let writer = ChunkedWriter::create(&file_path).await
                .map_err(|e| format!("Failed to create writer: {}", e))?;

            transfer.current_writer = Some(writer);
        }

        // Write chunk
        if let Some(ref mut writer) = transfer.current_writer {
            writer.write_chunk(data).await
                .map_err(|e| format!("Failed to write chunk: {}", e))?;

            transfer.bytes_written += data.len() as u64;

            tracing::debug!("Received chunk: id={}, size={}, total={}",
                id, data.len(), transfer.bytes_written);
        }

        // Auto-finalize if we've received all expected bytes
        if let Some(expected) = transfer.expected_total {
            if transfer.bytes_written >= expected {
                tracing::info!("Auto-finalizing transfer {} ({} bytes)", id, expected);
                drop(active);
                self.finalize_transfer(id).await?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Signal that the sender has finished sending `total_bytes` total.
    /// If all bytes are already received, finalizes immediately and returns Ok(true).
    /// Otherwise stores the expected total and returns Ok(false) — finalization will
    /// happen automatically in receive_chunk when the last chunk arrives.
    pub async fn signal_completion(&self, id: Uuid, total_bytes: u64) -> Result<bool, String> {
        let mut active = self.active_transfers.lock().await;

        let Some(transfer) = active.get_mut(&id) else {
            tracing::warn!("signal_completion for unknown transfer {} — already finalized?", id);
            return Ok(true);
        };

        transfer.expected_total = Some(total_bytes);

        if transfer.bytes_written >= total_bytes {
            // All bytes already received; finalize now.
            tracing::info!("signal_completion: all bytes already received for {}, finalizing", id);
            drop(active);
            self.finalize_transfer(id).await?;
            Ok(true)
        } else {
            tracing::info!(
                "signal_completion: {}/{} bytes received for {}, waiting for remaining",
                transfer.bytes_written, total_bytes, id
            );
            Ok(false)
        }
    }

    pub async fn finalize_transfer(&self, id: Uuid) -> Result<(), String> {
        let mut active = self.active_transfers.lock().await;

        if let Some(mut transfer) = active.remove(&id) {
            let has_dirs = transfer.has_dirs;

            if let Some(writer) = transfer.current_writer.take() {
                writer.finalize().await
                    .map_err(|e| format!("Failed to finalize: {}", e))?;

                tracing::info!("Transfer finalized: {} ({} bytes)", id, transfer.bytes_written);

                // If this was a directory transfer, decompress the archive
                if has_dirs {
                    let archive_path = self.root_dir.join(".drift").join(format!("{}.tar.gz", id));
                    let dest_dir = self.root_dir.join(&transfer.destination_path);

                    // Validate destination directory (path traversal protection)
                    if dest_dir.exists() {
                        let dest_canonical = dest_dir.canonicalize()
                            .map_err(|e| format!("Invalid destination: {}", e))?;
                        let root_canonical = self.root_dir.canonicalize()
                            .map_err(|e| format!("Invalid root: {}", e))?;
                        if !dest_canonical.starts_with(&root_canonical) {
                            return Err("Path traversal attempt blocked".to_string());
                        }
                    }

                    tracing::info!("Decompressing archive {:?} to {:?}", archive_path, dest_dir);
                    decompress::decompress_archive(&archive_path, &dest_dir)
                        .map_err(|e| format!("Failed to decompress: {}", e))?;
                }
            }

            // Notify any waiters (e.g. Pull transfers waiting for completion)
            if let Some(tx) = transfer.completion_tx {
                let _ = tx.send(());
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn abort_transfer(&self, id: Uuid) {
        self.active_transfers.lock().await.remove(&id);
        tracing::warn!("Transfer aborted: {}", id);
    }
}
