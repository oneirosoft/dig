use std::fs;
use std::io;

use super::fs::{ensure_store_dir, write_atomic, DigPaths};
use super::types::DigState;

pub fn load_state(paths: &DigPaths) -> io::Result<DigState> {
    if !paths.state_file.exists() {
        return Ok(DigState::default());
    }

    let bytes = fs::read(&paths.state_file)?;
    let state =
        serde_json::from_slice(&bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(state)
}

pub fn save_state(paths: &DigPaths, state: &DigState) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let bytes =
        serde_json::to_vec_pretty(state).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_atomic(&paths.state_file, &bytes)
}
