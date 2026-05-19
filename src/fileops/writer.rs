use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Compute the `.part` path for a given final path by appending `.part` to the
/// full filename.  E.g. `foo.tar.gz` → `foo.tar.gz.part`, `bar` → `bar.part`.
fn part_path_for(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".part");
    PathBuf::from(os)
}

#[allow(dead_code)]
pub struct ChunkedWriter {
    file: tokio::fs::File,
    temp_path: PathBuf,
    final_path: PathBuf,
    bytes_written: u64,
}

#[allow(dead_code)]
impl ChunkedWriter {
    /// Create a writer that stages data in a `.part` file next to the final path.
    /// Use this when there is no dedicated temp directory (e.g. CLI pull).
    pub async fn create(path: &Path) -> Result<Self, std::io::Error> {
        let temp_path = part_path_for(path);
        Self::create_with_temp(temp_path, path.to_path_buf()).await
    }

    /// Create a writer that stages data in an explicit temp file, then renames
    /// to `final_path` on [`finalize`].  Use this when a temp directory like
    /// `.drift/` is available.
    pub async fn create_with_temp(
        temp_path: PathBuf,
        final_path: PathBuf,
    ) -> Result<Self, std::io::Error> {
        if let Some(parent) = temp_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let (file, bytes_written) = if temp_path.exists() {
            let metadata = tokio::fs::metadata(&temp_path).await?;
            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&temp_path)
                .await?;
            (file, metadata.len())
        } else {
            let file = tokio::fs::File::create(&temp_path).await?;
            (file, 0)
        };

        Ok(Self {
            file,
            temp_path,
            final_path,
            bytes_written,
        })
    }

    pub async fn write_chunk(&mut self, data: &[u8]) -> Result<(), std::io::Error> {
        self.file.write_all(data).await?;
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    pub async fn finalize(mut self) -> Result<(), std::io::Error> {
        self.file.flush().await?;
        drop(self.file);
        if self.temp_path != self.final_path {
            tokio::fs::rename(&self.temp_path, &self.final_path).await?;
        }
        Ok(())
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Path to the temp file being written to (before finalize).
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    /// Path the file will be renamed to on finalize.
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// Check how many bytes have already been written for resume support.
    /// Uses the `.part` convention (temp file next to final path).
    pub async fn resume_offset(path: &Path) -> u64 {
        let part_path = part_path_for(path);
        tokio::fs::metadata(&part_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    }
}
