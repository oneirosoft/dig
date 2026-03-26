use std::io;

use crate::core::deleted_local;
use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::{
    BranchArchiveReason, PendingCleanCandidate, PendingCleanCandidateKind, PendingCleanOperation,
    PendingOperationKind, PendingOperationState, open_initialized, record_branch_archived,
};
use crate::core::workflow::{self, RestackExecutionEvent};

use super::types::{CleanApplyOutcome, CleanCandidate, CleanEvent, CleanPlan, CleanReason};

pub(crate) fn apply(plan: &CleanPlan) -> io::Result<CleanApplyOutcome> {
    apply_with_reporter(plan, &mut |_| Ok(()))
}

pub(crate) fn apply_with_reporter<F>(
    plan: &CleanPlan,
    reporter: &mut F,
) -> io::Result<CleanApplyOutcome>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    if plan.candidates.is_empty() {
        return Ok(CleanApplyOutcome {
            status: git::success_status()?,
            switched_to_trunk_from: None,
            restored_original_branch: None,
            untracked_branches: Vec::new(),
            deleted_branches: Vec::new(),
            restacked_branches: Vec::new(),
            failure_output: None,
            paused: false,
        });
    }

    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "clean")?;
    workflow::ensure_no_pending_operation(&session.paths, "clean")?;
    let current_branch = git::current_branch_name()?;
    let original_branch = current_branch.clone();

    let mut switched_to_trunk_from = None;
    if plan.targets_current_branch() && current_branch != session.config.trunk_branch {
        reporter(CleanEvent::SwitchingToTrunk {
            from_branch: current_branch.clone(),
            to_branch: session.config.trunk_branch.clone(),
        })?;
        let checkout = workflow::checkout_branch_if_needed(&session.config.trunk_branch)?;
        if !checkout.status.success() {
            return Ok(CleanApplyOutcome {
                status: checkout.status,
                switched_to_trunk_from: None,
                restored_original_branch: None,
                untracked_branches: Vec::new(),
                deleted_branches: Vec::new(),
                restacked_branches: Vec::new(),
                failure_output: None,
                paused: false,
            });
        }

        reporter(CleanEvent::SwitchedToTrunk {
            from_branch: current_branch.clone(),
            to_branch: session.config.trunk_branch.clone(),
        })?;
        switched_to_trunk_from = checkout.switched_from;
    }

    let mut untracked_branches = Vec::new();
    let mut deleted_branches = Vec::new();
    let mut restacked_branches = Vec::new();
    let mut last_status = git::success_status()?;
    for index in 0..plan.candidates.len() {
        let candidate = pending_clean_candidate_from_clean_candidate(&plan.candidates[index]);
        if let Some(outcome) = process_clean_candidate(
            &mut session,
            &original_branch,
            switched_to_trunk_from.clone(),
            candidate,
            plan.candidates[index + 1..]
                .iter()
                .map(pending_clean_candidate_from_clean_candidate)
                .collect(),
            &mut untracked_branches,
            &mut deleted_branches,
            &mut restacked_branches,
            reporter,
        )? {
            return Ok(outcome);
        }

        last_status = git::success_status()?;
    }

    let checkout = workflow::checkout_branch_if_needed(&session.config.trunk_branch)?;
    if checkout.switched_from.is_some() {
        if !checkout.status.success() {
            return Ok(CleanApplyOutcome {
                status: checkout.status,
                switched_to_trunk_from,
                restored_original_branch: None,
                untracked_branches,
                deleted_branches,
                restacked_branches,
                failure_output: None,
                paused: false,
            });
        }

        last_status = checkout.status;
    }

    let mut restored_original_branch = None;
    if let Some(outcome) = workflow::restore_original_branch_if_needed(&original_branch)? {
        if !outcome.status.success() {
            return Ok(CleanApplyOutcome {
                status: outcome.status,
                switched_to_trunk_from,
                restored_original_branch: None,
                untracked_branches,
                deleted_branches,
                restacked_branches,
                failure_output: None,
                paused: false,
            });
        }

        restored_original_branch = Some(outcome.restored_branch);
        last_status = outcome.status;
    } else if original_branch == session.config.trunk_branch {
        restored_original_branch = checkout
            .switched_from
            .as_ref()
            .map(|_| original_branch.clone());
    }

    Ok(CleanApplyOutcome {
        status: last_status,
        switched_to_trunk_from,
        restored_original_branch,
        untracked_branches,
        deleted_branches,
        restacked_branches,
        failure_output: None,
        paused: false,
    })
}

pub(crate) fn resume_after_sync_with_reporter<F>(
    pending_operation: PendingOperationState,
    payload: PendingCleanOperation,
    reporter: &mut F,
) -> io::Result<CleanApplyOutcome>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    let mut untracked_branches = payload.untracked_branches;
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let mut deleted_branches = payload.deleted_branches;
    let mut restacked_branches = payload.restacked_branches;

    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |event| match event {
            RestackExecutionEvent::Started(action) => reporter(CleanEvent::RebaseStarted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }),
            RestackExecutionEvent::Progress { action, progress } => {
                reporter(CleanEvent::RebaseProgress {
                    branch_name: action.branch_name.clone(),
                    onto_branch: action.new_base.branch_name.clone(),
                    current_commit: progress.current,
                    total_commits: progress.total,
                })
            }
            RestackExecutionEvent::Completed(action) => reporter(CleanEvent::RebaseCompleted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }),
        },
    )?;
    restacked_branches.extend(restack_outcome.restacked_branches.clone());

    if restack_outcome.paused {
        return Ok(CleanApplyOutcome {
            status: restack_outcome.status,
            switched_to_trunk_from: payload.switched_to_trunk_from,
            restored_original_branch: None,
            untracked_branches,
            deleted_branches,
            restacked_branches,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    let completion_status = complete_clean_candidate(
        &mut session,
        &payload.current_candidate,
        &mut untracked_branches,
        &mut deleted_branches,
        reporter,
    )?;
    if !completion_status.success() {
        return Ok(CleanApplyOutcome {
            status: completion_status,
            switched_to_trunk_from: payload.switched_to_trunk_from,
            restored_original_branch: None,
            untracked_branches,
            deleted_branches,
            restacked_branches,
            failure_output: None,
            paused: false,
        });
    }

    for index in 0..payload.remaining_candidates.len() {
        let candidate = payload.remaining_candidates[index].clone();
        let remaining_candidates = payload.remaining_candidates[index + 1..].to_vec();

        if let Some(outcome) = process_clean_candidate(
            &mut session,
            &payload.original_branch,
            payload.switched_to_trunk_from.clone(),
            candidate,
            remaining_candidates,
            &mut untracked_branches,
            &mut deleted_branches,
            &mut restacked_branches,
            reporter,
        )? {
            return Ok(outcome);
        }
    }

    let mut restored_original_branch = None;
    let mut status = restack_outcome.status;
    let checkout = workflow::checkout_branch_if_needed(&payload.trunk_branch)?;
    if checkout.switched_from.is_some() {
        if !checkout.status.success() {
            return Ok(CleanApplyOutcome {
                status: checkout.status,
                switched_to_trunk_from: payload.switched_to_trunk_from,
                restored_original_branch: None,
                untracked_branches,
                deleted_branches,
                restacked_branches,
                failure_output: None,
                paused: false,
            });
        }

        status = checkout.status;
    }

    if let Some(outcome) = workflow::restore_original_branch_if_needed(&payload.original_branch)? {
        if !outcome.status.success() {
            return Ok(CleanApplyOutcome {
                status: outcome.status,
                switched_to_trunk_from: payload.switched_to_trunk_from,
                restored_original_branch: None,
                untracked_branches,
                deleted_branches,
                restacked_branches,
                failure_output: None,
                paused: false,
            });
        }

        restored_original_branch = Some(outcome.restored_branch);
        status = outcome.status;
    } else if payload.original_branch == payload.trunk_branch {
        restored_original_branch = checkout
            .switched_from
            .as_ref()
            .map(|_| payload.original_branch.clone());
    }

    Ok(CleanApplyOutcome {
        status,
        switched_to_trunk_from: payload.switched_to_trunk_from,
        restored_original_branch,
        untracked_branches,
        deleted_branches,
        restacked_branches,
        failure_output: None,
        paused: false,
    })
}

fn process_clean_candidate<F>(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    switched_to_trunk_from: Option<String>,
    candidate: PendingCleanCandidate,
    remaining_candidates: Vec<PendingCleanCandidate>,
    untracked_branches: &mut Vec<String>,
    deleted_branches: &mut Vec<String>,
    restacked_branches: &mut Vec<crate::core::restack::RestackPreview>,
    reporter: &mut F,
) -> io::Result<Option<CleanApplyOutcome>>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    let node = session
        .state
        .find_branch_by_name(&candidate.branch_name)
        .cloned()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("tracked branch '{}' was not found", candidate.branch_name),
            )
        })?;

    let restack_actions = match candidate.kind {
        PendingCleanCandidateKind::DeletedLocally => deleted_local::restack_actions_for_step(
            &session.state,
            &deleted_local::plan_deleted_local_step_for_branch(
                &session.state,
                &session.config.trunk_branch,
                &node.branch_name,
            )?,
        )?,
        PendingCleanCandidateKind::IntegratedIntoParent { ref parent_base } => {
            restack::plan_after_branch_detach(
                &session.state,
                node.id,
                &node.branch_name,
                parent_base,
                &node.parent,
            )?
        }
    };
    let restack_outcome = workflow::execute_resumable_restack_operation(
        session,
        PendingOperationKind::Clean(PendingCleanOperation {
            trunk_branch: session.config.trunk_branch.clone(),
            original_branch: original_branch.to_string(),
            switched_to_trunk_from: switched_to_trunk_from.clone(),
            current_candidate: candidate.clone(),
            remaining_candidates,
            untracked_branches: untracked_branches.clone(),
            deleted_branches: deleted_branches.clone(),
            restacked_branches: restacked_branches.clone(),
        }),
        &restack_actions,
        &mut |event| match event {
            RestackExecutionEvent::Started(action) => reporter(CleanEvent::RebaseStarted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }),
            RestackExecutionEvent::Progress { action, progress } => {
                reporter(CleanEvent::RebaseProgress {
                    branch_name: action.branch_name.clone(),
                    onto_branch: action.new_base.branch_name.clone(),
                    current_commit: progress.current,
                    total_commits: progress.total,
                })
            }
            RestackExecutionEvent::Completed(action) => reporter(CleanEvent::RebaseCompleted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }),
        },
    )?;

    restacked_branches.extend(restack_outcome.restacked_branches.clone());
    if restack_outcome.paused {
        return Ok(Some(CleanApplyOutcome {
            status: restack_outcome.status,
            switched_to_trunk_from,
            restored_original_branch: None,
            untracked_branches: untracked_branches.clone(),
            deleted_branches: deleted_branches.clone(),
            restacked_branches: restacked_branches.clone(),
            failure_output: restack_outcome.failure_output,
            paused: true,
        }));
    }

    let completion_status = complete_clean_candidate(
        session,
        &candidate,
        untracked_branches,
        deleted_branches,
        reporter,
    )?;
    if !completion_status.success() {
        return Ok(Some(CleanApplyOutcome {
            status: completion_status,
            switched_to_trunk_from,
            restored_original_branch: None,
            untracked_branches: untracked_branches.clone(),
            deleted_branches: deleted_branches.clone(),
            restacked_branches: restacked_branches.clone(),
            failure_output: None,
            paused: false,
        }));
    }

    Ok(None)
}

fn complete_clean_candidate<F>(
    session: &mut crate::core::store::StoreSession,
    candidate: &PendingCleanCandidate,
    untracked_branches: &mut Vec<String>,
    deleted_branches: &mut Vec<String>,
    reporter: &mut F,
) -> io::Result<std::process::ExitStatus>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    match candidate.kind {
        PendingCleanCandidateKind::DeletedLocally => {
            archive_clean_candidate(session, &candidate.branch_name, reporter)?;
            untracked_branches.push(candidate.branch_name.clone());
            git::success_status()
        }
        PendingCleanCandidateKind::IntegratedIntoParent { .. } => {
            let status = delete_clean_candidate(session, &candidate.branch_name, reporter)?;
            if status.success() {
                deleted_branches.push(candidate.branch_name.clone());
            }
            Ok(status)
        }
    }
}

fn pending_clean_candidate_from_clean_candidate(
    candidate: &CleanCandidate,
) -> PendingCleanCandidate {
    PendingCleanCandidate {
        branch_name: candidate.branch_name.clone(),
        kind: match &candidate.reason {
            CleanReason::DeletedLocally => PendingCleanCandidateKind::DeletedLocally,
            CleanReason::IntegratedIntoParent { parent_base } => {
                PendingCleanCandidateKind::IntegratedIntoParent {
                    parent_base: parent_base.clone(),
                }
            }
        },
    }
}

fn delete_clean_candidate<F>(
    session: &mut crate::core::store::StoreSession,
    branch_name: &str,
    reporter: &mut F,
) -> io::Result<std::process::ExitStatus>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    let node = session
        .state
        .find_branch_by_name(branch_name)
        .cloned()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("tracked branch '{}' was not found", branch_name),
            )
        })?;
    let Some(parent_branch_name) =
        BranchGraph::new(&session.state).parent_branch_name(&node, &session.config.trunk_branch)
    else {
        return Err(io::Error::other(format!(
            "tracked parent for '{}' is missing from dig",
            node.branch_name
        )));
    };

    reporter(CleanEvent::DeleteStarted {
        branch_name: node.branch_name.clone(),
    })?;
    let status = git::delete_branch_force(&node.branch_name)?;
    if !status.success() {
        return Ok(status);
    }

    record_branch_archived(
        session,
        node.id,
        node.branch_name.clone(),
        BranchArchiveReason::IntegratedIntoParent {
            parent_branch: parent_branch_name,
        },
    )?;

    reporter(CleanEvent::DeleteCompleted {
        branch_name: node.branch_name,
    })?;

    Ok(status)
}

fn archive_clean_candidate<F>(
    session: &mut crate::core::store::StoreSession,
    branch_name: &str,
    reporter: &mut F,
) -> io::Result<()>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    let step = deleted_local::plan_deleted_local_step_for_branch(
        &session.state,
        &session.config.trunk_branch,
        branch_name,
    )?;

    reporter(CleanEvent::ArchiveStarted {
        branch_name: step.branch_name.clone(),
    })?;
    deleted_local::archive_deleted_local_step(session, &step)?;
    reporter(CleanEvent::ArchiveCompleted {
        branch_name: step.branch_name,
    })?;

    Ok(())
}
