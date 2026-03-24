use std::fs::OpenOptions;
use std::io;
use std::io::Write;

use super::fs::{ensure_store_dir, DigPaths};
use super::types::DigEvent;

pub fn append_event(paths: &DigPaths, event: &DigEvent) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.events_file)?;

    serde_json::to_writer(&mut file, event)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writeln!(file)?;

    Ok(())
}

pub fn ensure_events_file(paths: &DigPaths) -> io::Result<bool> {
    ensure_store_dir(paths)?;

    match OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&paths.events_file)
    {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(err),
    }
}
