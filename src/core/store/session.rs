use std::io;

use crate::core::git::{self, RepoContext};

use super::lock::StoreLock;
use super::{
    DaggerConfig, StoreInitialization, dagger_paths, initialize_store, load_config, load_state,
};
use crate::core::store::fs::DaggerPaths;
use crate::core::store::types::DaggerState;

pub struct StoreSession {
    pub repo: RepoContext,
    pub paths: DaggerPaths,
    pub config: DaggerConfig,
    pub state: DaggerState,
    _lock: StoreLock,
}

impl StoreSession {
    /// Build a session from pre-loaded parts, acquiring the lock.
    pub fn from_parts(
        repo: RepoContext,
        paths: DaggerPaths,
        config: DaggerConfig,
        state: DaggerState,
    ) -> io::Result<Self> {
        let lock = StoreLock::acquire(&paths.root)?;
        Ok(Self {
            repo,
            paths,
            config,
            state,
            _lock: lock,
        })
    }
}

pub fn open_initialized(missing_message: &str) -> io::Result<StoreSession> {
    let repo = git::resolve_repo_context()?;
    let paths = dagger_paths(&repo.git_dir);
    let config = load_config(&paths)?.ok_or_else(|| io::Error::other(missing_message))?;
    let lock = StoreLock::acquire(&paths.root)?;
    let state = load_state(&paths)?;

    Ok(StoreSession {
        repo,
        paths,
        config,
        state,
        _lock: lock,
    })
}

pub fn open_or_initialize(trunk_branch: &str) -> io::Result<(StoreSession, StoreInitialization)> {
    let repo = git::resolve_repo_context()?;
    let paths = dagger_paths(&repo.git_dir);
    let store_initialization = initialize_store(&paths, trunk_branch)?;
    let config =
        load_config(&paths)?.ok_or_else(|| io::Error::other("dagger config is missing"))?;
    let lock = StoreLock::acquire(&paths.root)?;
    let state = load_state(&paths)?;

    Ok((
        StoreSession {
            repo,
            paths,
            config,
            state,
            _lock: lock,
        },
        store_initialization,
    ))
}
