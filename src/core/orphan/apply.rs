use std::io;

use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::{
    BranchArchiveReason, PendingOperationKind, PendingOperationState, PendingOrphanOperation,
    open_initialized, record_branch_archived,
};
use crate::core::workflow;

use super::types::{OrphanOutcome, OrphanPlan};

pub(crate) fn apply(plan: &OrphanPlan) -> io::Result<OrphanOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "orphan")?;
    workflow::ensure_no_pending_operation(&session.paths, "orphan")?;

    let node = session
        .state
        .find_branch_by_id(plan.node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;

    let parent_branch_name = BranchGraph::new(&session.state)
        .parent_branch_name(&node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent for '{}' is missing from dig",
                plan.branch_name
            ))
        })?;

    if parent_branch_name != plan.parent_branch_name {
        return Err(io::Error::other(format!(
            "tracked parent for '{}' changed while planning orphan",
            plan.branch_name
        )));
    }

    let restack_actions = restack::plan_after_branch_detach(
        &session.state,
        node.id,
        &node.branch_name,
        &parent_branch_name,
        &node.parent,
    )?;
    let restack_outcome = workflow::execute_resumable_restack_operation(
        &mut session,
        PendingOperationKind::Orphan(PendingOrphanOperation {
            original_branch: plan.original_branch.clone(),
            branch_name: plan.branch_name.clone(),
            parent_branch_name: plan.parent_branch_name.clone(),
            node_id: plan.node_id,
        }),
        &restack_actions,
        &mut |_| Ok(()),
    )?;

    if restack_outcome.paused {
        return Ok(OrphanOutcome {
            status: restack_outcome.status,
            branch_name: plan.branch_name.clone(),
            parent_branch_name: plan.parent_branch_name.clone(),
            restacked_branches: restack_outcome.restacked_branches,
            restored_original_branch: None,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    complete_orphan(
        &mut session,
        plan.node_id,
        &plan.branch_name,
        &plan.parent_branch_name,
        &plan.original_branch,
        restack_outcome.restacked_branches,
        restack_outcome.status,
    )
}

pub(crate) fn resume_after_sync(
    pending_operation: PendingOperationState,
    payload: PendingOrphanOperation,
) -> io::Result<OrphanOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;

    if restack_outcome.paused {
        return Ok(OrphanOutcome {
            status: restack_outcome.status,
            branch_name: payload.branch_name,
            parent_branch_name: payload.parent_branch_name,
            restacked_branches: restack_outcome.restacked_branches,
            restored_original_branch: None,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    complete_orphan(
        &mut session,
        payload.node_id,
        &payload.branch_name,
        &payload.parent_branch_name,
        &payload.original_branch,
        restack_outcome.restacked_branches,
        restack_outcome.status,
    )
}

fn complete_orphan(
    session: &mut crate::core::store::StoreSession,
    node_id: uuid::Uuid,
    branch_name: &str,
    parent_branch_name: &str,
    original_branch: &str,
    restacked_branches: Vec<crate::core::restack::RestackPreview>,
    status: std::process::ExitStatus,
) -> io::Result<OrphanOutcome> {
    let node = session
        .state
        .find_branch_by_id(node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;

    record_branch_archived(
        session,
        node.id,
        node.branch_name,
        BranchArchiveReason::Orphaned,
    )?;

    let mut final_status = status;
    let mut restored_original_branch = None;
    let mut failure_output = None;

    if let Some(outcome) = workflow::restore_original_branch_if_needed(original_branch)? {
        if outcome.status.success() {
            restored_original_branch = Some(outcome.restored_branch);
            final_status = outcome.status;
        } else {
            final_status = outcome.status;
            failure_output = Some(format!(
                "orphan completed, but failed to return to '{}'",
                original_branch
            ));
        }
    } else if git::current_branch_name_if_any()?.as_deref() == Some(original_branch) {
        final_status = status;
    }

    Ok(OrphanOutcome {
        status: final_status,
        branch_name: branch_name.to_string(),
        parent_branch_name: parent_branch_name.to_string(),
        restacked_branches,
        restored_original_branch,
        failure_output,
        paused: false,
    })
}
