use crate::protocol::messages::FileEntry;
use std::path::Path;

pub fn list_directory(root: &Path, relative: &str) -> Result<Vec<FileEntry>, BrowseError> {
    let target = root.join(relative);
    let canonical = target
        .canonicalize()
        .map_err(|e| BrowseError::Io(e.to_string()))?;

    let root_canonical = root
        .canonicalize()
        .map_err(|e| BrowseError::Io(e.to_string()))?;

    // Path traversal protection
    if !canonical.starts_with(&root_canonical) {
        return Err(BrowseError::PathTraversal);
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&canonical)
        .map_err(|e| BrowseError::Io(e.to_string()))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| BrowseError::Io(e.to_string()))?;

        // Hide .drift temp directory
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".drift" {
            continue;
        }

        let metadata = entry.metadata().map_err(|e| BrowseError::Io(e.to_string()))?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };

        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size: metadata.len(),
            modified,
            #[cfg(unix)]
            permissions,
        });
    }

    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[derive(Debug, thiserror::Error)]
pub enum BrowseError {
    #[error("path traversal attempt blocked")]
    PathTraversal,
    #[error("IO error: {0}")]
    Io(String),
}
