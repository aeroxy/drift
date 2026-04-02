use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[allow(dead_code)]
pub struct ChunkedWriter {
    file: tokio::fs::File,
    part_path: PathBuf,
    final_path: PathBuf,
    bytes_written: u64,
}

#[allow(dead_code)]
impl ChunkedWriter {
    pub async fn create(path: &Path) -> Result<Self, std::io::Error> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let part_path = path.with_extension(
            format!(
                "{}.part",
                path.extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
        );

        let (file, bytes_written) = if part_path.exists() {
            let metadata = tokio::fs::metadata(&part_path).await?;
            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part_path)
                .await?;
            (file, metadata.len())
        } else {
            let file = tokio::fs::File::create(&part_path).await?;
            (file, 0)
        };

        Ok(Self {
            file,
            part_path,
            final_path: path.to_path_buf(),
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
        tokio::fs::rename(&self.part_path, &self.final_path).await?;
        Ok(())
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Check how many bytes have already been written for resume support
    pub async fn resume_offset(path: &Path) -> u64 {
        let part_path = path.with_extension(
            format!(
                "{}.part",
                path.extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
        );
        tokio::fs::metadata(&part_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    }
}
