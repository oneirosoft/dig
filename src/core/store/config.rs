use std::fs;
use std::io;

use super::fs::{ensure_store_dir, write_atomic, DigPaths};
use super::types::DigConfig;

pub fn load_config(paths: &DigPaths) -> io::Result<Option<DigConfig>> {
    if !paths.config_file.exists() {
        return Ok(None);
    }

    let bytes = fs::read(&paths.config_file)?;
    let config =
        serde_json::from_slice(&bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(Some(config))
}

pub fn save_config(paths: &DigPaths, config: &DigConfig) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let bytes =
        serde_json::to_vec_pretty(config).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_atomic(&paths.config_file, &bytes)
}
