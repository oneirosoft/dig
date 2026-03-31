use std::fs;
use std::io;

use super::fs::{DaggerPaths, ensure_store_dir, write_atomic};
use super::types::DaggerConfig;

pub fn load_config(paths: &DaggerPaths) -> io::Result<Option<DaggerConfig>> {
    if !paths.config_file.exists() {
        return Ok(None);
    }

    let bytes = fs::read(&paths.config_file)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let migrated = super::migrate::migrate_config(value)?;
    let config: DaggerConfig = serde_json::from_value(migrated)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(Some(config))
}

pub fn save_config(paths: &DaggerPaths, config: &DaggerConfig) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_atomic(&paths.config_file, &bytes)
}
