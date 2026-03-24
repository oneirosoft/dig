use std::io;
use std::process::ExitStatus;

use crate::core::git;
use crate::core::store::{dig_paths, initialize_store, load_config, load_state, StoreInitialization};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitOptions {}

#[derive(Debug)]
pub struct InitOutcome {
    pub status: ExitStatus,
    pub created_git_repo: bool,
    pub lineage: Vec<String>,
    pub store_initialization: StoreInitialization,
}

pub fn run(_: &InitOptions) -> io::Result<InitOutcome> {
    let (status, created_git_repo, repo_context) = match git::try_resolve_repo_context()? {
        Some(repo_context) => (git::probe_repo_status()?, false, repo_context),
        None => {
            let status = git::init_repository()?;
            if !status.success() {
                return Err(io::Error::other("git init failed"));
            }

            let repo_context = git::resolve_repo_context()?;
            (status, true, repo_context)
        }
    };

    let current_branch = git::current_branch_name_or("main")?;
    let store_paths = dig_paths(&repo_context.git_dir);
    let store_initialization = initialize_store(&store_paths, &current_branch)?;
    let config = load_config(&store_paths)?.ok_or_else(|| io::Error::other("dig config is missing"))?;
    let state = load_state(&store_paths)?;

    Ok(InitOutcome {
        status,
        created_git_repo,
        lineage: state.branch_lineage(&current_branch, &config.trunk_branch),
        store_initialization,
    })
}

#[cfg(test)]
mod tests {
    use crate::core::store::StoreInitialization;
    use crate::core::store::types::DigState;
    use std::process::{Command, Stdio};

    #[test]
    fn captures_store_bootstrap_details() {
        let status = Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .unwrap();
        let outcome = super::InitOutcome {
            status,
            created_git_repo: true,
            lineage: DigState::default().branch_lineage("main", "main"),
            store_initialization: StoreInitialization::default(),
        };

        assert!(outcome.created_git_repo);
        assert_eq!(outcome.lineage, vec!["main".to_string()]);
    }
}
