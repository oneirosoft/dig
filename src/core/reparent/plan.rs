use std::io;

use crate::core::branch;
use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::{ParentRef, open_initialized};
use crate::core::workflow;

use super::types::{ReparentOptions, ReparentPlan};

pub(crate) fn plan(options: &ReparentOptions) -> io::Result<ReparentPlan> {
    workflow::ensure_no_pending_operation_for_command("reparent")?;
    let session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    let original_branch = git::current_branch_name()?;
    let branch_name = resolve_branch_name(&original_branch, options.branch_name.as_deref())?;
    let parent_branch_name = options.parent_branch_name.trim();

    if parent_branch_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "parent branch name cannot be empty",
        ));
    }

    if branch_name == session.config.trunk_branch {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot reparent trunk branch '{}'",
                session.config.trunk_branch
            ),
        ));
    }

    if branch_name == parent_branch_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch cannot list itself as its parent",
        ));
    }

    if !git::branch_exists(&branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("branch '{}' does not exist", branch_name),
        ));
    }

    let node = session
        .state
        .find_branch_by_name(&branch_name)
        .cloned()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("branch '{}' is not tracked by dagger", branch_name),
            )
        })?;

    if !git::branch_exists(parent_branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent branch '{}' does not exist", parent_branch_name),
        ));
    }

    let graph = BranchGraph::new(&session.state);
    let current_parent_branch_name = graph
        .parent_branch_name(&node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent for '{}' is missing from dagger",
                branch_name
            ))
        })?;

    if !git::branch_exists(&current_parent_branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "current parent branch '{}' does not exist",
                current_parent_branch_name
            ),
        ));
    }

    if current_parent_branch_name == parent_branch_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "branch '{}' is already parented to '{}'",
                branch_name, parent_branch_name
            ),
        ));
    }

    let missing_descendants = graph.missing_local_descendants(node.id)?;
    if !missing_descendants.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "tracked descendants of '{}' are missing locally: {}",
                branch_name,
                missing_descendants.join(", ")
            ),
        ));
    }

    let new_parent =
        branch::resolve_parent_ref(&session.state, &session.config, parent_branch_name)?;

    if let ParentRef::Branch { node_id: parent_id } = new_parent
        && graph.active_descendant_ids(node.id).contains(&parent_id)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot reparent '{}' onto descendant '{}'",
                branch_name, parent_branch_name
            ),
        ));
    }

    let restack_plan = restack::previews_for_actions(&restack::plan_after_branch_reparent(
        &session.state,
        node.id,
        &node.branch_name,
        &current_parent_branch_name,
        &restack::RestackBaseTarget::local(parent_branch_name),
        &new_parent,
    )?);

    Ok(ReparentPlan {
        original_branch,
        branch_name,
        current_parent_branch_name,
        parent_branch_name: parent_branch_name.to_string(),
        node_id: node.id,
        current_parent: node.parent,
        new_parent,
        restack_plan,
    })
}

fn resolve_branch_name(
    original_branch: &str,
    requested_branch_name: Option<&str>,
) -> io::Result<String> {
    let branch_name = requested_branch_name.unwrap_or(original_branch).trim();

    if branch_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch name cannot be empty",
        ));
    }

    Ok(branch_name.to_string())
}
