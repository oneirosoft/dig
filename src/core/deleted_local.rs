use std::io;

use uuid::Uuid;

use crate::core::git;
use crate::core::graph::{BranchGraph, BranchTreeNode};
use crate::core::restack::{self, RestackAction, RestackBaseTarget, RestackPreview};
use crate::core::store::types::DigState;
use crate::core::store::{BranchArchiveReason, ParentRef, StoreSession, record_branch_archived};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeletedLocalBranchStep {
    pub node_id: Uuid,
    pub branch_name: String,
    pub new_parent_base: RestackBaseTarget,
    pub new_parent: ParentRef,
    pub tree: BranchTreeNode,
    pub restack_plan: Vec<RestackPreview>,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy)]
enum DeletedLocalScope {
    All,
    Subtree { root_node_id: Uuid },
}

pub(crate) fn collect_deleted_local_steps(
    state: &DigState,
    trunk_branch: &str,
) -> io::Result<Vec<DeletedLocalBranchStep>> {
    collect_deleted_local_steps_with_scope(state, trunk_branch, DeletedLocalScope::All)
}

pub(crate) fn collect_deleted_local_subtree_steps(
    state: &DigState,
    trunk_branch: &str,
    root_node_id: Uuid,
) -> io::Result<Vec<DeletedLocalBranchStep>> {
    collect_deleted_local_steps_with_scope(
        state,
        trunk_branch,
        DeletedLocalScope::Subtree { root_node_id },
    )
}

pub(crate) fn next_deleted_local_step(
    state: &DigState,
    trunk_branch: &str,
) -> io::Result<Option<DeletedLocalBranchStep>> {
    next_deleted_local_step_with_scope(state, trunk_branch, DeletedLocalScope::All)
}

pub(crate) fn restack_actions_for_step(
    state: &DigState,
    step: &DeletedLocalBranchStep,
) -> io::Result<Vec<RestackAction>> {
    restack::plan_after_deleted_branch(
        state,
        step.node_id,
        &step.branch_name,
        &step.new_parent_base,
        &step.new_parent,
    )
}

pub(crate) fn simulate_deleted_local_step(
    state: &mut DigState,
    step: &DeletedLocalBranchStep,
) -> io::Result<()> {
    for action in restack_actions_for_step(state, step)? {
        let _ = restack::finalize_action(state, &action)?;
    }

    state.archive_branch(step.node_id)
}

pub(crate) fn archive_deleted_local_step(
    session: &mut StoreSession,
    step: &DeletedLocalBranchStep,
) -> io::Result<()> {
    record_branch_archived(
        session,
        step.node_id,
        step.branch_name.clone(),
        BranchArchiveReason::DeletedLocally,
    )
}

fn collect_deleted_local_steps_with_scope(
    state: &DigState,
    trunk_branch: &str,
    scope: DeletedLocalScope,
) -> io::Result<Vec<DeletedLocalBranchStep>> {
    let mut simulated_state = state.clone();
    let mut steps = Vec::new();

    while let Some(step) =
        next_deleted_local_step_with_scope(&simulated_state, trunk_branch, scope)?
    {
        simulate_deleted_local_step(&mut simulated_state, &step)?;
        steps.push(step);
    }

    Ok(steps)
}

fn next_deleted_local_step_with_scope(
    state: &DigState,
    trunk_branch: &str,
    scope: DeletedLocalScope,
) -> io::Result<Option<DeletedLocalBranchStep>> {
    let graph = BranchGraph::new(state);
    let mut missing_nodes = Vec::new();

    for node_id in scoped_node_ids(state, scope) {
        let Some(node) = state.find_branch_by_id(node_id) else {
            continue;
        };

        if !git::branch_exists(&node.branch_name)? {
            missing_nodes.push((
                graph.branch_depth(node.id),
                node.branch_name.clone(),
                node.id,
            ));
        }
    }

    missing_nodes.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let Some((_, _, node_id)) = missing_nodes.into_iter().next() else {
        return Ok(None);
    };

    plan_deleted_local_step(state, trunk_branch, node_id).map(Some)
}

fn scoped_node_ids(state: &DigState, scope: DeletedLocalScope) -> Vec<Uuid> {
    match scope {
        DeletedLocalScope::All => state
            .nodes
            .iter()
            .filter(|node| !node.archived)
            .map(|node| node.id)
            .collect(),
        DeletedLocalScope::Subtree { root_node_id } => {
            let mut node_ids = Vec::new();
            if state.find_branch_by_id(root_node_id).is_some() {
                node_ids.push(root_node_id);
                node_ids.extend(BranchGraph::new(state).active_descendant_ids(root_node_id));
            }
            node_ids
        }
    }
}

fn plan_deleted_local_step(
    state: &DigState,
    trunk_branch: &str,
    node_id: Uuid,
) -> io::Result<DeletedLocalBranchStep> {
    let node = state
        .find_branch_by_id(node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;
    let graph = BranchGraph::new(state);
    let (new_parent_branch_name, new_parent) =
        resolve_replacement_parent(state, trunk_branch, &node.parent)?;
    let restack_plan = restack::previews_for_actions(&restack::plan_after_deleted_branch(
        state,
        node.id,
        &node.branch_name,
        &new_parent_branch_name,
        &new_parent,
    )?);

    Ok(DeletedLocalBranchStep {
        node_id: node.id,
        branch_name: node.branch_name.clone(),
        new_parent_base: new_parent_branch_name,
        new_parent,
        tree: graph.subtree(node.id)?,
        restack_plan,
        depth: graph.branch_depth(node.id),
    })
}

pub(crate) fn plan_deleted_local_step_for_branch(
    state: &DigState,
    trunk_branch: &str,
    branch_name: &str,
) -> io::Result<DeletedLocalBranchStep> {
    let node = state.find_branch_by_name(branch_name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("tracked branch '{}' was not found", branch_name),
        )
    })?;

    plan_deleted_local_step(state, trunk_branch, node.id)
}

pub(crate) fn resolve_replacement_parent(
    state: &DigState,
    trunk_branch: &str,
    parent: &ParentRef,
) -> io::Result<(RestackBaseTarget, ParentRef)> {
    let mut current_parent = parent.clone();

    loop {
        match current_parent {
            ParentRef::Trunk => {
                return Ok((RestackBaseTarget::local(trunk_branch), ParentRef::Trunk));
            }
            ParentRef::Branch { node_id } => {
                let parent_node = state
                    .find_any_branch_by_id(node_id)
                    .ok_or_else(|| io::Error::other("tracked parent branch was not found"))?;

                if !parent_node.archived && git::branch_exists(&parent_node.branch_name)? {
                    return Ok((
                        RestackBaseTarget::local(&parent_node.branch_name),
                        ParentRef::Branch {
                            node_id: parent_node.id,
                        },
                    ));
                }

                current_parent = parent_node.parent.clone();
            }
        }
    }
}
