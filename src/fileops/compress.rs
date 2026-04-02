use std::path::{Path, PathBuf};
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;

/// Compress a directory into a .tar.gz file inside the .drift temp directory.
/// Returns (archive_path, archive_size).
pub fn compress_directory(root_dir: &Path, relative_path: &str) -> Result<(PathBuf, u64), CompressError> {
    let source = root_dir.join(relative_path);
    let source = source.canonicalize()
        .map_err(|e| CompressError::Io(format!("Failed to resolve path: {}", e)))?;

    if !source.is_dir() {
        return Err(CompressError::NotADirectory);
    }

    // Create .drift temp directory
    let drift_dir = root_dir.join(".drift");
    std::fs::create_dir_all(&drift_dir)
        .map_err(|e| CompressError::Io(format!("Failed to create .drift dir: {}", e)))?;

    // Create archive file
    let archive_name = format!("{}.tar.gz", relative_path.replace('/', "_"));
    let archive_path = drift_dir.join(&archive_name);

    let file = std::fs::File::create(&archive_path)
        .map_err(|e| CompressError::Io(format!("Failed to create archive: {}", e)))?;

    let encoder = GzEncoder::new(file, Compression::fast());
    let mut archive = Builder::new(encoder);

    // Add directory contents to archive with relative paths
    archive.append_dir_all(relative_path, &source)
        .map_err(|e| CompressError::Io(format!("Failed to archive directory: {}", e)))?;

    // Finalize
    let encoder = archive.into_inner()
        .map_err(|e| CompressError::Io(format!("Failed to finalize archive: {}", e)))?;
    encoder.finish()
        .map_err(|e| CompressError::Io(format!("Failed to finish compression: {}", e)))?;

    // Get archive size
    let size = std::fs::metadata(&archive_path)
        .map_err(|e| CompressError::Io(format!("Failed to read archive size: {}", e)))?
        .len();

    tracing::info!("Compressed {} -> {} ({} bytes)", relative_path, archive_path.display(), size);

    Ok((archive_path, size))
}

/// Clean up a temp archive file
pub fn cleanup_archive(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!("Failed to clean up archive {}: {}", path.display(), e);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompressError {
    #[error("not a directory")]
    NotADirectory,
    #[error("IO error: {0}")]
    Io(String),
}
