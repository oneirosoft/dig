use std::fs;
use std::io;

use super::fs::{DaggerPaths, ensure_store_dir, write_atomic};
use super::types::DaggerState;

pub fn load_state(paths: &DaggerPaths) -> io::Result<DaggerState> {
    if !paths.state_file.exists() {
        return Ok(DaggerState::default());
    }

    let bytes = fs::read(&paths.state_file)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let migrated = super::migrate::migrate_state(value)?;
    let state: DaggerState = serde_json::from_value(migrated)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(state)
}

pub fn save_state(paths: &DaggerPaths, state: &DaggerState) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_atomic(&paths.state_file, &bytes)
}
