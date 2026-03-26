use std::io;
use std::process::ExitStatus;

use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::graph::BranchLineageNode;
use crate::core::store::{StoreInitialization, open_or_initialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitOptions {}

#[derive(Debug)]
pub struct InitOutcome {
    pub status: ExitStatus,
    pub created_git_repo: bool,
    pub lineage: Vec<BranchLineageNode>,
    pub store_initialization: StoreInitialization,
}

pub fn run(_: &InitOptions) -> io::Result<InitOutcome> {
    let (status, created_git_repo) = match git::try_resolve_repo_context()? {
        Some(_) => (git::probe_repo_status()?, false),
        None => {
            let status = git::init_repository()?;
            if !status.success() {
                return Err(io::Error::other("git init failed"));
            }

            (status, true)
        }
    };

    let current_branch = git::current_branch_name_or("main")?;
    let (session, store_initialization) = open_or_initialize(&current_branch)?;
    let graph = BranchGraph::new(&session.state);

    Ok(InitOutcome {
        status,
        created_git_repo,
        lineage: graph.lineage(&current_branch, &session.config.trunk_branch),
        store_initialization,
    })
}

#[cfg(test)]
mod tests {
    use crate::core::graph::{BranchGraph, BranchLineageNode};
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
            lineage: BranchGraph::new(&DigState::default()).lineage("main", "main"),
            store_initialization: StoreInitialization::default(),
        };

        assert!(outcome.created_git_repo);
        assert_eq!(
            outcome.lineage,
            vec![BranchLineageNode {
                branch_name: "main".to_string(),
                pull_request_number: None,
            }]
        );
    }
}
