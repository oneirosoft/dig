use std::io;

use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::open_initialized;

use super::types::{OrphanOptions, OrphanPlan};

pub(crate) fn plan(options: &OrphanOptions) -> io::Result<OrphanPlan> {
    let session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let original_branch = git::current_branch_name()?;
    let branch_name = resolve_branch_name(&original_branch, options.branch_name.as_deref())?;

    if branch_name == session.config.trunk_branch {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot orphan trunk branch '{}'",
                session.config.trunk_branch
            ),
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
                format!("branch '{}' is not tracked by dig", branch_name),
            )
        })?;

    let graph = BranchGraph::new(&session.state);
    let parent_branch_name = graph
        .parent_branch_name(&node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent for '{}' is missing from dig",
                branch_name
            ))
        })?;

    if !git::branch_exists(&parent_branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent branch '{}' does not exist", parent_branch_name),
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

    let restack_plan = restack::previews_for_actions(&restack::plan_after_branch_detach(
        &session.state,
        node.id,
        &node.branch_name,
        &parent_branch_name,
        &node.parent,
    )?);

    Ok(OrphanPlan {
        trunk_branch: session.config.trunk_branch,
        original_branch,
        branch_name,
        parent_branch_name,
        node_id: node.id,
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
