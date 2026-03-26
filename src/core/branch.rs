use std::io;
use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::graph::BranchLineageNode;
use crate::core::store::types::DigState;
use crate::core::store::{
    BranchNode, DigConfig, ParentRef, now_unix_timestamp_secs, open_or_initialize,
    record_branch_created,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchOptions {
    pub name: String,
    pub parent_branch_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BranchOutcome {
    pub status: ExitStatus,
    pub created_node: Option<BranchNode>,
    pub lineage: Vec<BranchLineageNode>,
}

pub fn run(options: &BranchOptions) -> io::Result<BranchOutcome> {
    let branch_name = options.name.trim();
    if branch_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch name cannot be empty",
        ));
    }

    if git::branch_exists(branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("branch '{branch_name}' already exists"),
        ));
    }

    let current_branch = git::current_branch_name()?;
    let (mut session, _) = open_or_initialize(&current_branch)?;

    if session.state.find_branch_by_name(branch_name).is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("branch '{branch_name}' is already tracked by dig"),
        ));
    }

    let parent_branch_name =
        resolve_parent_branch_name(&current_branch, options.parent_branch_name.as_deref())?;

    if parent_branch_name == branch_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch cannot list itself as its parent",
        ));
    }

    if !git::branch_exists(&parent_branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent branch '{}' does not exist", parent_branch_name),
        ));
    }

    let parent = resolve_parent_ref(&session.state, &session.config, &parent_branch_name)?;
    let parent_head_oid = git::ref_oid(&parent_branch_name)?;

    let created_node = BranchNode {
        id: Uuid::new_v4(),
        branch_name: branch_name.to_string(),
        parent,
        base_ref: parent_branch_name.clone(),
        fork_point_oid: parent_head_oid.clone(),
        head_oid_at_creation: parent_head_oid,
        created_at_unix_secs: now_unix_timestamp_secs(),
        pull_request: None,
        archived: false,
    };

    let status = git::create_and_checkout_branch(branch_name, &parent_branch_name)?;

    if !status.success() {
        return Ok(BranchOutcome {
            status,
            created_node: None,
            lineage: vec![BranchLineageNode {
                branch_name: branch_name.to_string(),
                pull_request_number: None,
            }],
        });
    }

    record_branch_created(&mut session, created_node.clone())?;
    let graph = BranchGraph::new(&session.state);

    Ok(BranchOutcome {
        status,
        created_node: Some(created_node),
        lineage: graph.lineage(branch_name, &session.config.trunk_branch),
    })
}

fn resolve_parent_branch_name(
    current_branch: &str,
    requested_parent_branch: Option<&str>,
) -> io::Result<String> {
    let parent_branch_name = requested_parent_branch.unwrap_or(current_branch).trim();

    if parent_branch_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "parent branch name cannot be empty",
        ));
    }

    Ok(parent_branch_name.to_string())
}

pub(crate) fn resolve_parent_ref(
    state: &DigState,
    config: &DigConfig,
    parent_branch_name: &str,
) -> io::Result<ParentRef> {
    if parent_branch_name == config.trunk_branch {
        return Ok(ParentRef::Trunk);
    }

    state
        .find_branch_by_name(parent_branch_name)
        .map(|node| ParentRef::Branch { node_id: node.id })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "parent branch '{}' is not tracked by dig and does not match trunk '{}'",
                    parent_branch_name, config.trunk_branch
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{BranchOptions, resolve_parent_branch_name, resolve_parent_ref};
    use crate::core::store::types::DigState;
    use crate::core::store::{BranchNode, DigConfig, ParentRef};
    use uuid::Uuid;

    #[test]
    fn preserves_requested_branch_name() {
        let options = BranchOptions {
            name: "feature/api".into(),
            parent_branch_name: None,
        };

        assert_eq!(options.name, "feature/api");
    }

    #[test]
    fn resolves_requested_trunk_parent() {
        let state = DigState::default();
        let config = DigConfig::new("main".into());

        assert_eq!(
            resolve_parent_ref(&state, &config, "main").unwrap(),
            ParentRef::Trunk
        );
    }

    #[test]
    fn resolves_requested_tracked_parent_branch() {
        let parent_id = Uuid::new_v4();
        let state = DigState {
            version: crate::core::store::types::DIG_STATE_VERSION,
            nodes: vec![BranchNode {
                id: parent_id,
                branch_name: "feature/base".into(),
                parent: ParentRef::Trunk,
                base_ref: "main".into(),
                fork_point_oid: "abc123".into(),
                head_oid_at_creation: "abc123".into(),
                created_at_unix_secs: 1,
                pull_request: None,
                archived: false,
            }],
        };
        let config = DigConfig::new("main".into());

        assert_eq!(
            resolve_parent_ref(&state, &config, "feature/base").unwrap(),
            ParentRef::Branch { node_id: parent_id }
        );
    }

    #[test]
    fn resolves_parent_branch_name_override() {
        assert_eq!(
            resolve_parent_branch_name("feature/base", Some("main")).unwrap(),
            "main"
        );
    }
}
