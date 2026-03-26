use std::io;

use crate::core::branch;
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::{
    PendingOperationKind, PendingOperationState, PendingReparentOperation, open_initialized,
};
use crate::core::workflow;

use super::types::{ReparentOutcome, ReparentPlan};

pub(crate) fn apply(plan: &ReparentPlan) -> io::Result<ReparentOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "reparent")?;
    workflow::ensure_no_pending_operation(&session.paths, "reparent")?;

    let node = session
        .state
        .find_branch_by_id(plan.node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;

    let current_parent_branch_name = BranchGraph::new(&session.state)
        .parent_branch_name(&node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent for '{}' is missing from dig",
                plan.branch_name
            ))
        })?;

    if current_parent_branch_name != plan.current_parent_branch_name
        || node.parent != plan.current_parent
    {
        return Err(io::Error::other(format!(
            "tracked parent for '{}' changed while planning reparent",
            plan.branch_name
        )));
    }

    let new_parent =
        branch::resolve_parent_ref(&session.state, &session.config, &plan.parent_branch_name)?;
    if new_parent != plan.new_parent {
        return Err(io::Error::other(format!(
            "tracked parent for '{}' changed while planning reparent",
            plan.parent_branch_name
        )));
    }

    let restack_actions = restack::plan_after_branch_reparent(
        &session.state,
        node.id,
        &node.branch_name,
        &current_parent_branch_name,
        &plan.parent_branch_name,
        &plan.new_parent,
    )?;
    let restack_outcome = workflow::execute_resumable_restack_operation(
        &mut session,
        PendingOperationKind::Reparent(PendingReparentOperation {
            original_branch: plan.original_branch.clone(),
            branch_name: plan.branch_name.clone(),
            parent_branch_name: plan.parent_branch_name.clone(),
        }),
        &restack_actions,
        &mut |_| Ok(()),
    )?;

    if restack_outcome.paused {
        return Ok(ReparentOutcome {
            status: restack_outcome.status,
            branch_name: plan.branch_name.clone(),
            parent_branch_name: plan.parent_branch_name.clone(),
            restacked_branches: restack_outcome.restacked_branches,
            restored_original_branch: None,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    let restored_original_branch = restore_original_branch_if_needed(&plan.original_branch)?;

    Ok(ReparentOutcome {
        status: restack_outcome.status,
        branch_name: plan.branch_name.clone(),
        parent_branch_name: plan.parent_branch_name.clone(),
        restacked_branches: restack_outcome.restacked_branches,
        restored_original_branch,
        failure_output: None,
        paused: false,
    })
}

pub(crate) fn resume_after_sync(
    pending_operation: PendingOperationState,
    payload: PendingReparentOperation,
) -> io::Result<ReparentOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;

    if restack_outcome.paused {
        return Ok(ReparentOutcome {
            status: restack_outcome.status,
            branch_name: payload.branch_name,
            parent_branch_name: payload.parent_branch_name,
            restacked_branches: restack_outcome.restacked_branches,
            restored_original_branch: None,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    let restored_original_branch = restore_original_branch_if_needed(&payload.original_branch)?;

    Ok(ReparentOutcome {
        status: restack_outcome.status,
        branch_name: payload.branch_name,
        parent_branch_name: payload.parent_branch_name,
        restacked_branches: restack_outcome.restacked_branches,
        restored_original_branch,
        failure_output: None,
        paused: false,
    })
}

fn restore_original_branch_if_needed(original_branch: &str) -> io::Result<Option<String>> {
    if let Some(outcome) = workflow::restore_original_branch_if_needed(original_branch)? {
        if !outcome.status.success() {
            return Err(io::Error::other(format!(
                "reparent completed, but failed to return to '{}'",
                original_branch
            )));
        }

        return Ok(Some(outcome.restored_branch));
    }

    Ok(None)
}
