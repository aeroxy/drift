use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::fileops::writer::ChunkedWriter;
use crate::fileops::decompress;
use crate::protocol::messages::TransferEntry;

pub struct ActiveTransfer {
    pub entries: Vec<TransferEntry>,
    pub current_writer: Option<ChunkedWriter>,
    pub bytes_written: u64,
    pub has_dirs: bool,
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

    pub async fn start_transfer(&self, id: Uuid, entries: Vec<TransferEntry>) {
        tracing::info!("Starting to receive transfer: {} ({} entries)", id, entries.len());
        let has_dirs = entries.iter().any(|e| e.is_dir);

        // If transfer includes directories, write to a temp archive in .drift/
        let mut active = self.active_transfers.lock().await;
        active.insert(id, ActiveTransfer {
            entries,
            current_writer: None,
            bytes_written: 0,
            has_dirs,
        });
    }

    pub async fn receive_chunk(&self, id: Uuid, _offset: u64, data: &[u8]) -> Result<(), String> {
        let mut active = self.active_transfers.lock().await;

        let transfer = active.get_mut(&id)
            .ok_or_else(|| format!("Transfer {} not found", id))?;

        // Initialize writer if needed
        if transfer.current_writer.is_none() {
            let file_path = if transfer.has_dirs {
                // Write to .drift/ temp directory as archive
                let drift_dir = self.root_dir.join(".drift");
                std::fs::create_dir_all(&drift_dir)
                    .map_err(|e| format!("Failed to create .drift dir: {}", e))?;
                drift_dir.join(format!("{}.tar.gz", id))
            } else {
                // Single file: write directly to root_dir
                let entry = &transfer.entries[0];
                self.root_dir.join(&entry.relative_path)
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

        Ok(())
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
                    tracing::info!("Decompressing archive: {:?}", archive_path);
                    decompress::decompress_archive(&archive_path, &self.root_dir)
                        .map_err(|e| format!("Failed to decompress: {}", e))?;
                }
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
