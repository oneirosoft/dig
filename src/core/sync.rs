use std::io;
use std::process::ExitStatus;

use crate::core::store::{clear_operation, load_operation, open_initialized};
use crate::core::{adopt, clean, commit, git, merge, orphan};

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
}

#[derive(Debug)]
pub struct SyncOutcome {
    pub status: ExitStatus,
    pub completion: Option<SyncCompletion>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

pub fn run(options: &SyncOptions) -> io::Result<SyncOutcome> {
    if !options.continue_operation {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dig sync is not implemented yet; use 'dig sync --continue' only when resuming a paused restack",
        ));
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
    }
}
