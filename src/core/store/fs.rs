use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DigPaths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub state_file: PathBuf,
    pub events_file: PathBuf,
}

pub fn dig_paths(git_dir: &Path) -> DigPaths {
    let root = git_dir.join("dig");

    DigPaths {
        config_file: root.join("config.json"),
        state_file: root.join("state.json"),
        events_file: root.join("events.ndjson"),
        root,
    }
}

pub fn ensure_store_dir(paths: &DigPaths) -> io::Result<()> {
    fs::create_dir_all(&paths.root)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cannot determine parent directory"))?;

    fs::create_dir_all(parent)?;

    let temp_path = parent.join(format!(".tmp-{}", Uuid::new_v4()));
    fs::write(&temp_path, bytes)?;

    if path.exists() {
        // TODO: Replace this with a fully atomic cross-platform strategy if Windows overwrite behavior matters.
        fs::remove_file(path)?;
    }

    fs::rename(temp_path, path)
}
