use super::model::{PermissionEditIntent, PermissionEditOperation, SnapshotStatus};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFailure {
    Missing,
    TooLarge,
    NonUtf8,
    NotRegularFile,
    Unreadable,
}

impl ReadFailure {
    pub fn status(self) -> SnapshotStatus {
        match self {
            Self::Missing => SnapshotStatus::SourceMismatch,
            Self::TooLarge => SnapshotStatus::TooLarge,
            Self::NonUtf8 => SnapshotStatus::NonUtf8,
            Self::NotRegularFile => SnapshotStatus::NotRegularFile,
            Self::Unreadable => SnapshotStatus::Unreadable,
        }
    }
}

pub fn operation_paths(intent: &PermissionEditIntent) -> Vec<String> {
    match &intent.operation {
        PermissionEditOperation::TextReplace { path, .. }
        | PermissionEditOperation::WholeFileWrite { path, .. } => vec![path.clone()],
        PermissionEditOperation::PatchSet { files } => {
            let mut paths = Vec::new();
            for file in files {
                if let Some(path) = &file.old_path {
                    paths.push(path.clone());
                }
                if file.old_path.as_deref() != Some(file.new_path.as_str()) {
                    paths.push(file.new_path.clone());
                }
            }
            paths.sort();
            paths.dedup();
            paths
        }
        PermissionEditOperation::Unsupported { .. } => Vec::new(),
    }
}

pub fn protected_paths(intent: &PermissionEditIntent) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        operation_paths(intent)
            .into_iter()
            .filter_map(|path| {
                let resolved = resolve_path(&intent.workspace, &path)?;
                is_macos_protected(&resolved, &home).then(|| resolved.to_string_lossy().to_string())
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = intent;
        Vec::new()
    }
}

pub fn resolve_path(workspace: &str, path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let input = Path::new(path);
    let combined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        Path::new(workspace).join(input)
    };
    Some(normalize_lexically(&combined))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    normalized.components().next_back(),
                    Some(Component::RootDir)
                ) {
                    normalized.pop();
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn is_macos_protected(path: &Path, home: &Path) -> bool {
    let roots = [
        home.join("Desktop"),
        home.join("Documents"),
        home.join("Downloads"),
        home.join("Library/Mobile Documents"),
        home.join("Library/CloudStorage"),
        PathBuf::from("/Volumes"),
    ];
    roots.iter().any(|root| path.starts_with(root))
}

pub fn read_text_limited(
    workspace: &str,
    path: &str,
    protected: &HashSet<PathBuf>,
    total_read: &mut u64,
) -> Result<String, ReadFailure> {
    let resolved = resolve_path(workspace, path).ok_or(ReadFailure::Unreadable)?;
    if protected.contains(&resolved) {
        return Err(ReadFailure::Unreadable);
    }
    let resolved = resolve_parent_symlinks(&resolved)?;
    #[cfg(target_os = "macos")]
    if dirs::home_dir().is_some_and(|home| is_macos_protected(&resolved, &home)) {
        return Err(ReadFailure::Unreadable);
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(&resolved) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReadFailure::Missing)
        }
        Err(_) => return Err(ReadFailure::Unreadable),
    };
    let metadata = file.metadata().map_err(|_| ReadFailure::Unreadable)?;
    if !metadata.file_type().is_file() {
        return Err(ReadFailure::NotRegularFile);
    }
    if metadata.len() > super::MAX_FILE_BYTES
        || total_read.saturating_add(metadata.len()) > super::MAX_TOTAL_READ_BYTES
    {
        return Err(ReadFailure::TooLarge);
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(super::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadFailure::Unreadable)?;
    if bytes.len() as u64 > super::MAX_FILE_BYTES
        || total_read.saturating_add(bytes.len() as u64) > super::MAX_TOTAL_READ_BYTES
    {
        return Err(ReadFailure::TooLarge);
    }
    *total_read = total_read.saturating_add(bytes.len() as u64);
    String::from_utf8(bytes).map_err(|_| ReadFailure::NonUtf8)
}

fn resolve_parent_symlinks(path: &Path) -> Result<PathBuf, ReadFailure> {
    let Some(parent) = path.parent() else {
        return Err(ReadFailure::Unreadable);
    };
    let resolved_parent = resolve_existing_symlinks(parent, 0)?;
    let file_name = path.file_name().ok_or(ReadFailure::Unreadable)?;
    Ok(resolved_parent.join(file_name))
}

fn resolve_existing_symlinks(path: &Path, depth: usize) -> Result<PathBuf, ReadFailure> {
    if depth > 16 {
        return Err(ReadFailure::Unreadable);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        let candidate = current.join(component.as_os_str());
        let metadata =
            std::fs::symlink_metadata(&candidate).map_err(|_| ReadFailure::Unreadable)?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&candidate).map_err(|_| ReadFailure::Unreadable)?;
            let target = if target.is_absolute() {
                target
            } else {
                candidate
                    .parent()
                    .ok_or(ReadFailure::Unreadable)?
                    .join(target)
            };
            current = resolve_existing_symlinks(&normalize_lexically(&target), depth + 1)?;
        } else {
            current = candidate;
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn protected_prefixes_use_path_components() {
        let home = Path::new("/Users/test");
        assert!(is_macos_protected(
            Path::new("/Users/test/Documents/a.txt"),
            home
        ));
        assert!(!is_macos_protected(
            Path::new("/Users/test/DocumentsBackup/a.txt"),
            home
        ));
        assert!(is_macos_protected(Path::new("/Volumes/Disk/a"), home));
    }

    #[test]
    fn resolves_relative_paths_without_touching_disk() {
        assert_eq!(
            resolve_path("/tmp/work", "../other/a.txt").unwrap(),
            PathBuf::from("/tmp/other/a.txt")
        );
    }

    #[test]
    fn reads_utf8_and_rejects_non_utf8() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("bad.txt"), [0xff, 0xfe]).unwrap();
        let protected = HashSet::new();
        let mut total = 0;
        assert_eq!(
            read_text_limited(
                dir.path().to_str().unwrap(),
                "ok.txt",
                &protected,
                &mut total
            )
            .unwrap(),
            "hello"
        );
        assert_eq!(
            read_text_limited(
                dir.path().to_str().unwrap(),
                "bad.txt",
                &protected,
                &mut total
            )
            .unwrap_err(),
            ReadFailure::NonUtf8
        );
    }
}
