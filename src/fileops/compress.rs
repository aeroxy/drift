use flate2::Compression;
use flate2::write::GzEncoder;
use std::path::{Path, PathBuf};
use tar::Builder;

fn append_dir_all_excluding(
    archive: &mut Builder<GzEncoder<std::fs::File>>,
    base_prefix: &Path,
    source: &Path,
    drift_dir_to_exclude: &Path,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();

        if path == drift_dir_to_exclude {
            continue;
        }

        let file_name = entry.file_name();
        let archive_path = base_prefix.join(&file_name);

        if file_type.is_dir() {
            archive.append_dir(&archive_path, &path)?;
            append_dir_all_excluding(archive, &archive_path, &path, drift_dir_to_exclude)?;
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

    // Stage the .drift temp dir inside the source directory to keep it contained.
    // If the source is read-only (e.g., `/` on macOS), fallback to the OS temp dir.
    let mut drift_dir = source.join(".drift");
    
    // Edge case: if relative_path is empty/root, the name would just be ".tar.gz" or "..tar.gz"
    let mut archive_name = format!("{}.tar.gz", relative_path.replace(['/', '\\'], "_"));
    if archive_name == ".tar.gz" || archive_name == "..tar.gz" {
        archive_name = "root.tar.gz".to_string();
    }
    
    let mut archive_path = drift_dir.join(&archive_name);
    
    // Try to create in source/.drift/ and open the file
    let file = if std::fs::create_dir_all(&drift_dir).is_ok() {
        if let Ok(f) = std::fs::File::create(&archive_path) {
            Some(f)
        } else {
            let _ = std::fs::remove_dir(&drift_dir);
            None
        }
    } else {
        None
    };

    // If we couldn't create/write there, fallback to system temp dir with a unique subdirectory
    let file = if let Some(f) = file {
        f
    } else {
        drift_dir = std::env::temp_dir()
            .join(".drift")
            .join(format!(".drift-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&drift_dir)
            .map_err(|e| CompressError::Io(format!("Failed to create fallback temp dir: {}", e)))?;
        archive_path = drift_dir.join(&archive_name);
        match std::fs::File::create(&archive_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_dir(&drift_dir);
                return Err(CompressError::Io(format!("Failed to create archive: {}", e)));
            }
        }
    };

    struct CleanupGuard {
        path: PathBuf,
        active: bool,
    }
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            if self.active {
                cleanup_archive(&self.path);
            }
        }
    }
    let mut cleanup_guard = CleanupGuard {
        path: archive_path.clone(),
        active: true,
    };

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
    
    append_dir_all_excluding(&mut archive, base_prefix, &source, &source.join(".drift"))
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

    cleanup_guard.active = false;
    Ok((archive_path, size))
}

/// Clean up a temp archive file and its parent .drift directory if empty
pub fn cleanup_archive(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Failed to clean up archive {}: {}", path.display(), e);
        }
    }
    
    // Best-effort cleanup of the parent directory (the staging folder)
    if let Some(parent) = path.parent() {
        let name = parent.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == ".drift" || name.starts_with(".drift-") {
            let _ = std::fs::remove_dir(parent); // Only succeeds if directory is empty
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompressError {
    #[error("not a directory")]
    NotADirectory,
    #[error("IO error: {0}")]
    Io(String),
}
