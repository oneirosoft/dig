use std::fs;
use std::io;
use std::path::Path;

use crate::core::git::{self, CommitMetadata};
use crate::core::restack;
use crate::core::store::{
    BranchArchiveReason, PendingMergeOperation, PendingOperationKind, PendingOperationState,
    open_initialized, record_branch_archived,
};
use crate::core::workflow::{self, RestackExecutionEvent};

use super::types::{
    DeleteMergedBranchOutcome, MergeEvent, MergeMode, MergeOutcome, MergePlan, MergeResumeOutcome,
};

pub(crate) fn apply(plan: &MergePlan) -> io::Result<MergeOutcome> {
    apply_with_reporter(plan, &mut |_| Ok(()))
}

pub(crate) fn apply_with_reporter<F>(plan: &MergePlan, reporter: &mut F) -> io::Result<MergeOutcome>
where
    F: FnMut(MergeEvent) -> io::Result<()>,
{
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_ready_for_operation(&session.repo, "merge")?;
    workflow::ensure_no_pending_operation(&session.paths, "merge")?;
    let current_branch = git::current_branch_name()?;
    let source_commits = if plan.mode == MergeMode::Squash {
        git::commit_metadata_in_range(&format!(
            "{}..{}",
            plan.target_branch_name, plan.source_branch_name
        ))?
    } else {
        Vec::new()
    };

    let mut switched_to_target_from = None;
    if current_branch != plan.target_branch_name {
        reporter(MergeEvent::SwitchingToTarget {
            from_branch: current_branch.clone(),
            to_branch: plan.target_branch_name.clone(),
        })?;
        let checkout = workflow::checkout_branch_if_needed(&plan.target_branch_name)?;
        if !checkout.status.success() {
            return Ok(MergeOutcome {
                status: checkout.status,
                switched_to_target_from: None,
                restacked_branches: Vec::new(),
                failure_output: None,
                paused: false,
            });
        }

        reporter(MergeEvent::SwitchedToTarget {
            from_branch: current_branch.clone(),
            to_branch: plan.target_branch_name.clone(),
        })?;
        switched_to_target_from = checkout.switched_from;
    }

    reporter(MergeEvent::MergeStarted {
        source_branch: plan.source_branch_name.clone(),
        target_branch: plan.target_branch_name.clone(),
        mode: plan.mode,
    })?;

    let merge_output = match plan.mode {
        MergeMode::Normal => git::merge_branch(&plan.source_branch_name)?,
        MergeMode::Squash => run_squash_merge(plan, &session.repo.git_dir, &source_commits)?,
    };

    if !merge_output.status.success() {
        return Ok(MergeOutcome {
            status: merge_output.status,
            switched_to_target_from,
            restacked_branches: Vec::new(),
            failure_output: Some(merge_output.combined_output()),
            paused: false,
        });
    }

    reporter(MergeEvent::MergeCompleted {
        source_branch: plan.source_branch_name.clone(),
        target_branch: plan.target_branch_name.clone(),
        mode: plan.mode,
    })?;

    let node = session
        .state
        .find_branch_by_id(plan.source_node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;
    let restack_actions = restack::plan_after_branch_detach(
        &session.state,
        node.id,
        &node.branch_name,
        &plan.target_branch_name,
        &node.parent,
    )?;

    let restack_outcome = workflow::execute_resumable_restack_operation(
        &mut session,
        PendingOperationKind::Merge(PendingMergeOperation {
            trunk_branch: plan.trunk_branch.clone(),
            source_branch_name: plan.source_branch_name.clone(),
            target_branch_name: plan.target_branch_name.clone(),
            source_node_id: plan.source_node_id,
            switched_to_target_from: switched_to_target_from.clone(),
        }),
        &restack_actions,
        &mut |event| match event {
            RestackExecutionEvent::Started(action) => reporter(MergeEvent::RebaseStarted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base_branch_name.clone(),
            }),
            RestackExecutionEvent::Progress { action, progress } => {
                reporter(MergeEvent::RebaseProgress {
                    branch_name: action.branch_name.clone(),
                    onto_branch: action.new_base_branch_name.clone(),
                    current_commit: progress.current,
                    total_commits: progress.total,
                })
            }
            RestackExecutionEvent::Completed(action) => reporter(MergeEvent::RebaseCompleted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base_branch_name.clone(),
            }),
        },
    )?;
    if restack_outcome.paused {
        return Ok(MergeOutcome {
            status: restack_outcome.status,
            switched_to_target_from,
            restacked_branches: restack_outcome.restacked_branches,
            failure_output: restack_outcome.failure_output,
            paused: true,
        });
    }
    let mut last_status = restack_outcome.status;
    let restacked_branches = restack_outcome.restacked_branches;

    let checkout = workflow::checkout_branch_if_needed(&plan.target_branch_name)?;
    if checkout.switched_from.is_some() {
        if !checkout.status.success() {
            return Ok(MergeOutcome {
                status: checkout.status,
                switched_to_target_from,
                restacked_branches,
                failure_output: None,
                paused: false,
            });
        }

        last_status = checkout.status;
    }

    Ok(MergeOutcome {
        status: last_status,
        switched_to_target_from,
        restacked_branches,
        failure_output: None,
        paused: false,
    })
}

pub(crate) fn delete_merged_branch(plan: &MergePlan) -> io::Result<DeleteMergedBranchOutcome> {
    delete_merged_branch_by_id(plan.source_node_id, &plan.target_branch_name)
}

pub(crate) fn delete_merged_branch_by_id(
    source_node_id: uuid::Uuid,
    target_branch_name: &str,
) -> io::Result<DeleteMergedBranchOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let node = session
        .state
        .find_branch_by_id(source_node_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found"))?;

    let status = git::delete_branch_force(&node.branch_name)?;
    if !status.success() {
        return Ok(DeleteMergedBranchOutcome {
            status,
            deleted_branch_name: None,
        });
    }

    record_branch_archived(
        &mut session,
        node.id,
        node.branch_name.clone(),
        BranchArchiveReason::IntegratedIntoParent {
            parent_branch: target_branch_name.to_string(),
        },
    )?;

    Ok(DeleteMergedBranchOutcome {
        status,
        deleted_branch_name: Some(node.branch_name),
    })
}

pub(crate) fn resume_after_sync(
    pending_operation: PendingOperationState,
    payload: PendingMergeOperation,
) -> io::Result<MergeResumeOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;

    if restack_outcome.paused {
        return Ok(MergeResumeOutcome {
            trunk_branch: payload.trunk_branch,
            source_branch_name: payload.source_branch_name,
            target_branch_name: payload.target_branch_name,
            source_node_id: payload.source_node_id,
            outcome: MergeOutcome {
                status: restack_outcome.status,
                switched_to_target_from: payload.switched_to_target_from,
                restacked_branches: restack_outcome.restacked_branches,
                failure_output: restack_outcome.failure_output,
                paused: true,
            },
        });
    }

    let checkout = workflow::checkout_branch_if_needed(&payload.target_branch_name)?;
    let status = if checkout.switched_from.is_some() {
        checkout.status
    } else {
        restack_outcome.status
    };
    let target_branch_name = payload.target_branch_name.clone();

    Ok(MergeResumeOutcome {
        trunk_branch: payload.trunk_branch,
        source_branch_name: payload.source_branch_name,
        target_branch_name,
        source_node_id: payload.source_node_id,
        outcome: MergeOutcome {
            status,
            switched_to_target_from: payload.switched_to_target_from,
            restacked_branches: restack_outcome.restacked_branches,
            failure_output: (!status.success()).then(|| {
                format!(
                    "merge completed, but failed to return to '{}'",
                    payload.target_branch_name
                )
            }),
            paused: false,
        },
    })
}

fn run_squash_merge(
    plan: &MergePlan,
    git_dir: &Path,
    source_commits: &[CommitMetadata],
) -> io::Result<git::GitCommandOutput> {
    let merge_output = git::squash_merge_branch(&plan.source_branch_name)?;
    if !merge_output.status.success() {
        return Ok(merge_output);
    }

    if !git::has_staged_changes()? {
        return Ok(merge_output);
    }

    let message = build_squash_commit_message(
        &plan.source_branch_name,
        &plan.target_branch_name,
        &plan.messages,
        source_commits,
    );
    let message_path = git_dir.join("DIG_MERGE_MSG");
    fs::write(&message_path, message)?;
    let commit_output = git::commit_with_message_file(&message_path);
    let remove_result = fs::remove_file(&message_path);
    let commit_output = commit_output?;
    if let Err(err) = remove_result {
        return Err(err);
    }

    Ok(commit_output)
}

pub(super) fn build_squash_commit_message(
    source_branch_name: &str,
    target_branch_name: &str,
    messages: &[String],
    source_commits: &[CommitMetadata],
) -> String {
    let mut sections = Vec::new();

    if messages.is_empty() {
        sections.push(format!(
            "merge {} into {}",
            source_branch_name, target_branch_name
        ));
    } else {
        sections.push(messages.join("\n\n"));
    }

    if !source_commits.is_empty() {
        let commit_listing = source_commits
            .iter()
            .map(|commit| format!("commit {}\n    {}", commit.sha, commit.subject))
            .collect::<Vec<_>>()
            .join("\n\n");
        sections.push(commit_listing);
    }

    sections.join("\n\n")
}
