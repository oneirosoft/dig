use std::io;
use std::process::ExitStatus;

use crate::core::clean::{self, CleanOptions};
use crate::core::deleted_local;
use crate::core::graph::BranchGraph;
use crate::core::restack::{self, RestackAction, RestackPreview};
use crate::core::store::{
    PendingOperationKind, PendingOperationState, PendingSyncOperation, PendingSyncPhase,
    clear_operation, load_operation, open_initialized,
};
use crate::core::workflow;
use crate::core::{adopt, commit, git, merge, orphan};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOptions {
    pub continue_operation: bool,
}

#[derive(Debug)]
pub enum SyncCompletion {
    Commit(commit::CommitOutcome),
    Adopt(adopt::AdoptOutcome),
    Merge(merge::MergeResumeOutcome),
    Clean {
        trunk_branch: String,
        outcome: clean::CleanApplyOutcome,
    },
    Orphan(orphan::OrphanOutcome),
    Full(FullSyncOutcome),
}

#[derive(Debug)]
pub struct FullSyncOutcome {
    pub deleted_branches: Vec<String>,
    pub restacked_branches: Vec<RestackPreview>,
    pub cleanup_plan: clean::CleanPlan,
}

#[derive(Debug)]
pub struct SyncOutcome {
    pub status: ExitStatus,
    pub completion: Option<SyncCompletion>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

#[derive(Debug, Default, Clone)]
struct LocalSyncProgress {
    deleted_branches: Vec<String>,
    restacked_branches: Vec<RestackPreview>,
}

#[derive(Debug)]
struct LocalSyncOutcome {
    status: ExitStatus,
    deleted_branches: Vec<String>,
    restacked_branches: Vec<RestackPreview>,
    failure_output: Option<String>,
    paused: bool,
}

#[derive(Debug)]
struct OutdatedBranchStep {
    branch_name: String,
    actions: Vec<RestackAction>,
}

pub fn run(options: &SyncOptions) -> io::Result<SyncOutcome> {
    if !options.continue_operation {
        return run_full_sync();
    }

    let session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let pending_operation = load_operation(&session.paths)?
        .ok_or_else(|| io::Error::other("no paused dig operation to resume"))?;

    if !git::is_rebase_in_progress(&session.repo) {
        clear_operation(&session.paths)?;
        return Err(io::Error::other(format!(
            "paused dig {} operation is stale; rerun the original command",
            pending_operation.origin.command_name()
        )));
    }

    let continue_output = git::continue_rebase()?;
    if !continue_output.status.success() {
        return Ok(SyncOutcome {
            status: continue_output.status,
            completion: None,
            failure_output: Some(continue_output.combined_output()),
            paused: true,
        });
    }

    match pending_operation.origin.clone() {
        crate::core::store::PendingOperationKind::Commit(payload) => {
            let outcome = commit::resume_after_sync(pending_operation, payload)?;
            let status = outcome.status;
            let failure_output = outcome.failure_output.clone();
            let paused = outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Commit(outcome)),
                failure_output,
                paused,
            })
        }
        crate::core::store::PendingOperationKind::Adopt(payload) => {
            let outcome = adopt::resume_after_sync(pending_operation, payload)?;
            let status = outcome.status;
            let failure_output = outcome.failure_output.clone();
            let paused = outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Adopt(outcome)),
                failure_output,
                paused,
            })
        }
        crate::core::store::PendingOperationKind::Merge(payload) => {
            let outcome = merge::resume_after_sync(pending_operation, payload)?;
            let status = outcome.outcome.status;
            let failure_output = outcome.outcome.failure_output.clone();
            let paused = outcome.outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Merge(outcome)),
                failure_output,
                paused,
            })
        }
        crate::core::store::PendingOperationKind::Clean(payload) => {
            let trunk_branch = payload.trunk_branch.clone();
            let outcome = clean::resume_after_sync(pending_operation, payload)?;
            let status = outcome.status;
            let failure_output = outcome.failure_output.clone();
            let paused = outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Clean {
                    trunk_branch,
                    outcome,
                }),
                failure_output,
                paused,
            })
        }
        crate::core::store::PendingOperationKind::Orphan(payload) => {
            let outcome = orphan::resume_after_sync(pending_operation, payload)?;
            let status = outcome.status;
            let failure_output = outcome.failure_output.clone();
            let paused = outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Orphan(outcome)),
                failure_output,
                paused,
            })
        }
        crate::core::store::PendingOperationKind::Sync(payload) => {
            let outcome = resume_full_sync(pending_operation, payload)?;
            finalize_full_sync_outcome(outcome)
        }
    }
}

fn run_full_sync() -> io::Result<SyncOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "sync")?;
    workflow::ensure_no_pending_operation(&session.paths, "sync")?;

    let original_branch = git::current_branch_name()?;
    let outcome = execute_local_sync(&mut session, original_branch, LocalSyncProgress::default())?;

    finalize_full_sync_outcome(outcome)
}

fn resume_full_sync(
    pending_operation: PendingOperationState,
    payload: PendingSyncOperation,
) -> io::Result<LocalSyncOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let mut progress = LocalSyncProgress {
        deleted_branches: payload.deleted_branches,
        restacked_branches: payload.restacked_branches,
    };

    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;
    progress
        .restacked_branches
        .extend(restack_outcome.restacked_branches.clone());

    if restack_outcome.paused {
        return Ok(LocalSyncOutcome {
            status: restack_outcome.status,
            deleted_branches: progress.deleted_branches,
            restacked_branches: progress.restacked_branches,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    execute_local_sync(&mut session, payload.original_branch, progress)
}

fn finalize_full_sync_outcome(outcome: LocalSyncOutcome) -> io::Result<SyncOutcome> {
    if outcome.paused || !outcome.status.success() {
        return Ok(SyncOutcome {
            status: outcome.status,
            completion: None,
            failure_output: outcome.failure_output,
            paused: outcome.paused,
        });
    }

    let cleanup_plan = clean::plan(&CleanOptions::default())?;

    Ok(SyncOutcome {
        status: outcome.status,
        completion: Some(SyncCompletion::Full(FullSyncOutcome {
            deleted_branches: outcome.deleted_branches,
            restacked_branches: outcome.restacked_branches,
            cleanup_plan,
        })),
        failure_output: outcome.failure_output,
        paused: false,
    })
}

fn execute_local_sync(
    session: &mut crate::core::store::StoreSession,
    original_branch: String,
    mut progress: LocalSyncProgress,
) -> io::Result<LocalSyncOutcome> {
    loop {
        if let Some(step) = plan_deleted_local_branch_step(session)? {
            if let Some(outcome) =
                apply_deleted_local_branch_step(session, &original_branch, &mut progress, step)?
            {
                return Ok(outcome);
            }

            continue;
        }

        if let Some(step) = plan_outdated_branch_step(session)? {
            if let Some(outcome) =
                apply_outdated_branch_step(session, &original_branch, &mut progress, step)?
            {
                return Ok(outcome);
            }

            continue;
        }

        return finish_local_sync(&original_branch, progress);
    }
}

fn finish_local_sync(
    original_branch: &str,
    progress: LocalSyncProgress,
) -> io::Result<LocalSyncOutcome> {
    let mut failure_output = None;
    let mut status = git::success_status()?;

    if let Some(outcome) = workflow::restore_original_branch_if_needed(original_branch)? {
        status = outcome.status;
        if !status.success() {
            failure_output = Some(format!(
                "sync completed, but failed to return to '{}'",
                original_branch
            ));
        }
    }

    Ok(LocalSyncOutcome {
        status,
        deleted_branches: progress.deleted_branches,
        restacked_branches: progress.restacked_branches,
        failure_output,
        paused: false,
    })
}

fn plan_deleted_local_branch_step(
    session: &crate::core::store::StoreSession,
) -> io::Result<Option<deleted_local::DeletedLocalBranchStep>> {
    deleted_local::next_deleted_local_step(&session.state, &session.config.trunk_branch)
}

fn apply_deleted_local_branch_step(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    step: deleted_local::DeletedLocalBranchStep,
) -> io::Result<Option<LocalSyncOutcome>> {
    let restack_actions = deleted_local::restack_actions_for_step(&session.state, &step)?;

    deleted_local::archive_deleted_local_step(session, &step)?;
    progress.deleted_branches.push(step.branch_name.clone());

    execute_sync_restack_step(
        session,
        original_branch,
        progress,
        PendingSyncPhase::ReconcileDeletedLocalBranches,
        &step.branch_name,
        &restack_actions,
    )
}

fn plan_outdated_branch_step(
    session: &crate::core::store::StoreSession,
) -> io::Result<Option<OutdatedBranchStep>> {
    let graph = BranchGraph::new(&session.state);
    let mut candidates = session
        .state
        .nodes
        .iter()
        .filter(|node| !node.archived)
        .cloned()
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        graph
            .branch_depth(left.id)
            .cmp(&graph.branch_depth(right.id))
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });

    for node in candidates {
        if !git::branch_exists(&node.branch_name)? {
            continue;
        }

        let parent_branch_name = graph
            .parent_branch_name(&node, &session.config.trunk_branch)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "tracked parent for '{}' is missing from dig",
                    node.branch_name
                ))
            })?;

        if !git::branch_exists(&parent_branch_name)? {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("parent branch '{}' does not exist", parent_branch_name),
            ));
        }

        if clean::branch_is_integrated(&parent_branch_name, &node.branch_name)? {
            continue;
        }

        let parent_head_oid = git::ref_oid(&parent_branch_name)?;
        let old_upstream_oid = git::merge_base(&parent_branch_name, &node.branch_name)?;
        if old_upstream_oid == parent_head_oid {
            continue;
        }

        let old_head_oid = git::ref_oid(&node.branch_name)?;
        let actions = restack::plan_after_branch_rebase(
            &session.state,
            node.id,
            &node.branch_name,
            &old_upstream_oid,
            &old_head_oid,
            &parent_branch_name,
        )?;

        return Ok(Some(OutdatedBranchStep {
            branch_name: node.branch_name,
            actions,
        }));
    }

    Ok(None)
}

fn apply_outdated_branch_step(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    step: OutdatedBranchStep,
) -> io::Result<Option<LocalSyncOutcome>> {
    execute_sync_restack_step(
        session,
        original_branch,
        progress,
        PendingSyncPhase::RestackOutdatedLocalStacks,
        &step.branch_name,
        &step.actions,
    )
}

fn execute_sync_restack_step(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    phase: PendingSyncPhase,
    step_branch_name: &str,
    actions: &[RestackAction],
) -> io::Result<Option<LocalSyncOutcome>> {
    if actions.is_empty() {
        return Ok(None);
    }

    let restack_outcome = workflow::execute_resumable_restack_operation(
        session,
        PendingOperationKind::Sync(PendingSyncOperation {
            original_branch: original_branch.to_string(),
            deleted_branches: progress.deleted_branches.clone(),
            restacked_branches: progress.restacked_branches.clone(),
            phase,
            step_branch_name: step_branch_name.to_string(),
        }),
        actions,
        &mut |_| Ok(()),
    )?;
    progress
        .restacked_branches
        .extend(restack_outcome.restacked_branches.clone());

    if restack_outcome.paused {
        return Ok(Some(LocalSyncOutcome {
            status: restack_outcome.status,
            deleted_branches: progress.deleted_branches.clone(),
            restacked_branches: progress.restacked_branches.clone(),
            failure_output: restack_outcome.failure_output,
            paused: true,
        }));
    }

    Ok(None)
}
