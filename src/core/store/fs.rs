use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DaggerPaths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub operation_file: PathBuf,
    pub state_file: PathBuf,
    pub events_file: PathBuf,
}

pub fn dagger_paths(git_dir: &Path) -> DaggerPaths {
    let root = git_dir.join(".dagger");

    DaggerPaths {
        config_file: root.join("config.json"),
        operation_file: root.join("operation.json"),
        state_file: root.join("state.json"),
        events_file: root.join("events.ndjson"),
        root,
    }
}

pub fn ensure_store_dir(paths: &DaggerPaths) -> io::Result<()> {
    fs::create_dir_all(&paths.root)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cannot determine parent directory"))?;

    fs::create_dir_all(parent)?;

    let temp_path = parent.join(format!(".tmp-{}", Uuid::new_v4()));
    fs::write(&temp_path, bytes)?;

    // On Unix, rename(2) atomically overwrites the target file.
    // On Windows, rename fails if the target exists, so we must remove first.
    // The Windows path has a small window where a crash loses the file.
    #[cfg(unix)]
    {
        fs::rename(&temp_path, path)?;
    }

    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
        if let Err(e) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn write_atomic_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new_file.json");

        write_atomic(&path, b"hello world").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn write_atomic_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.json");

        fs::write(&path, b"old content").unwrap();
        write_atomic(&path, b"new content").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn write_atomic_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("file.json");

        write_atomic(&path, b"nested").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
    }
}
