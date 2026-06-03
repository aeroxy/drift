use flate2::Compression;
use flate2::write::GzEncoder;
use std::path::{Path, PathBuf};
use tar::Builder;

fn append_dir_all_excluding(
    archive: &mut Builder<GzEncoder<std::fs::File>>,
    base_prefix: &Path,
    source: &Path,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let file_name = entry.file_name();

        if file_name == ".drift" {
            continue;
        }

        let path = entry.path();
        let archive_path = base_prefix.join(&file_name);

        if file_type.is_dir() {
            archive.append_dir(&archive_path, &path)?;
            append_dir_all_excluding(archive, &archive_path, &path)?;
        } else {
            archive.append_path_with_name(&path, &archive_path)?;
        }
    }
    Ok(())
}

/// Compress a directory into a .tar.gz file inside the .drift temp directory.
/// Returns (archive_path, archive_size).
pub fn compress_directory(
    root_dir: &Path,
    relative_path: &str,
) -> Result<(PathBuf, u64), CompressError> {
    let source = root_dir.join(relative_path);
    let source = source
        .canonicalize()
        .map_err(|e| CompressError::Io(format!("Failed to resolve path: {}", e)))?;

    let root_canonical = root_dir
        .canonicalize()
        .map_err(|e| CompressError::Io(format!("Failed to resolve root_dir: {}", e)))?;

    if !source.starts_with(&root_canonical) {
        return Err(CompressError::Io("Path traversal attempt blocked".to_string()));
    }

    if !source.is_dir() {
        return Err(CompressError::NotADirectory);
    }

    // Stage the .drift temp dir next to the source (under the local pane),
    // not under root_dir. The root may be read-only (e.g. drift launched from `/`)
    // even when the source folder is writable. Co-locating .drift with the source
    // also keeps the staging on the same filesystem as the archive it produces.
    let parent = source
        .parent()
        .ok_or_else(|| CompressError::Io("Source has no parent directory".to_string()))?;
    let drift_dir = parent.join(".drift");
    std::fs::create_dir_all(&drift_dir)
        .map_err(|e| CompressError::Io(format!("Failed to create .drift dir: {}", e)))?;

    // Create archive file
    let archive_name = format!("{}.tar.gz", relative_path.replace('/', "_"));
    let archive_path = drift_dir.join(&archive_name);

    let file = std::fs::File::create(&archive_path)
        .map_err(|e| CompressError::Io(format!("Failed to create archive: {}", e)))?;

    let encoder = GzEncoder::new(file, Compression::fast());
    let mut archive = Builder::new(encoder);

    // Add directory contents to archive using only the directory's own name as prefix,
    // not the full relative_path (which may include subdirectory prefixes from the sender).
    let dir_name = source
        .file_name()
        .ok_or_else(|| CompressError::Io("Invalid directory path".to_string()))?;
    
    let base_prefix = Path::new(dir_name);
    archive
        .append_dir(base_prefix, &source)
        .map_err(|e| CompressError::Io(format!("Failed to archive directory: {}", e)))?;
    
    append_dir_all_excluding(&mut archive, base_prefix, &source)
        .map_err(|e| CompressError::Io(format!("Failed to archive directory contents: {}", e)))?;

    // Finalize
    let encoder = archive
        .into_inner()
        .map_err(|e| CompressError::Io(format!("Failed to finalize archive: {}", e)))?;
    encoder
        .finish()
        .map_err(|e| CompressError::Io(format!("Failed to finish compression: {}", e)))?;

    // Get archive size
    let size = std::fs::metadata(&archive_path)
        .map_err(|e| CompressError::Io(format!("Failed to read archive size: {}", e)))?
        .len();

    tracing::info!(
        "Compressed {} -> {} ({} bytes)",
        relative_path,
        archive_path.display(),
        size
    );

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
