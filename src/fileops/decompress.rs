use std::path::Path;
use flate2::read::GzDecoder;
use tar::Archive;

/// Decompress a .tar.gz archive into root_dir, then delete the archive.
pub fn decompress_archive(archive_path: &Path, root_dir: &Path) -> Result<(), DecompressError> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| DecompressError::Io(format!("Failed to open archive: {}", e)))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    archive.unpack(root_dir)
        .map_err(|e| DecompressError::Io(format!("Failed to extract archive: {}", e)))?;

    tracing::info!("Decompressed {} into {}", archive_path.display(), root_dir.display());

    // Delete the archive
    if let Err(e) = std::fs::remove_file(archive_path) {
        tracing::warn!("Failed to clean up archive: {}", e);
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DecompressError {
    #[error("IO error: {0}")]
    Io(String),
}
