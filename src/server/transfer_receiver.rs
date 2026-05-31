use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

use crate::fileops::decompress;
use crate::fileops::writer::ChunkedWriter;
use crate::protocol::messages::TransferEntry;

pub struct ActiveTransfer {
    pub entries: Vec<TransferEntry>,
    /// Writers indexed by file_index (for multi-file transfers)
    pub writers: HashMap<u32, ChunkedWriter>,
    /// Bytes written per file (by file_index)
    pub bytes_per_file: HashMap<u32, u64>,
    /// Total bytes written across all files
    pub total_bytes_written: u64,
    pub has_dirs: bool,
    pub destination_path: String,
    /// Set when TransferComplete arrives, triggering auto-finalize in receive_chunk.
    expected_total: Option<u64>,
    completion_tx: Option<oneshot::Sender<Result<u64, String>>>,
}

pub struct TransferReceiver {
    root_dir: PathBuf,
    active_transfers: Arc<Mutex<HashMap<Uuid, ActiveTransfer>>>,
}

/// Reject destination paths that could escape `root_dir`. A legitimate
/// `destination_path` is always a relative subpath produced by panel navigation
/// (or `.`); an absolute path or any `..` component signals a traversal attempt
/// from a malicious browser or peer. The check is lexical (no filesystem access)
/// so it holds even when the target does not yet exist — closing the TOCTOU gap
/// where validating only existing parents lets non-existent traversal targets through.
fn validate_destination_path(destination_path: &str) -> Result<(), String> {
    use std::path::Component;
    for component in std::path::Path::new(destination_path).components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(format!(
                "Invalid destination path (escapes root): {}",
                destination_path
            ));
        }
    }
    Ok(())
}

impl TransferReceiver {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            active_transfers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start_transfer(
        &self,
        id: Uuid,
        entries: Vec<TransferEntry>,
        destination_path: String,
    ) -> Result<(), String> {
        validate_destination_path(&destination_path)?;
        tracing::info!(
            "Starting to receive transfer: {} ({} entries) to {}",
            id,
            entries.len(),
            destination_path
        );
        let has_dirs = entries.iter().any(|e| e.is_dir);

        let mut active = self.active_transfers.lock().await;
        active.insert(
            id,
            ActiveTransfer {
                entries,
                writers: HashMap::new(),
                bytes_per_file: HashMap::new(),
                total_bytes_written: 0,
                has_dirs,
                destination_path,
                expected_total: None,
                completion_tx: None,
            },
        );
        Ok(())
    }

    /// Like `start_transfer` but returns a receiver that fires once `finalize_transfer` completes.
    /// Used by Pull transfers so the browser-side handler can wait for the download to finish.
    pub async fn start_transfer_with_notify(
        &self,
        id: Uuid,
        entries: Vec<TransferEntry>,
        destination_path: String,
    ) -> Result<oneshot::Receiver<Result<u64, String>>, String> {
        validate_destination_path(&destination_path)?;
        tracing::info!(
            "Starting to receive transfer (with notify): {} ({} entries) to {}",
            id,
            entries.len(),
            destination_path
        );
        let has_dirs = entries.iter().any(|e| e.is_dir);
        let (tx, rx) = oneshot::channel();

        let mut active = self.active_transfers.lock().await;
        active.insert(
            id,
            ActiveTransfer {
                entries,
                writers: HashMap::new(),
                bytes_per_file: HashMap::new(),
                total_bytes_written: 0,
                has_dirs,
                destination_path,
                expected_total: None,
                completion_tx: Some(tx),
            },
        );

        Ok(rx)
    }

    /// Write a chunk into the active transfer.
    /// `file_index` identifies which file within the transfer this chunk belongs to.
    /// Returns Ok(true) if the transfer was auto-finalized (all expected bytes received),
    /// Ok(false) if still in progress, or Err on write failure.
    pub async fn receive_chunk(
        &self,
        id: Uuid,
        file_index: u32,
        _offset: u64,
        data: &[u8],
    ) -> Result<bool, String> {
        let mut active = self.active_transfers.lock().await;

        // Unknown transfer — silently drop the chunk. This can happen during Pull setup
        // when the remote starts sending binary frames before TransferAccepted is processed.
        let Some(transfer) = active.get_mut(&id) else {
            tracing::warn!("Dropping chunk for unknown transfer {}", id);
            return Ok(false);
        };

        // Get or create writer for this file_index
        if !transfer.writers.contains_key(&file_index) {
            // Get the entry for this file_index
            let entry = transfer
                .entries
                .get(file_index as usize)
                .ok_or_else(|| format!("Invalid file_index {} for transfer {}", file_index, id))?;

            // Stage temp files under the *destination* directory rather than the
            // served root. The root may be read-only (e.g. drift launched from `/`)
            // even when the chosen destination is writable. Co-locating `.drift` with
            // the destination also keeps the temp file on the same filesystem as the
            // final path, so the finalize rename stays atomic (no cross-device EXDEV).
            let dest_dir = self.root_dir.join(&transfer.destination_path);
            let drift_dir = dest_dir.join(".drift");

            let (temp_path, final_path) = if entry.is_dir {
                // Directory: stage archive in .drift/, finalize renames in-place
                let archive = drift_dir.join(format!("{}_{}.tar.gz", id, file_index));
                (archive.clone(), archive)
            } else {
                // Regular file: stage in .drift/, finalize moves to destination
                let file_name = std::path::Path::new(&entry.relative_path)
                    .file_name()
                    .ok_or_else(|| format!("Invalid path: {}", entry.relative_path))?;

                let dest_path = dest_dir.join(file_name);

                // Validate that the destination is within root_dir (path traversal protection)
                let root_canonical = self
                    .root_dir
                    .canonicalize()
                    .map_err(|e| format!("Invalid root: {}", e))?;
                if let Some(parent) = dest_path.parent() {
                    if parent.exists() {
                        let parent_canonical = parent
                            .canonicalize()
                            .map_err(|e| format!("Invalid parent path: {}", e))?;
                        if !parent_canonical.starts_with(&root_canonical) {
                            return Err("Path traversal attempt blocked".to_string());
                        }
                    }
                }

                let temp = drift_dir.join(format!(
                    "{}_{}_{}", id, file_index,
                    file_name.to_string_lossy()
                ));
                (temp, dest_path)
            };

            tracing::info!(
                "Creating writer for file_index={}: temp={:?}, final={:?}",
                file_index,
                temp_path,
                final_path
            );
            let writer = ChunkedWriter::create_with_temp(temp_path, final_path)
                .await
                .map_err(|e| format!("Failed to create writer: {}", e))?;

            transfer.writers.insert(file_index, writer);
        }

        // Write chunk
        if let Some(writer) = transfer.writers.get_mut(&file_index) {
            writer
                .write_chunk(data)
                .await
                .map_err(|e| format!("Failed to write chunk: {}", e))?;

            *transfer.bytes_per_file.entry(file_index).or_insert(0) += data.len() as u64;
            transfer.total_bytes_written += data.len() as u64;

            tracing::debug!(
                "Received chunk: id={}, file_index={}, size={}, total={}, file_total={}",
                id,
                file_index,
                data.len(),
                transfer.total_bytes_written,
                transfer.bytes_per_file[&file_index]
            );
        }

        // Auto-finalize if we've received all expected bytes
        if let Some(expected) = transfer.expected_total {
            if transfer.total_bytes_written >= expected {
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
            tracing::warn!(
                "signal_completion for unknown transfer {} — already finalized?",
                id
            );
            return Ok(true);
        };

        transfer.expected_total = Some(total_bytes);

        if transfer.total_bytes_written >= total_bytes {
            // All bytes already received; finalize now.
            tracing::info!(
                "signal_completion: all bytes already received for {}, finalizing",
                id
            );
            drop(active);
            self.finalize_transfer(id).await?;
            Ok(true)
        } else {
            tracing::info!(
                "signal_completion: {}/{} bytes received for {}, waiting for remaining",
                transfer.total_bytes_written,
                total_bytes,
                id
            );
            Ok(false)
        }
    }

    pub async fn finalize_transfer(&self, id: Uuid) -> Result<(), String> {
        let mut active = self.active_transfers.lock().await;

        if let Some(mut transfer) = active.remove(&id) {
            let has_dirs = transfer.has_dirs;
            let file_count = transfer.writers.len();

            // Collect all potential temp paths and final destination paths for atomic rollback.
            // We must collect these before draining transfer.writers.
            let mut temp_paths = Vec::new();
            let mut dest_paths = Vec::new();
            for (&idx, writer) in &transfer.writers {
                let entry = transfer.entries.get(idx as usize);
                if entry.map(|e| e.is_dir).unwrap_or(false) {
                    temp_paths.push(writer.temp_path().to_path_buf());
                } else {
                    temp_paths.push(writer.temp_path().to_path_buf());
                    dest_paths.push(writer.final_path().to_path_buf());
                }
            }

            // Ensure any directories that didn't have writers open are also handled
            for (idx, entry) in transfer.entries.iter().enumerate() {
                if entry.is_dir {
                    let path = self.archive_path(&transfer.destination_path, id, idx);
                    if !temp_paths.contains(&path) {
                        temp_paths.push(path);
                    }
                }
            }

            // Finalize all writers (take ownership from HashMap).
            // For regular files this renames .drift/temp → destination.
            // For directories this is a no-op rename (archive stays in .drift/).
            let mut finalize_failed = false;
            let mut finalize_err = String::new();
            for (file_index, writer) in transfer.writers.drain() {
                if let Err(e) = writer.finalize().await {
                    finalize_failed = true;
                    finalize_err = format!("Failed to finalize file {}: {}", file_index, e);
                    break;
                }
            }

            if finalize_failed {
                // Finalization failed — clean up all temp files and any successfully finalized files
                let mut rollback_paths = temp_paths;
                rollback_paths.extend(dest_paths);
                Self::remove_files(rollback_paths).await;

                if let Some(tx) = transfer.completion_tx.take() {
                    let _ = tx.send(Err(finalize_err.clone()));
                }
                return Err(finalize_err);
            }

            tracing::info!(
                "Transfer finalized: {} ({} bytes across {} files)",
                id,
                transfer.total_bytes_written,
                file_count
            );

            // If this was a directory transfer, decompress each directory's archive
            if has_dirs {
                let dest_dir = self.root_dir.join(&transfer.destination_path);

                // Validate destination directory (path traversal protection)
                if dest_dir.exists() {
                    let dest_canonical = dest_dir
                        .canonicalize()
                        .map_err(|e| format!("Invalid destination: {}", e))?;
                    let root_canonical = self
                        .root_dir
                        .canonicalize()
                        .map_err(|e| format!("Invalid root: {}", e))?;
                    if !dest_canonical.starts_with(&root_canonical) {
                        return Err("Path traversal attempt blocked".to_string());
                    }
                }

                for (idx, entry) in transfer.entries.iter().enumerate() {
                    if entry.is_dir {
                        let archive_path = self.archive_path(&transfer.destination_path, id, idx);
                        tracing::info!(
                            "Decompressing archive {:?} to {:?}",
                            archive_path,
                            dest_dir
                        );
                        if let Err(e) = decompress::decompress_archive(&archive_path, &dest_dir) {
                            let err_msg = format!(
                                "Failed to decompress {}: {}", entry.relative_path, e
                            );

                            // Decompression failed — clean up all temp files and any regular files already moved
                            let mut rollback_paths = temp_paths;
                            rollback_paths.extend(dest_paths);
                            Self::remove_files(rollback_paths).await;

                            if let Some(tx) = transfer.completion_tx.take() {
                                let _ = tx.send(Err(err_msg.clone()));
                            }
                            return Err(err_msg);
                        }

                        // Archive consumed successfully — clean it up
                        let _ = tokio::fs::remove_file(&archive_path).await;
                    }
                }
            }

            // Best-effort: remove the now-empty .drift staging dir from the
            // destination. `remove_dir` only succeeds when the directory is empty,
            // so concurrent transfers staging into the same dir are unaffected.
            let _ = tokio::fs::remove_dir(
                self.root_dir.join(&transfer.destination_path).join(".drift"),
            )
            .await;

            // Notify any waiters (e.g. Pull transfers waiting for completion)
            if let Some(tx) = transfer.completion_tx {
                let _ = tx.send(Ok(transfer.total_bytes_written));
            }
        }

        Ok(())
    }

    /// Path to the temporary tar.gz archive for a directory entry in a transfer.
    /// Staged under the destination directory (see `receive_chunk`), not the served root.
    fn archive_path(&self, destination_path: &str, id: Uuid, index: usize) -> std::path::PathBuf {
        self.root_dir
            .join(destination_path)
            .join(".drift")
            .join(format!("{}_{}.tar.gz", id, index))
    }

    /// Remove a list of files concurrently, logging warnings for unexpected errors.
    /// NotFound is silently ignored (the file may never have been created).
    async fn remove_files(paths: Vec<PathBuf>) {
        let futs = paths.into_iter().map(|p| async move {
            if let Err(e) = tokio::fs::remove_file(&p).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("Cleanup: failed to remove {}: {}", p.display(), e);
                }
            }
        });
        futures_util::future::join_all(futs).await;
    }

    /// Signal that the sender encountered an error for this transfer.
    /// Cleans up any partial state and notifies waiters with the error.
    /// Returns true if an active transfer was found and removed.
    pub async fn signal_error(&self, id: Uuid, error: String) -> bool {
        let mut active = self.active_transfers.lock().await;
        let Some(transfer) = active.remove(&id) else {
            return false;
        };
        drop(active);

        tracing::error!("Transfer error for {}: {}", id, error);

        // Collect temp paths, then drop writers to release file handles before deletion.
        let paths: Vec<PathBuf> = transfer
            .writers
            .into_iter()
            .map(|(_, writer)| {
                let p = writer.temp_path().to_path_buf();
                drop(writer);
                p
            })
            .collect();
        Self::remove_files(paths).await;

        if let Some(tx) = transfer.completion_tx {
            let _ = tx.send(Err(error));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::validate_destination_path;

    #[test]
    fn accepts_legitimate_relative_destinations() {
        for p in [".", "sub", "a/b/c", "client-sub1/client-sub2"] {
            assert!(validate_destination_path(p).is_ok(), "should accept {p:?}");
        }
    }

    #[test]
    fn rejects_traversal_and_absolute_destinations() {
        for p in [
            "..",
            "../escape",
            "a/../../escape",
            "sub/..",
            "/etc/passwd",
            "/",
        ] {
            assert!(validate_destination_path(p).is_err(), "should reject {p:?}");
        }
    }
}
