use std::collections::{BTreeSet, HashSet};
use std::io;
use std::process::ExitStatus;

use crate::core::clean::{self, CleanOptions, CleanPlanMode};
use crate::core::deleted_local;
use crate::core::gh::{self, PullRequestState, PullRequestStatus};
use crate::core::graph::BranchGraph;
use crate::core::restack::{self, RestackAction, RestackPreview};
use crate::core::store::types::DaggerState;
use crate::core::store::{
    BranchNode, ParentRef, PendingOperationKind, PendingOperationState, PendingSyncOperation,
    PendingSyncPhase, clear_operation, load_operation, open_initialized,
};
use crate::core::workflow;
use crate::core::{adopt, commit, git, merge, orphan, reparent};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOptions {
    pub continue_operation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStage {
    LocalSync {
        phase: PendingSyncPhase,
        step_branch_name: String,
        active_branch_name: String,
        deleted_branches: Vec<String>,
        restacked_branches: Vec<RestackPreview>,
    },
    CleanupResume {
        plan: clean::CleanPlan,
        active_branch_name: String,
        untracked_branches: Vec<String>,
        deleted_branches: Vec<String>,
        restacked_branches: Vec<RestackPreview>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    FetchingRemotes,
    RepairingClosedPullRequests,
    RemovingMergedLocalBranches,
    ReconcilingDeletedLocalBranch {
        step_branch_name: String,
    },
    PreparingRestack {
        step_branch_name: String,
    },
    RestackingBranch {
        branch_name: String,
        onto_branch: String,
    },
    InspectingPullRequestUpdates,
    UpdatingPullRequestBase {
        branch_name: String,
        pull_request_number: u64,
    },
    PushingRemoteBranch {
        branch_name: String,
        remote_name: String,
        kind: RemotePushActionKind,
    },
    DeletingBranch {
        branch_name: String,
    },
    ArchivingBranch {
        branch_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    StatusChanged(SyncStatus),
    StageStarted(SyncStage),
    BranchArchived {
        branch_name: String,
    },
    RestackStarted {
        branch_name: String,
        onto_branch: String,
    },
    RestackProgress {
        branch_name: String,
        onto_branch: String,
        current_commit: usize,
        total_commits: usize,
    },
    RestackCompleted {
        branch_name: String,
        onto_branch: String,
    },
    Cleanup(clean::CleanEvent),
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
    Reparent(reparent::ReparentOutcome),
    Full(FullSyncOutcome),
}

#[derive(Debug)]
pub struct FullSyncOutcome {
    pub repaired_pull_requests: Vec<PullRequestRepairOutcome>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePushActionKind {
    CreateRemoteBranch,
    UpdateRemoteBranch,
    ForceUpdateRemoteBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePushAction {
    pub target: git::BranchPushTarget,
    pub kind: RemotePushActionKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemotePushPlan {
    pub actions: Vec<RemotePushAction>,
}

#[derive(Debug)]
pub struct RemotePushOutcome {
    pub status: ExitStatus,
    pub pushed_actions: Vec<RemotePushAction>,
    pub failed_action: Option<RemotePushAction>,
    pub failure_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestRepairOutcome {
    pub branch_name: String,
    pub pull_request_number: u64,
    pub old_base_branch_name: String,
    pub new_base_branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestUpdateAction {
    pub branch_name: String,
    pub pull_request_number: u64,
    pub new_base_branch_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestUpdatePlan {
    pub actions: Vec<PullRequestUpdateAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPullRequestRepair {
    branch_name: String,
    pull_request_number: u64,
    old_base_branch_name: String,
    new_base_branch_name: String,
    was_draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentPullRequestRepairPlan {
    remote_target: git::BranchPushTarget,
    restore_source_ref: String,
    new_base_branch_name: String,
}

#[derive(Debug, Default, Clone)]
struct LocalSyncProgress {
    repaired_pull_requests: Vec<PullRequestRepairOutcome>,
    deleted_branches: Vec<String>,
    restacked_branches: Vec<RestackPreview>,
}

#[derive(Debug)]
struct LocalSyncOutcome {
    status: ExitStatus,
    remote_sync_enabled: bool,
    repaired_pull_requests: Vec<PullRequestRepairOutcome>,
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
    run_with_reporter(options, &mut |_| Ok(()))
}

pub fn run_with_reporter<F>(options: &SyncOptions, reporter: &mut F) -> io::Result<SyncOutcome>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    if !options.continue_operation {
        return run_full_sync_with_reporter(reporter);
    }

    let session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    let pending_operation = load_operation(&session.paths)?
        .ok_or_else(|| io::Error::other("no paused dgr operation to resume"))?;

    if !git::is_rebase_in_progress(&session.repo) {
        clear_operation(&session.paths)?;
        return Err(io::Error::other(format!(
            "paused dgr {} operation is stale; rerun the original command",
            pending_operation.origin.command_name()
        )));
    }

    match pending_operation.origin.clone() {
        PendingOperationKind::Commit(payload) => {
            let continue_output = git::continue_rebase()?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

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
        PendingOperationKind::Adopt(payload) => {
            let continue_output = git::continue_rebase()?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

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
        PendingOperationKind::Merge(payload) => {
            let continue_output = git::continue_rebase()?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

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
        PendingOperationKind::Clean(payload) => {
            let mut restacked_branches = payload.restacked_branches.clone();
            restacked_branches.extend(pending_operation.completed_branches().iter().cloned());
            let active_action = pending_operation.active_action().clone();
            reporter(SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
                branch_name: active_action.branch_name.clone(),
                onto_branch: active_action.new_base.branch_name.clone(),
            }))?;
            reporter(SyncEvent::StageStarted(SyncStage::CleanupResume {
                plan: clean::plan_for_resume(&payload)?,
                active_branch_name: active_action.branch_name.clone(),
                untracked_branches: payload.untracked_branches.clone(),
                deleted_branches: payload.deleted_branches.clone(),
                restacked_branches,
            }))?;
            let continue_output = git::continue_rebase_with_progress(|progress| {
                report_cleanup_continue_progress(reporter, &active_action, progress)
            })?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

            let trunk_branch = payload.trunk_branch.clone();
            let outcome =
                clean::resume_after_sync_with_reporter(pending_operation, payload, &mut |event| {
                    reporter(SyncEvent::Cleanup(event))
                })?;
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
        PendingOperationKind::Orphan(payload) => {
            let continue_output = git::continue_rebase()?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

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
        PendingOperationKind::Reparent(payload) => {
            let continue_output = git::continue_rebase()?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

            let outcome = reparent::resume_after_sync(pending_operation, payload)?;
            let status = outcome.status;
            let failure_output = outcome.failure_output.clone();
            let paused = outcome.paused;
            Ok(SyncOutcome {
                status,
                completion: Some(SyncCompletion::Reparent(outcome)),
                failure_output,
                paused,
            })
        }
        PendingOperationKind::Sync(payload) => {
            let active_action = pending_operation.active_action().clone();
            report_resumed_full_sync_stage_started(reporter, &pending_operation, &payload)?;
            let continue_output = git::continue_rebase_with_progress(|progress| {
                report_sync_continue_progress(reporter, &active_action, progress)
            })?;
            if !continue_output.status.success() {
                return Ok(paused_continue_outcome(continue_output));
            }

            let outcome =
                resume_full_sync_with_reporter(pending_operation, payload, reporter, false)?;
            finalize_full_sync_outcome(outcome)
        }
    }
}

fn paused_continue_outcome(continue_output: git::GitCommandOutput) -> SyncOutcome {
    SyncOutcome {
        status: continue_output.status,
        completion: None,
        failure_output: Some(continue_output.combined_output()),
        paused: true,
    }
}

fn run_full_sync_with_reporter<F>(reporter: &mut F) -> io::Result<SyncOutcome>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    let mut session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "sync")?;
    workflow::ensure_no_pending_operation(&session.paths, "sync")?;
    clean::reconcile_branch_divergence_state(&mut session)?;
    reporter(SyncEvent::StatusChanged(SyncStatus::FetchingRemotes))?;
    let remote_sync_enabled = fetch_sync_remotes(&session)?;
    let repaired_pull_requests = if remote_sync_enabled {
        reporter(SyncEvent::StatusChanged(
            SyncStatus::RepairingClosedPullRequests,
        ))?;
        repair_closed_pull_requests_for_deleted_parent_branches(&session)?
    } else {
        Vec::new()
    };
    let original_branch = git::current_branch_name()?;
    if remote_sync_enabled {
        reporter(SyncEvent::StatusChanged(
            SyncStatus::RemovingMergedLocalBranches,
        ))?;
        delete_local_branches_merged_into_deleted_parent_branches(&session, &original_branch)?;
    }
    let outcome = execute_local_sync(
        &mut session,
        original_branch,
        LocalSyncProgress {
            repaired_pull_requests,
            ..LocalSyncProgress::default()
        },
        remote_sync_enabled,
        reporter,
    )?;

    finalize_full_sync_outcome(outcome)
}

fn resume_full_sync_with_reporter<F>(
    pending_operation: PendingOperationState,
    payload: PendingSyncOperation,
    reporter: &mut F,
    emit_stage_started: bool,
) -> io::Result<LocalSyncOutcome>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    let mut session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    clean::reconcile_branch_divergence_state(&mut session)?;
    if emit_stage_started {
        report_resumed_full_sync_stage_started(reporter, &pending_operation, &payload)?;
    }

    let mut progress = LocalSyncProgress {
        repaired_pull_requests: Vec::new(),
        deleted_branches: payload.deleted_branches,
        restacked_branches: payload.restacked_branches,
    };

    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |event| report_restack_event(reporter, event),
    )?;
    progress
        .restacked_branches
        .extend(restack_outcome.restacked_branches.clone());

    if restack_outcome.paused {
        return Ok(LocalSyncOutcome {
            status: restack_outcome.status,
            remote_sync_enabled: payload.remote_sync_enabled,
            repaired_pull_requests: progress.repaired_pull_requests,
            deleted_branches: progress.deleted_branches,
            restacked_branches: progress.restacked_branches,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }

    execute_local_sync(
        &mut session,
        payload.original_branch,
        progress,
        payload.remote_sync_enabled,
        reporter,
    )
}

fn report_resumed_full_sync_stage_started<F>(
    reporter: &mut F,
    pending_operation: &PendingOperationState,
    payload: &PendingSyncOperation,
) -> io::Result<()>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    reporter(SyncEvent::StatusChanged(status_for_local_sync_phase(
        payload.phase,
        &payload.step_branch_name,
    )))?;
    let mut resumed_restacked_branches = payload.restacked_branches.clone();
    resumed_restacked_branches.extend(pending_operation.completed_branches().iter().cloned());

    reporter(SyncEvent::StageStarted(SyncStage::LocalSync {
        phase: payload.phase,
        step_branch_name: payload.step_branch_name.clone(),
        active_branch_name: pending_operation.active_action().branch_name.clone(),
        deleted_branches: payload.deleted_branches.clone(),
        restacked_branches: resumed_restacked_branches,
    }))
}

fn report_sync_continue_progress<F>(
    reporter: &mut F,
    action: &RestackAction,
    progress: git::RebaseProgress,
) -> io::Result<()>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    reporter(SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
        branch_name: action.branch_name.clone(),
        onto_branch: action.new_base.branch_name.clone(),
    }))?;
    reporter(SyncEvent::RestackProgress {
        branch_name: action.branch_name.clone(),
        onto_branch: action.new_base.branch_name.clone(),
        current_commit: progress.current,
        total_commits: progress.total,
    })
}

fn report_cleanup_continue_progress<F>(
    reporter: &mut F,
    action: &RestackAction,
    progress: git::RebaseProgress,
) -> io::Result<()>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    reporter(SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
        branch_name: action.branch_name.clone(),
        onto_branch: action.new_base.branch_name.clone(),
    }))?;
    reporter(SyncEvent::Cleanup(clean::CleanEvent::RebaseProgress {
        branch_name: action.branch_name.clone(),
        onto_branch: action.new_base.branch_name.clone(),
        current_commit: progress.current,
        total_commits: progress.total,
    }))
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

    let cleanup_plan = if outcome.remote_sync_enabled {
        clean::plan_for_sync()?
    } else {
        clean::plan(&CleanOptions::default())?
    };

    Ok(SyncOutcome {
        status: outcome.status,
        completion: Some(SyncCompletion::Full(FullSyncOutcome {
            repaired_pull_requests: outcome.repaired_pull_requests,
            deleted_branches: outcome.deleted_branches,
            restacked_branches: outcome.restacked_branches,
            cleanup_plan,
        })),
        failure_output: outcome.failure_output,
        paused: false,
    })
}

fn execute_local_sync<F>(
    session: &mut crate::core::store::StoreSession,
    original_branch: String,
    mut progress: LocalSyncProgress,
    remote_sync_enabled: bool,
    reporter: &mut F,
) -> io::Result<LocalSyncOutcome>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    let cleanup_mode = clean::mode_for_sync(remote_sync_enabled);

    loop {
        clean::reconcile_branch_divergence_state(session)?;

        if let Some(step) = plan_deleted_local_branch_step(session)? {
            if let Some(outcome) = apply_deleted_local_branch_step(
                session,
                &original_branch,
                &mut progress,
                step,
                remote_sync_enabled,
                reporter,
            )? {
                return Ok(outcome);
            }

            continue;
        }

        if let Some(step) = plan_outdated_branch_step(session, cleanup_mode)? {
            if let Some(outcome) = apply_outdated_branch_step(
                session,
                &original_branch,
                &mut progress,
                step,
                remote_sync_enabled,
                reporter,
            )? {
                return Ok(outcome);
            }

            continue;
        }

        return finish_local_sync(&original_branch, progress, remote_sync_enabled);
    }
}

fn finish_local_sync(
    original_branch: &str,
    progress: LocalSyncProgress,
    remote_sync_enabled: bool,
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
        remote_sync_enabled,
        repaired_pull_requests: progress.repaired_pull_requests,
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

fn apply_deleted_local_branch_step<F>(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    step: deleted_local::DeletedLocalBranchStep,
    remote_sync_enabled: bool,
    reporter: &mut F,
) -> io::Result<Option<LocalSyncOutcome>>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    let restack_actions = deleted_local::restack_actions_for_step(&session.state, &step)?;

    if !restack_actions.is_empty() {
        report_local_sync_stage_started(
            reporter,
            PendingSyncPhase::ReconcileDeletedLocalBranches,
            &step.branch_name,
            &restack_actions,
            progress,
        )?;
    }
    deleted_local::archive_deleted_local_step(session, &step)?;
    progress.deleted_branches.push(step.branch_name.clone());
    if !restack_actions.is_empty() {
        reporter(SyncEvent::BranchArchived {
            branch_name: step.branch_name.clone(),
        })?;
    }

    execute_sync_restack_step(
        session,
        original_branch,
        progress,
        PendingSyncPhase::ReconcileDeletedLocalBranches,
        &step.branch_name,
        &restack_actions,
        remote_sync_enabled,
        reporter,
        false,
    )
}

fn plan_outdated_branch_step(
    session: &crate::core::store::StoreSession,
    cleanup_mode: CleanPlanMode,
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

        if clean::cleanup_candidate_for_branch(
            &session.state,
            &session.config.trunk_branch,
            &node,
            cleanup_mode,
        )?
        .is_some()
        {
            continue;
        }

        let (parent_base, _) = deleted_local::resolve_replacement_parent(
            &session.state,
            &session.config.trunk_branch,
            &node.parent,
        )
        .map_err(|_| {
            io::Error::other(format!(
                "tracked parent for '{}' is missing from dagger",
                node.branch_name
            ))
        })?;
        let parent_branch_name = parent_base.branch_name;

        if !git::branch_exists(&parent_branch_name)? {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("parent branch '{}' does not exist", parent_branch_name),
            ));
        }

        if clean::tracked_branch_is_integrated(&node, &parent_branch_name)? {
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
            &restack::RestackBaseTarget::local(&parent_branch_name),
        )?;

        return Ok(Some(OutdatedBranchStep {
            branch_name: node.branch_name,
            actions,
        }));
    }

    Ok(None)
}

fn apply_outdated_branch_step<F>(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    step: OutdatedBranchStep,
    remote_sync_enabled: bool,
    reporter: &mut F,
) -> io::Result<Option<LocalSyncOutcome>>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    execute_sync_restack_step(
        session,
        original_branch,
        progress,
        PendingSyncPhase::RestackOutdatedLocalStacks,
        &step.branch_name,
        &step.actions,
        remote_sync_enabled,
        reporter,
        true,
    )
}

fn execute_sync_restack_step<F>(
    session: &mut crate::core::store::StoreSession,
    original_branch: &str,
    progress: &mut LocalSyncProgress,
    phase: PendingSyncPhase,
    step_branch_name: &str,
    actions: &[RestackAction],
    remote_sync_enabled: bool,
    reporter: &mut F,
    emit_stage_started: bool,
) -> io::Result<Option<LocalSyncOutcome>>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    if actions.is_empty() {
        return Ok(None);
    }

    if emit_stage_started {
        report_local_sync_stage_started(reporter, phase, step_branch_name, actions, progress)?;
    }

    let restack_outcome = workflow::execute_resumable_restack_operation(
        session,
        PendingOperationKind::Sync(PendingSyncOperation {
            original_branch: original_branch.to_string(),
            remote_sync_enabled,
            deleted_branches: progress.deleted_branches.clone(),
            restacked_branches: progress.restacked_branches.clone(),
            phase,
            step_branch_name: step_branch_name.to_string(),
        }),
        actions,
        &mut |event| report_restack_event(reporter, event),
    )?;
    progress
        .restacked_branches
        .extend(restack_outcome.restacked_branches.clone());

    if restack_outcome.paused {
        return Ok(Some(LocalSyncOutcome {
            status: restack_outcome.status,
            remote_sync_enabled,
            repaired_pull_requests: progress.repaired_pull_requests.clone(),
            deleted_branches: progress.deleted_branches.clone(),
            restacked_branches: progress.restacked_branches.clone(),
            failure_output: restack_outcome.failure_output,
            paused: true,
        }));
    }

    Ok(None)
}

fn report_local_sync_stage_started<F>(
    reporter: &mut F,
    phase: PendingSyncPhase,
    step_branch_name: &str,
    actions: &[RestackAction],
    progress: &LocalSyncProgress,
) -> io::Result<()>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    reporter(SyncEvent::StatusChanged(status_for_local_sync_phase(
        phase,
        step_branch_name,
    )))?;
    reporter(SyncEvent::StageStarted(SyncStage::LocalSync {
        phase,
        step_branch_name: step_branch_name.to_string(),
        active_branch_name: actions[0].branch_name.clone(),
        deleted_branches: progress.deleted_branches.clone(),
        restacked_branches: progress.restacked_branches.clone(),
    }))
}

fn status_for_local_sync_phase(phase: PendingSyncPhase, step_branch_name: &str) -> SyncStatus {
    match phase {
        PendingSyncPhase::ReconcileDeletedLocalBranches => {
            SyncStatus::ReconcilingDeletedLocalBranch {
                step_branch_name: step_branch_name.to_string(),
            }
        }
        PendingSyncPhase::RestackOutdatedLocalStacks => SyncStatus::PreparingRestack {
            step_branch_name: step_branch_name.to_string(),
        },
    }
}

fn report_restack_event<F>(
    reporter: &mut F,
    event: workflow::RestackExecutionEvent<'_>,
) -> io::Result<()>
where
    F: FnMut(SyncEvent) -> io::Result<()>,
{
    match event {
        workflow::RestackExecutionEvent::Started(action) => {
            reporter(SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }))?;
            reporter(SyncEvent::RestackStarted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            })
        }
        workflow::RestackExecutionEvent::Progress { action, progress } => {
            reporter(SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            }))?;
            reporter(SyncEvent::RestackProgress {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
                current_commit: progress.current,
                total_commits: progress.total,
            })
        }
        workflow::RestackExecutionEvent::Completed(action) => {
            reporter(SyncEvent::RestackCompleted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base.branch_name.clone(),
            })
        }
    }
}

fn repair_closed_pull_requests_for_deleted_parent_branches(
    session: &crate::core::store::StoreSession,
) -> io::Result<Vec<PullRequestRepairOutcome>> {
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

    let mut repaired_pull_requests = Vec::new();
    for node in candidates {
        let Some(parent_plan) = plan_parent_pull_request_repair(session, &node)? else {
            continue;
        };

        let pending_repairs = plan_pull_request_repairs_for_children(
            session,
            &node,
            &parent_plan.new_base_branch_name,
        )?;
        if pending_repairs.is_empty() {
            continue;
        }

        restore_remote_branch_for_pull_request_repair(
            &parent_plan.remote_target,
            &parent_plan.restore_source_ref,
        )?;

        for repair in &pending_repairs {
            gh::reopen_pull_request(repair.pull_request_number).map_err(|err| {
                io::Error::other(format!(
                    "failed to reopen tracked pull request #{} for '{}': {err}",
                    repair.pull_request_number, repair.branch_name
                ))
            })?;

            if !repair.was_draft {
                gh::mark_pull_request_as_draft(repair.pull_request_number).map_err(|err| {
                    io::Error::other(format!(
                        "failed to convert tracked pull request #{} for '{}' back to draft: {err}",
                        repair.pull_request_number, repair.branch_name
                    ))
                })?;
            }

            gh::retarget_pull_request_base(
                repair.pull_request_number,
                &repair.new_base_branch_name,
            )
            .map_err(|err| {
                io::Error::other(format!(
                    "failed to retarget tracked pull request #{} for '{}' onto '{}': {err}",
                    repair.pull_request_number, repair.branch_name, repair.new_base_branch_name
                ))
            })?;
        }

        delete_restored_remote_branch_after_pull_request_repair(&parent_plan.remote_target)?;

        repaired_pull_requests.extend(pending_repairs.into_iter().map(|repair| {
            PullRequestRepairOutcome {
                branch_name: repair.branch_name,
                pull_request_number: repair.pull_request_number,
                old_base_branch_name: repair.old_base_branch_name,
                new_base_branch_name: repair.new_base_branch_name,
            }
        }));
    }

    Ok(repaired_pull_requests)
}

fn delete_local_branches_merged_into_deleted_parent_branches(
    session: &crate::core::store::StoreSession,
    current_branch_name: &str,
) -> io::Result<()> {
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
            .branch_depth(right.id)
            .cmp(&graph.branch_depth(left.id))
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });

    for node in candidates {
        if node.branch_name == current_branch_name || !git::branch_exists(&node.branch_name)? {
            continue;
        }
        if !parent_branch_is_unavailable_for_sync_cleanup(&session.state, &node)? {
            continue;
        }

        let Some(remote_target) = git::branch_push_target(&node.branch_name)? else {
            continue;
        };
        if git::remote_tracking_branch_exists(
            &remote_target.remote_name,
            &remote_target.branch_name,
        )? {
            continue;
        }
        if merged_pull_request_restore_source(&node)?.is_none() {
            continue;
        }

        let delete_status = git::delete_branch_force(&node.branch_name)?;
        if !delete_status.success() {
            return Err(io::Error::other(format!(
                "failed to remove merged local branch '{}' before sync cleanup",
                node.branch_name
            )));
        }
    }

    Ok(())
}

fn parent_branch_is_unavailable_for_sync_cleanup(
    state: &DaggerState,
    node: &BranchNode,
) -> io::Result<bool> {
    let ParentRef::Branch { node_id } = node.parent else {
        return Ok(false);
    };
    let Some(parent_node) = state.find_any_branch_by_id(node_id) else {
        return Ok(false);
    };

    Ok(parent_node.archived || !git::branch_exists(&parent_node.branch_name)?)
}

fn plan_parent_pull_request_repair(
    session: &crate::core::store::StoreSession,
    node: &BranchNode,
) -> io::Result<Option<ParentPullRequestRepairPlan>> {
    let Some(remote_target) = git::branch_push_target(&node.branch_name)? else {
        return Ok(None);
    };
    if git::remote_tracking_branch_exists(&remote_target.remote_name, &remote_target.branch_name)? {
        return Ok(None);
    }

    if let Some(cleanup_candidate) = clean::cleanup_candidate_for_branch(
        &session.state,
        &session.config.trunk_branch,
        node,
        CleanPlanMode::RemoteAwareSync,
    )? {
        let restore_source_ref = if git::branch_exists(&node.branch_name)? {
            node.branch_name.clone()
        } else if let Some(source_ref) = merged_pull_request_restore_source(node)? {
            source_ref
        } else {
            return Ok(None);
        };

        return Ok(Some(ParentPullRequestRepairPlan {
            remote_target,
            restore_source_ref,
            new_base_branch_name: cleanup_candidate.parent_branch_name,
        }));
    }

    let Some(restore_source_ref) = merged_pull_request_restore_source(node)? else {
        return Ok(None);
    };
    let Ok((new_parent_base, _)) = deleted_local::resolve_replacement_parent(
        &session.state,
        &session.config.trunk_branch,
        &node.parent,
    ) else {
        return Ok(None);
    };

    Ok(Some(ParentPullRequestRepairPlan {
        remote_target,
        restore_source_ref,
        new_base_branch_name: new_parent_base.branch_name,
    }))
}

fn merged_pull_request_restore_source(node: &BranchNode) -> io::Result<Option<String>> {
    let Some(pull_request) = node.pull_request.as_ref() else {
        return Ok(None);
    };
    let pull_request_status = gh::view_pull_request(pull_request.number).map_err(|err| {
        io::Error::other(format!(
            "failed to inspect tracked pull request #{} for '{}': {}",
            pull_request.number, node.branch_name, err
        ))
    })?;

    if pull_request_status.state != PullRequestState::Merged
        || pull_request_status.merged_at.is_none()
    {
        return Ok(None);
    }

    Ok(pull_request_status.head_ref_oid)
}

fn plan_pull_request_repairs_for_children(
    session: &crate::core::store::StoreSession,
    parent_node: &BranchNode,
    new_base_branch_name: &str,
) -> io::Result<Vec<PendingPullRequestRepair>> {
    let graph = BranchGraph::new(&session.state);
    let mut children = graph
        .active_children_ids(parent_node.id)
        .into_iter()
        .filter_map(|child_id| session.state.find_branch_by_id(child_id).cloned())
        .collect::<Vec<_>>();
    children.sort_by(|left, right| left.branch_name.cmp(&right.branch_name));

    let mut pending_repairs = Vec::new();
    for child in children {
        if !git::branch_exists(&child.branch_name)? {
            continue;
        }

        let Some(tracked_pull_request) = child.pull_request.as_ref() else {
            continue;
        };
        let pull_request_status =
            gh::view_pull_request(tracked_pull_request.number).map_err(|err| {
                io::Error::other(format!(
                    "failed to inspect tracked pull request #{} for '{}': {err}",
                    tracked_pull_request.number, child.branch_name
                ))
            })?;

        if pull_request_needs_repair(
            &pull_request_status,
            &child.branch_name,
            &parent_node.branch_name,
        ) {
            pending_repairs.push(PendingPullRequestRepair {
                branch_name: child.branch_name.clone(),
                pull_request_number: tracked_pull_request.number,
                old_base_branch_name: parent_node.branch_name.clone(),
                new_base_branch_name: new_base_branch_name.to_string(),
                was_draft: pull_request_status.is_draft,
            });
        }
    }

    Ok(pending_repairs)
}

fn pull_request_needs_repair(
    pull_request_status: &PullRequestStatus,
    expected_head_branch_name: &str,
    expected_base_branch_name: &str,
) -> bool {
    pull_request_status.state == PullRequestState::Closed
        && pull_request_status.merged_at.is_none()
        && pull_request_status.head_ref_name == expected_head_branch_name
        && pull_request_status.base_ref_name == expected_base_branch_name
}

fn restore_remote_branch_for_pull_request_repair(
    target: &git::BranchPushTarget,
    restore_source_ref: &str,
) -> io::Result<()> {
    let push_output = git::push_ref_to_remote_branch(
        &target.remote_name,
        restore_source_ref,
        &target.branch_name,
    )?;
    if push_output.status.success() {
        Ok(())
    } else {
        let combined_output = push_output.combined_output();
        Err(io::Error::other(if combined_output.is_empty() {
            format!(
                "failed to temporarily restore remote branch '{}' on '{}'",
                target.branch_name, target.remote_name
            )
        } else {
            format!(
                "failed to temporarily restore remote branch '{}' on '{}': {}",
                target.branch_name, target.remote_name, combined_output
            )
        }))
    }
}

fn delete_restored_remote_branch_after_pull_request_repair(
    target: &git::BranchPushTarget,
) -> io::Result<()> {
    let delete_output = git::delete_branch_from_remote(target)?;
    if delete_output.status.success() {
        Ok(())
    } else {
        let combined_output = delete_output.combined_output();
        Err(io::Error::other(if combined_output.is_empty() {
            format!(
                "failed to delete temporary remote branch '{}' on '{}'",
                target.branch_name, target.remote_name
            )
        } else {
            format!(
                "failed to delete temporary remote branch '{}' on '{}': {}",
                target.branch_name, target.remote_name, combined_output
            )
        }))
    }
}

fn fetch_sync_remotes(session: &crate::core::store::StoreSession) -> io::Result<bool> {
    let mut remote_names = BTreeSet::new();

    for node in session.state.nodes.iter().filter(|node| !node.archived) {
        if !git::branch_exists(&node.branch_name)? {
            continue;
        }

        if let Some(target) = git::branch_push_target(&node.branch_name)? {
            remote_names.insert(target.remote_name);
        }
    }

    if remote_names.is_empty() {
        return Ok(false);
    }

    for remote_name in remote_names {
        let fetch_output = git::fetch_remote(&remote_name)?;
        if !fetch_output.status.success() {
            let combined_output = fetch_output.combined_output();
            return Err(io::Error::other(if combined_output.is_empty() {
                format!("git fetch --prune '{remote_name}' failed")
            } else {
                format!("git fetch --prune '{remote_name}' failed: {combined_output}")
            }));
        }
    }

    Ok(true)
}

pub fn plan_remote_pushes(
    restacked_branch_names: &[String],
    excluded_branch_names: &[String],
) -> io::Result<RemotePushPlan> {
    let session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    let excluded_branch_names = excluded_branch_names
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut planned_branch_names = HashSet::new();
    let mut actions = Vec::new();

    for branch_name in dedup_branch_names(restacked_branch_names) {
        let Some(action) = plan_remote_push_action(
            &branch_name,
            &excluded_branch_names,
            true,
            &mut planned_branch_names,
        )?
        else {
            continue;
        };

        actions.push(action);
    }

    let mut active_branch_names = session
        .state
        .nodes
        .iter()
        .filter(|node| !node.archived)
        .map(|node| node.branch_name.clone())
        .collect::<Vec<_>>();
    active_branch_names.sort();

    for branch_name in active_branch_names {
        let Some(action) = plan_remote_push_action(
            &branch_name,
            &excluded_branch_names,
            false,
            &mut planned_branch_names,
        )?
        else {
            continue;
        };

        actions.push(action);
    }

    Ok(RemotePushPlan { actions })
}

pub fn execute_remote_push_plan(plan: &RemotePushPlan) -> io::Result<RemotePushOutcome> {
    execute_remote_push_plan_with_reporter(plan, &mut |_| Ok(()))
}

pub fn execute_remote_push_plan_with_reporter<F>(
    plan: &RemotePushPlan,
    reporter: &mut F,
) -> io::Result<RemotePushOutcome>
where
    F: FnMut(SyncStatus) -> io::Result<()>,
{
    let mut pushed_actions = Vec::new();

    for action in &plan.actions {
        reporter(SyncStatus::PushingRemoteBranch {
            branch_name: action.target.branch_name.clone(),
            remote_name: action.target.remote_name.clone(),
            kind: action.kind,
        })?;
        let push_output = match action.kind {
            RemotePushActionKind::CreateRemoteBranch | RemotePushActionKind::UpdateRemoteBranch => {
                git::push_branch_to_remote(&action.target)?
            }
            RemotePushActionKind::ForceUpdateRemoteBranch => {
                git::force_push_branch_to_remote_with_lease(&action.target)?
            }
        };

        if !push_output.status.success() {
            return Ok(RemotePushOutcome {
                status: push_output.status,
                pushed_actions,
                failed_action: Some(action.clone()),
                failure_output: Some(push_output.combined_output()),
            });
        }

        pushed_actions.push(action.clone());
    }

    Ok(RemotePushOutcome {
        status: git::success_status()?,
        pushed_actions,
        failed_action: None,
        failure_output: None,
    })
}

pub fn plan_pull_request_updates(
    restacked_branch_names: &[String],
) -> io::Result<PullRequestUpdatePlan> {
    let session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    let candidate_branch_names = dedup_branch_names(restacked_branch_names);
    let mut actions = Vec::new();

    for branch_name in candidate_branch_names {
        let Some(node) = session.state.find_branch_by_name(&branch_name) else {
            continue;
        };
        let Some(pull_request) = node.pull_request.as_ref() else {
            continue;
        };
        let Ok((parent_base, _)) = deleted_local::resolve_replacement_parent(
            &session.state,
            &session.config.trunk_branch,
            &node.parent,
        ) else {
            continue;
        };

        let pull_request_status = gh::view_pull_request(pull_request.number).map_err(|err| {
            io::Error::other(format!(
                "failed to inspect tracked pull request #{} for '{}': {}",
                pull_request.number, node.branch_name, err
            ))
        })?;

        if pull_request_status.state != PullRequestState::Open
            || pull_request_status.base_ref_name == parent_base.branch_name
        {
            continue;
        }

        actions.push(PullRequestUpdateAction {
            branch_name: node.branch_name.clone(),
            pull_request_number: pull_request.number,
            new_base_branch_name: parent_base.branch_name,
        });
    }

    Ok(PullRequestUpdatePlan { actions })
}

pub fn execute_pull_request_update_plan(
    plan: &PullRequestUpdatePlan,
) -> io::Result<Vec<PullRequestUpdateAction>> {
    execute_pull_request_update_plan_with_reporter(plan, &mut |_| Ok(()))
}

pub fn execute_pull_request_update_plan_with_reporter<F>(
    plan: &PullRequestUpdatePlan,
    reporter: &mut F,
) -> io::Result<Vec<PullRequestUpdateAction>>
where
    F: FnMut(SyncStatus) -> io::Result<()>,
{
    let mut updated_actions = Vec::new();

    for action in &plan.actions {
        reporter(SyncStatus::UpdatingPullRequestBase {
            branch_name: action.branch_name.clone(),
            pull_request_number: action.pull_request_number,
        })?;
        gh::retarget_pull_request_base(action.pull_request_number, &action.new_base_branch_name)
            .map_err(|err| {
                io::Error::other(format!(
                    "failed to retarget tracked pull request #{} for '{}' onto '{}': {}",
                    action.pull_request_number,
                    action.branch_name,
                    action.new_base_branch_name,
                    err
                ))
            })?;
        updated_actions.push(action.clone());
    }

    Ok(updated_actions)
}

fn dedup_branch_names(branch_names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for branch_name in branch_names {
        if seen.insert(branch_name.clone()) {
            deduped.push(branch_name.clone());
        }
    }

    deduped
}

fn plan_remote_push_action(
    branch_name: &str,
    excluded_branch_names: &HashSet<String>,
    allow_force_update: bool,
    planned_branch_names: &mut HashSet<String>,
) -> io::Result<Option<RemotePushAction>> {
    if excluded_branch_names.contains(branch_name)
        || !planned_branch_names.insert(branch_name.into())
    {
        return Ok(None);
    }

    if !git::branch_exists(branch_name)? {
        return Ok(None);
    }

    let Some(target) = git::branch_push_target(branch_name)? else {
        return Ok(None);
    };
    let Some(remote_oid) =
        git::remote_tracking_branch_oid(&target.remote_name, &target.branch_name)?
    else {
        return Ok(Some(RemotePushAction {
            target,
            kind: RemotePushActionKind::CreateRemoteBranch,
        }));
    };

    let local_oid = git::ref_oid(branch_name)?;
    if remote_oid == local_oid {
        return Ok(None);
    }

    if allow_force_update {
        return Ok(Some(RemotePushAction {
            target,
            kind: RemotePushActionKind::ForceUpdateRemoteBranch,
        }));
    }

    let remote_tracking_branch_ref =
        git::remote_tracking_branch_ref(&target.remote_name, &target.branch_name);
    if git::merge_base(&remote_tracking_branch_ref, branch_name)? == remote_oid {
        return Ok(Some(RemotePushAction {
            target,
            kind: RemotePushActionKind::UpdateRemoteBranch,
        }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{
        PullRequestUpdatePlan, RemotePushActionKind, RemotePushPlan, SyncEvent, SyncOptions,
        SyncStage, SyncStatus, execute_pull_request_update_plan_with_reporter,
        execute_remote_push_plan_with_reporter, plan_remote_pushes, pull_request_needs_repair, run,
        run_with_reporter,
    };
    use crate::core::gh::{PullRequestState, PullRequestStatus};
    use crate::core::git::BranchPushTarget;
    use crate::core::test_support::{
        append_file, commit_file, create_tracked_branch, git_ok, git_output, initialize_main_repo,
        with_temp_repo,
    };
    use std::env;
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn initialize_origin_remote(repo: &std::path::Path) {
        git_ok(repo, &["init", "--bare", ".git/origin.git"]);
        git_ok(repo, &["remote", "add", "origin", ".git/origin.git"]);
        git_ok(repo, &["push", "-u", "origin", "main"]);
        git_ok(
            repo,
            &[
                "--git-dir=.git/origin.git",
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ],
        );
    }

    fn install_fake_executable(bin_dir: &Path, name: &str, script: &str) {
        fs::create_dir_all(bin_dir).unwrap();
        #[cfg(unix)]
        {
            let path = bin_dir.join(name);
            fs::write(&path, script).unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
        #[cfg(windows)]
        {
            // On Windows, Command::new("gh") finds gh.cmd in PATH
            let path = bin_dir.join(format!("{name}.cmd"));
            fs::write(&path, script).unwrap();
        }
    }

    fn path_with_prepend(dir: &Path) -> String {
        let existing_path = env::var("PATH").unwrap_or_default();
        if existing_path.is_empty() {
            dir.display().to_string()
        } else {
            let sep = if cfg!(windows) { ";" } else { ":" };
            format!("{}{sep}{existing_path}", dir.display())
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        original_value: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: String) -> Self {
            let original_value = env::var(key).ok();
            unsafe {
                env::set_var(key, value);
            }
            Self {
                key,
                original_value,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original_value {
                Some(value) => unsafe {
                    env::set_var(self.key, value);
                },
                None => unsafe {
                    env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    fn plans_force_pushes_and_missing_remote_branches_while_excluding_cleanup_candidates() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            initialize_origin_remote(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
            create_tracked_branch("feat/auth-ui");
            commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
            git_ok(repo, &["checkout", "feat/auth"]);
            append_file(
                repo,
                "auth.txt",
                "auth local\n",
                "feat: auth local follow-up",
            );
            create_tracked_branch("feat/merged");
            commit_file(repo, "merged.txt", "merged\n", "feat: merged");

            let plan = plan_remote_pushes(&["feat/auth".to_string()], &["feat/merged".to_string()])
                .unwrap();

            assert_eq!(plan.actions.len(), 2);
            assert_eq!(plan.actions[0].target.branch_name, "feat/auth");
            assert_eq!(
                plan.actions[0].kind,
                RemotePushActionKind::ForceUpdateRemoteBranch
            );
            assert_eq!(plan.actions[1].target.branch_name, "feat/auth-ui");
            assert_eq!(
                plan.actions[1].kind,
                RemotePushActionKind::CreateRemoteBranch
            );
            assert!(
                plan.actions
                    .iter()
                    .all(|action| action.target.branch_name != "feat/merged")
            );
        });
    }

    #[test]
    fn plans_fast_forward_pushes_for_active_branches_ahead_of_remote() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            initialize_origin_remote(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
            append_file(
                repo,
                "auth.txt",
                "auth local\n",
                "feat: auth local follow-up",
            );

            let plan = plan_remote_pushes(&[], &[]).unwrap();

            assert_eq!(plan.actions.len(), 1);
            assert_eq!(plan.actions[0].target.branch_name, "feat/auth");
            assert_eq!(
                plan.actions[0].kind,
                RemotePushActionKind::UpdateRemoteBranch
            );
        });
    }

    #[test]
    fn repairs_only_closed_unmerged_pull_requests_with_expected_head_and_base() {
        assert!(pull_request_needs_repair(
            &PullRequestStatus {
                number: 42,
                state: PullRequestState::Closed,
                merged_at: None,
                base_ref_name: "feat/auth".into(),
                head_ref_name: "feat/auth-ui".into(),
                head_ref_oid: None,
                is_draft: false,
                url: "https://github.com/oneirosoft/dagger/pull/42".into(),
            },
            "feat/auth-ui",
            "feat/auth",
        ));
        assert!(!pull_request_needs_repair(
            &PullRequestStatus {
                number: 42,
                state: PullRequestState::Open,
                merged_at: None,
                base_ref_name: "feat/auth".into(),
                head_ref_name: "feat/auth-ui".into(),
                head_ref_oid: None,
                is_draft: false,
                url: "https://github.com/oneirosoft/dagger/pull/42".into(),
            },
            "feat/auth-ui",
            "feat/auth",
        ));
        assert!(!pull_request_needs_repair(
            &PullRequestStatus {
                number: 42,
                state: PullRequestState::Closed,
                merged_at: Some("2026-03-26T12:00:00Z".into()),
                base_ref_name: "feat/auth".into(),
                head_ref_name: "feat/auth-ui".into(),
                head_ref_oid: None,
                is_draft: false,
                url: "https://github.com/oneirosoft/dagger/pull/42".into(),
            },
            "feat/auth-ui",
            "feat/auth",
        ));
        assert!(!pull_request_needs_repair(
            &PullRequestStatus {
                number: 42,
                state: PullRequestState::Closed,
                merged_at: None,
                base_ref_name: "main".into(),
                head_ref_name: "feat/auth-ui".into(),
                head_ref_oid: None,
                is_draft: false,
                url: "https://github.com/oneirosoft/dagger/pull/42".into(),
            },
            "feat/auth-ui",
            "feat/auth",
        ));
        assert!(!pull_request_needs_repair(
            &PullRequestStatus {
                number: 42,
                state: PullRequestState::Closed,
                merged_at: None,
                base_ref_name: "feat/auth".into(),
                head_ref_name: "feat/auth-api".into(),
                head_ref_oid: None,
                is_draft: false,
                url: "https://github.com/oneirosoft/dagger/pull/42".into(),
            },
            "feat/auth-ui",
            "feat/auth",
        ));
    }

    #[test]
    fn emits_preflight_status_events_before_first_local_sync_stage() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            initialize_origin_remote(repo);
            crate::core::init::run(&crate::core::init::InitOptions::default()).unwrap();
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
            git_ok(repo, &["checkout", "main"]);
            commit_file(repo, "README.md", "root\nmain\n", "feat: trunk follow-up");
            git_ok(repo, &["checkout", "feat/auth"]);

            let mut events = Vec::new();
            let outcome = run_with_reporter(&SyncOptions::default(), &mut |event| {
                events.push(event.clone());
                Ok(())
            })
            .unwrap();

            assert!(outcome.status.success());
            let fetch_index = events
                .iter()
                .position(|event| {
                    matches!(event, SyncEvent::StatusChanged(SyncStatus::FetchingRemotes))
                })
                .unwrap();
            let repair_index = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        SyncEvent::StatusChanged(SyncStatus::RepairingClosedPullRequests)
                    )
                })
                .unwrap();
            let prune_index = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        SyncEvent::StatusChanged(SyncStatus::RemovingMergedLocalBranches)
                    )
                })
                .unwrap();
            let stage_index = events
                .iter()
                .position(|event| {
                    matches!(event, SyncEvent::StageStarted(SyncStage::LocalSync { .. }))
                })
                .unwrap();

            assert!(fetch_index < repair_index);
            assert!(repair_index < prune_index);
            assert!(prune_index < stage_index);
        });
    }

    #[test]
    fn reports_pull_request_base_updates_before_each_retarget() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);

            let bin_dir = repo.join("fake-bin");
            let log_path = repo.join("gh.log");
            #[cfg(unix)]
            let script = format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"{}\"\n",
                log_path.display()
            );
            #[cfg(windows)]
            let script = format!("@echo off\r\necho %* >> \"{}\"\r\n", log_path.display());
            install_fake_executable(&bin_dir, "gh", &script);
            fs::write(&log_path, "").unwrap();
            let _path_guard = EnvVarGuard::set("PATH", path_with_prepend(&bin_dir));
            // On Windows, Command::new("gh") only resolves gh.exe, not gh.cmd.
            // Point DAGGER_GH_BIN at the .cmd wrapper so gh_program() uses it directly.
            #[cfg(windows)]
            let _gh_bin_guard = EnvVarGuard::set(
                "DAGGER_GH_BIN",
                bin_dir.join("gh.cmd").display().to_string(),
            );

            let plan = PullRequestUpdatePlan {
                actions: vec![
                    super::PullRequestUpdateAction {
                        branch_name: "feat/auth".into(),
                        pull_request_number: 42,
                        new_base_branch_name: "main".into(),
                    },
                    super::PullRequestUpdateAction {
                        branch_name: "feat/auth-ui".into(),
                        pull_request_number: 43,
                        new_base_branch_name: "feat/auth".into(),
                    },
                ],
            };
            let mut statuses = Vec::new();

            let updated = execute_pull_request_update_plan_with_reporter(&plan, &mut |status| {
                statuses.push(status);
                Ok(())
            })
            .unwrap();

            assert_eq!(updated, plan.actions);
            assert_eq!(
                statuses,
                vec![
                    SyncStatus::UpdatingPullRequestBase {
                        branch_name: "feat/auth".into(),
                        pull_request_number: 42,
                    },
                    SyncStatus::UpdatingPullRequestBase {
                        branch_name: "feat/auth-ui".into(),
                        pull_request_number: 43,
                    },
                ]
            );

            let log = fs::read_to_string(log_path).unwrap();
            assert!(log.contains("pr edit 42 --base main"));
            assert!(log.contains("pr edit 43 --base feat/auth"));
        });
    }

    #[test]
    fn reports_remote_push_status_before_each_push() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            initialize_origin_remote(repo);
            crate::core::init::run(&crate::core::init::InitOptions::default()).unwrap();
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-ui");
            commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");

            let plan = RemotePushPlan {
                actions: vec![
                    super::RemotePushAction {
                        target: BranchPushTarget {
                            remote_name: "origin".into(),
                            branch_name: "feat/auth".into(),
                        },
                        kind: RemotePushActionKind::CreateRemoteBranch,
                    },
                    super::RemotePushAction {
                        target: BranchPushTarget {
                            remote_name: "origin".into(),
                            branch_name: "feat/auth-ui".into(),
                        },
                        kind: RemotePushActionKind::CreateRemoteBranch,
                    },
                ],
            };
            let mut statuses = Vec::new();

            let outcome = execute_remote_push_plan_with_reporter(&plan, &mut |status| {
                statuses.push(status);
                Ok(())
            })
            .unwrap();

            assert!(outcome.status.success());
            assert_eq!(outcome.pushed_actions, plan.actions);
            assert_eq!(
                statuses,
                vec![
                    SyncStatus::PushingRemoteBranch {
                        branch_name: "feat/auth".into(),
                        remote_name: "origin".into(),
                        kind: RemotePushActionKind::CreateRemoteBranch,
                    },
                    SyncStatus::PushingRemoteBranch {
                        branch_name: "feat/auth-ui".into(),
                        remote_name: "origin".into(),
                        kind: RemotePushActionKind::CreateRemoteBranch,
                    },
                ]
            );
            assert!(
                git_output(repo, &["ls-remote", "--heads", "origin", "feat/auth"])
                    .contains("refs/heads/feat/auth")
            );
            assert!(
                git_output(repo, &["ls-remote", "--heads", "origin", "feat/auth-ui"])
                    .contains("refs/heads/feat/auth-ui")
            );
        });
    }

    #[test]
    fn emits_local_sync_restack_events_for_outdated_branch_restack() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            crate::core::init::run(&crate::core::init::InitOptions::default()).unwrap();
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-ui");
            commit_file(repo, "ui.txt", "ui\n", "feat: ui");
            git_ok(repo, &["checkout", "feat/auth"]);
            append_file(repo, "auth.txt", "more\n", "feat: parent follow-up");
            git_ok(repo, &["checkout", "main"]);

            let mut events = Vec::new();
            let outcome = run_with_reporter(&SyncOptions::default(), &mut |event| {
                events.push(event.clone());
                Ok(())
            })
            .unwrap();

            assert!(outcome.status.success());
            assert!(matches!(
                &events[0],
                SyncEvent::StatusChanged(SyncStatus::FetchingRemotes)
            ));
            assert!(matches!(
                &events[1],
                SyncEvent::StatusChanged(SyncStatus::PreparingRestack {
                    step_branch_name,
                }) if step_branch_name == "feat/auth-ui"
            ));
            assert!(matches!(
                &events[2],
                SyncEvent::StageStarted(SyncStage::LocalSync {
                    step_branch_name,
                    active_branch_name,
                    ..
                }) if step_branch_name == "feat/auth-ui" && active_branch_name == "feat/auth-ui"
            ));
            assert!(events.iter().any(|event| matches!(
                event,
                SyncEvent::StatusChanged(SyncStatus::RestackingBranch {
                    branch_name,
                    onto_branch,
                }) if branch_name == "feat/auth-ui" && onto_branch == "feat/auth"
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                SyncEvent::RestackStarted {
                    branch_name,
                    onto_branch,
                } if branch_name == "feat/auth-ui" && onto_branch == "feat/auth"
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                SyncEvent::RestackProgress {
                    branch_name,
                    current_commit,
                    total_commits,
                    ..
                } if branch_name == "feat/auth-ui" && *current_commit == 1 && *total_commits == 1
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                SyncEvent::RestackCompleted {
                    branch_name,
                    onto_branch,
                } if branch_name == "feat/auth-ui" && onto_branch == "feat/auth"
            )));
        });
    }

    #[test]
    fn archives_deleted_local_branch_before_descendant_restack_events() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            crate::core::init::run(&crate::core::init::InitOptions::default()).unwrap();
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-api");
            commit_file(repo, "api.txt", "api\n", "feat: api");
            create_tracked_branch("feat/auth-api-tests");
            commit_file(repo, "tests.txt", "tests\n", "feat: tests");
            git_ok(repo, &["checkout", "feat/auth"]);
            git_ok(repo, &["branch", "-D", "feat/auth-api"]);

            let mut events = Vec::new();
            let outcome = run_with_reporter(&SyncOptions::default(), &mut |event| {
                events.push(event.clone());
                Ok(())
            })
            .unwrap();

            assert!(outcome.status.success());
            let archive_index = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        SyncEvent::BranchArchived { branch_name } if branch_name == "feat/auth-api"
                    )
                })
                .unwrap();
            let restack_index = events
                .iter()
                .position(|event| {
                    matches!(
                        event,
                        SyncEvent::RestackStarted { branch_name, .. }
                            if branch_name == "feat/auth-api-tests"
                    )
                })
                .unwrap();

            assert!(archive_index < restack_index);
        });
    }

    #[test]
    fn resumes_sync_with_completed_snapshot_and_active_branch() {
        with_temp_repo("dgr-sync-core", |repo| {
            initialize_main_repo(repo);
            crate::core::init::run(&crate::core::init::InitOptions::default()).unwrap();
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-ui");
            commit_file(repo, "shared.txt", "child\n", "feat: ui");
            git_ok(repo, &["checkout", "main"]);
            commit_file(repo, "shared.txt", "main\n", "feat: trunk");
            git_ok(repo, &["checkout", "feat/auth"]);

            let paused = run(&SyncOptions::default()).unwrap();
            assert!(!paused.status.success());

            std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
            git_ok(repo, &["add", "shared.txt"]);

            let mut events = Vec::new();
            let outcome = run_with_reporter(
                &SyncOptions {
                    continue_operation: true,
                },
                &mut |event| {
                    events.push(event.clone());
                    Ok(())
                },
            )
            .unwrap();

            assert!(outcome.status.success());
            assert!(paused.paused);
            assert!(matches!(
                &events[0],
                SyncEvent::StatusChanged(SyncStatus::PreparingRestack {
                    step_branch_name,
                }) if step_branch_name == "feat/auth"
            ));
            assert!(matches!(
                &events[1],
                SyncEvent::StageStarted(SyncStage::LocalSync {
                    active_branch_name,
                    restacked_branches,
                    ..
                }) if active_branch_name == "feat/auth-ui"
                    && restacked_branches
                        .iter()
                        .any(|branch| branch.branch_name == "feat/auth")
            ));
        });
    }
}
