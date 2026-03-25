use std::io;
use std::process::ExitStatus;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::git::{self, RebaseProgress};
use crate::core::graph::BranchGraph;
use crate::core::store::ParentRef;
use crate::core::store::types::DigState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestackAction {
    pub node_id: Uuid,
    pub branch_name: String,
    pub old_upstream_branch_name: String,
    pub old_upstream_oid: String,
    pub new_base_branch_name: String,
    pub new_parent: Option<ParentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestackPreview {
    pub branch_name: String,
    pub onto_branch: String,
    pub parent_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentChange {
    pub branch_id: Uuid,
    pub branch_name: String,
    pub old_parent: ParentRef,
    pub new_parent: ParentRef,
    pub old_base_ref: String,
    pub new_base_ref: String,
}

#[derive(Debug)]
pub struct RestackStepOutcome {
    pub status: ExitStatus,
    pub parent_change: Option<ParentChange>,
    pub stderr: String,
}

pub fn plan_after_branch_detach(
    state: &DigState,
    detached_node_id: Uuid,
    detached_branch_name: &str,
    new_parent_branch_name: &str,
    new_parent: &ParentRef,
) -> io::Result<Vec<RestackAction>> {
    let mut actions = Vec::new();
    let graph = BranchGraph::new(state);

    for child_id in graph.active_children_ids(detached_node_id) {
        collect_restack_actions(
            state,
            child_id,
            detached_branch_name,
            new_parent_branch_name,
            Some(new_parent.clone()),
            &mut actions,
        )?;
    }

    Ok(actions)
}

pub fn plan_after_branch_advance(
    state: &DigState,
    advanced_node_id: Uuid,
    advanced_branch_name: &str,
    old_head_oid: &str,
) -> io::Result<Vec<RestackAction>> {
    let mut actions = Vec::new();
    let graph = BranchGraph::new(state);

    for child_id in graph.active_children_ids(advanced_node_id) {
        collect_branch_advance_actions(
            state,
            child_id,
            advanced_branch_name,
            old_head_oid,
            advanced_branch_name,
            &mut actions,
        )?;
    }

    Ok(actions)
}

pub fn previews_for_actions(actions: &[RestackAction]) -> Vec<RestackPreview> {
    actions
        .iter()
        .map(|action| RestackPreview {
            branch_name: action.branch_name.clone(),
            onto_branch: action.new_base_branch_name.clone(),
            parent_changed: action.new_parent.is_some(),
        })
        .collect()
}

pub fn apply_action<F>(
    state: &mut DigState,
    action: &RestackAction,
    on_progress: F,
) -> io::Result<RestackStepOutcome>
where
    F: FnMut(RebaseProgress) -> io::Result<()>,
{
    let result = git::rebase_onto_with_progress(
        &action.new_base_branch_name,
        &action.old_upstream_oid,
        &action.branch_name,
        on_progress,
    )?;
    let status = result.status;

    if !status.success() {
        return Ok(RestackStepOutcome {
            status,
            parent_change: None,
            stderr: result.stderr,
        });
    }

    let parent_change = finalize_action(state, action)?;

    Ok(RestackStepOutcome {
        status,
        parent_change,
        stderr: result.stderr,
    })
}

pub fn finalize_action(
    state: &mut DigState,
    action: &RestackAction,
) -> io::Result<Option<ParentChange>> {
    let Some(new_parent) = &action.new_parent else {
        return Ok(None);
    };

    let (old_parent, old_base_ref) = state.reparent_branch(
        action.node_id,
        new_parent.clone(),
        action.new_base_branch_name.clone(),
    )?;

    Ok(Some(ParentChange {
        branch_id: action.node_id,
        branch_name: action.branch_name.clone(),
        old_parent,
        new_parent: new_parent.clone(),
        old_base_ref,
        new_base_ref: action.new_base_branch_name.clone(),
    }))
}

fn collect_branch_advance_actions(
    state: &DigState,
    node_id: Uuid,
    old_upstream_branch_name: &str,
    old_upstream_oid: &str,
    new_base_branch_name: &str,
    actions: &mut Vec<RestackAction>,
) -> io::Result<()> {
    let node = load_active_branch_node(state, node_id)?;
    let branch_name = node.branch_name.clone();
    actions.push(RestackAction {
        node_id,
        branch_name: branch_name.clone(),
        old_upstream_branch_name: old_upstream_branch_name.to_string(),
        old_upstream_oid: old_upstream_oid.to_string(),
        new_base_branch_name: new_base_branch_name.to_string(),
        new_parent: None,
    });

    for child_id in BranchGraph::new(state).active_children_ids(node_id) {
        let branch_head_oid = git::ref_oid(&branch_name)?;
        collect_branch_advance_actions(
            state,
            child_id,
            &branch_name,
            &branch_head_oid,
            &branch_name,
            actions,
        )?;
    }

    Ok(())
}

fn collect_restack_actions(
    state: &DigState,
    node_id: Uuid,
    old_upstream_branch_name: &str,
    new_base_branch_name: &str,
    new_parent: Option<ParentRef>,
    actions: &mut Vec<RestackAction>,
) -> io::Result<()> {
    let node = load_active_branch_node(state, node_id)?;
    let old_upstream_oid = git::ref_oid(old_upstream_branch_name)?;
    let branch_name = node.branch_name.clone();
    actions.push(RestackAction {
        node_id,
        branch_name: branch_name.clone(),
        old_upstream_branch_name: old_upstream_branch_name.to_string(),
        old_upstream_oid,
        new_base_branch_name: new_base_branch_name.to_string(),
        new_parent,
    });

    for child_id in BranchGraph::new(state).active_children_ids(node_id) {
        collect_restack_actions(state, child_id, &branch_name, &branch_name, None, actions)?;
    }

    Ok(())
}

fn load_active_branch_node(
    state: &DigState,
    node_id: Uuid,
) -> io::Result<crate::core::store::BranchNode> {
    let node = state.find_branch_by_id(node_id).cloned().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "tracked descendant branch was not found",
        )
    })?;

    if !git::branch_exists(&node.branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "tracked descendant branch '{}' no longer exists locally",
                node.branch_name
            ),
        ));
    }

    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::{RestackAction, plan_after_branch_advance, previews_for_actions};
    use crate::core::git;
    use crate::core::store::types::DIG_STATE_VERSION;
    use crate::core::store::{BranchNode, ParentRef};
    use crate::core::test_support::{commit_file, git_ok, initialize_main_repo, with_temp_repo};
    use uuid::Uuid;

    #[test]
    fn builds_restack_previews_from_actions() {
        let previews = previews_for_actions(&[
            RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth-api".into(),
                old_upstream_branch_name: "feat/auth".into(),
                old_upstream_oid: "abc123".into(),
                new_base_branch_name: "main".into(),
                new_parent: Some(ParentRef::Trunk),
            },
            RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth-api-tests".into(),
                old_upstream_branch_name: "feat/auth-api".into(),
                old_upstream_oid: "def456".into(),
                new_base_branch_name: "feat/auth-api".into(),
                new_parent: None,
            },
        ]);

        assert_eq!(previews[0].branch_name, "feat/auth-api");
        assert_eq!(previews[0].onto_branch, "main");
        assert!(previews[0].parent_changed);
        assert_eq!(previews[1].branch_name, "feat/auth-api-tests");
        assert_eq!(previews[1].onto_branch, "feat/auth-api");
        assert!(!previews[1].parent_changed);
    }

    #[test]
    fn plans_restack_after_branch_advance_with_old_head_for_immediate_child() {
        with_temp_repo("dig-restack", |repo| {
            initialize_main_repo(repo);
            git_ok(repo, &["checkout", "-b", "feat/auth"]);
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["checkout", "-b", "feat/auth-api"]);
            commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");
            git_ok(repo, &["checkout", "-b", "feat/auth-api-tests"]);
            commit_file(
                repo,
                "auth-api-tests.txt",
                "tests\n",
                "feat: auth api tests",
            );

            let parent_id = Uuid::new_v4();
            let child_id = Uuid::new_v4();
            let grandchild_id = Uuid::new_v4();
            let state = crate::core::store::types::DigState {
                version: DIG_STATE_VERSION,
                nodes: vec![
                    BranchNode {
                        id: parent_id,
                        branch_name: "feat/auth".into(),
                        parent: ParentRef::Trunk,
                        base_ref: "main".into(),
                        fork_point_oid: "root".into(),
                        head_oid_at_creation: "root".into(),
                        created_at_unix_secs: 1,
                        archived: false,
                    },
                    BranchNode {
                        id: child_id,
                        branch_name: "feat/auth-api".into(),
                        parent: ParentRef::Branch { node_id: parent_id },
                        base_ref: "feat/auth".into(),
                        fork_point_oid: "auth".into(),
                        head_oid_at_creation: "auth".into(),
                        created_at_unix_secs: 2,
                        archived: false,
                    },
                    BranchNode {
                        id: grandchild_id,
                        branch_name: "feat/auth-api-tests".into(),
                        parent: ParentRef::Branch { node_id: child_id },
                        base_ref: "feat/auth-api".into(),
                        fork_point_oid: "api".into(),
                        head_oid_at_creation: "api".into(),
                        created_at_unix_secs: 3,
                        archived: false,
                    },
                ],
            };

            let planned =
                plan_after_branch_advance(&state, parent_id, "feat/auth", "old-parent-head-oid")
                    .unwrap();

            assert_eq!(planned.len(), 2);
            assert_eq!(planned[0].node_id, child_id);
            assert_eq!(planned[0].branch_name, "feat/auth-api");
            assert_eq!(planned[0].old_upstream_branch_name, "feat/auth");
            assert_eq!(planned[0].old_upstream_oid, "old-parent-head-oid");
            assert_eq!(planned[0].new_base_branch_name, "feat/auth");
            assert_eq!(planned[0].new_parent, None);

            assert_eq!(planned[1].node_id, grandchild_id);
            assert_eq!(planned[1].branch_name, "feat/auth-api-tests");
            assert_eq!(planned[1].old_upstream_branch_name, "feat/auth-api");
            assert_eq!(
                planned[1].old_upstream_oid,
                git::ref_oid("feat/auth-api").unwrap()
            );
            assert_eq!(planned[1].new_base_branch_name, "feat/auth-api");
            assert_eq!(planned[1].new_parent, None);
        });
    }
}
