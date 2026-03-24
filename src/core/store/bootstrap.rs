use std::io;

use super::config::save_config;
use super::events::ensure_events_file;
use super::fs::DigPaths;
use super::state::save_state;
use super::types::{DigConfig, DigState};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreInitialization {
    pub created_config: bool,
    pub created_state: bool,
    pub created_events: bool,
}

impl StoreInitialization {
    pub fn created_anything(&self) -> bool {
        self.created_config || self.created_state || self.created_events
    }
}

pub fn initialize_store(paths: &DigPaths, trunk_branch: &str) -> io::Result<StoreInitialization> {
    let mut initialization = StoreInitialization::default();

    if !paths.config_file.exists() {
        save_config(paths, &DigConfig::new(trunk_branch.to_string()))?;
        initialization.created_config = true;
    }

    if !paths.state_file.exists() {
        save_state(paths, &DigState::default())?;
        initialization.created_state = true;
    }

    initialization.created_events = ensure_events_file(paths)?;

    Ok(initialization)
}

#[cfg(test)]
mod tests {
    use super::StoreInitialization;

    #[test]
    fn reports_when_bootstrap_created_files() {
        let initialization = StoreInitialization {
            created_config: true,
            created_state: false,
            created_events: false,
        };

        assert!(initialization.created_anything());
    }
}
