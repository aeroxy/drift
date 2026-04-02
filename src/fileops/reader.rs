use tokio::io::AsyncReadExt;
use std::path::Path;

#[allow(dead_code)]
pub const CHUNK_SIZE: usize = 64 * 1024; // 64KB

#[allow(dead_code)]
pub struct ChunkedReader {
    file: tokio::fs::File,
    offset: u64,
    total_size: u64,
}

#[allow(dead_code)]
impl ChunkedReader {
    pub async fn open(path: &Path, resume_offset: u64) -> Result<Self, std::io::Error> {
        let mut file = tokio::fs::File::open(path).await?;
        let metadata = file.metadata().await?;
        let total_size = metadata.len();

        if resume_offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(resume_offset)).await?;
        }

        Ok(Self {
            file,
            offset: resume_offset,
            total_size,
        })
    }

    pub async fn read_chunk(&mut self) -> Result<Option<(u64, Vec<u8>)>, std::io::Error> {
        let mut buf = vec![0u8; CHUNK_SIZE];
        let n = self.file.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        let offset = self.offset;
        self.offset += n as u64;
        Ok(Some((offset, buf)))
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }
}
