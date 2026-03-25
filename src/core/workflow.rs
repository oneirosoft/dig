use std::io;
use std::process::ExitStatus;

use crate::core::git::{self, RebaseProgress, RepoContext};
use crate::core::restack::{self, RestackAction, RestackPreview};
use crate::core::store::fs::DigPaths;
use crate::core::store::session::StoreSession;
use crate::core::store::{
    PendingOperationKind, PendingOperationState, clear_operation, load_operation,
    record_branch_reparented, save_operation,
};

#[derive(Debug)]
pub(crate) struct CheckoutBranchOutcome {
    pub status: ExitStatus,
    pub switched_from: Option<String>,
}

#[derive(Debug)]
pub(crate) struct RestoreBranchOutcome {
    pub status: ExitStatus,
    pub restored_branch: String,
}

#[derive(Debug)]
pub(crate) struct ResumableRestackExecutionOutcome {
    pub status: ExitStatus,
    pub restacked_branches: Vec<RestackPreview>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RestackExecutionEvent<'a> {
    Started(&'a RestackAction),
    Progress {
        action: &'a RestackAction,
        progress: RebaseProgress,
    },
    Completed(&'a RestackAction),
}

pub(crate) fn ensure_ready_for_operation(repo: &RepoContext, command_name: &str) -> io::Result<()> {
    git::ensure_clean_worktree(command_name)?;
    git::ensure_no_in_progress_operations(repo, command_name)
}

pub(crate) fn ensure_no_pending_operation(paths: &DigPaths, command_name: &str) -> io::Result<()> {
    let Some(operation) = load_operation(paths)? else {
        return Ok(());
    };

    Err(io::Error::other(format!(
        "dig {command_name} cannot run while a dig {} operation is paused; run 'dig sync --continue'",
        operation.origin.command_name()
    )))
}

pub(crate) fn checkout_branch_if_needed(target_branch: &str) -> io::Result<CheckoutBranchOutcome> {
    let current_branch = git::current_branch_name_if_any()?;
    if current_branch.as_deref() == Some(target_branch) {
        return Ok(CheckoutBranchOutcome {
            status: git::success_status()?,
            switched_from: None,
        });
    }

    let status = git::switch_branch(target_branch)?;

    Ok(CheckoutBranchOutcome {
        switched_from: status.success().then_some(current_branch).flatten(),
        status,
    })
}

pub(crate) fn restore_original_branch_if_needed(
    original_branch: &str,
) -> io::Result<Option<RestoreBranchOutcome>> {
    let current_branch = git::current_branch_name_if_any()?;
    if current_branch.as_deref() == Some(original_branch) {
        return Ok(None);
    }

    if !git::branch_exists(original_branch)? {
        return Ok(None);
    }

    let status = git::switch_branch(original_branch)?;

    Ok(Some(RestoreBranchOutcome {
        status,
        restored_branch: original_branch.to_string(),
    }))
}

pub(crate) fn execute_resumable_restack_operation<F>(
    session: &mut StoreSession,
    origin: PendingOperationKind,
    actions: &[RestackAction],
    on_event: &mut F,
) -> io::Result<ResumableRestackExecutionOutcome>
where
    F: for<'a> FnMut(RestackExecutionEvent<'a>) -> io::Result<()>,
{
    let Some(mut pending_operation) = PendingOperationState::start(origin, actions.to_vec()) else {
        clear_operation(&session.paths)?;

        return Ok(ResumableRestackExecutionOutcome {
            status: git::success_status()?,
            restacked_branches: Vec::new(),
            failure_output: None,
            paused: false,
        });
    };

    run_pending_restack_operation(session, &mut pending_operation, on_event)
}

pub(crate) fn continue_resumable_restack_operation<F>(
    session: &mut StoreSession,
    pending_operation: PendingOperationState,
    on_event: &mut F,
) -> io::Result<ResumableRestackExecutionOutcome>
where
    F: for<'a> FnMut(RestackExecutionEvent<'a>) -> io::Result<()>,
{
    let action = pending_operation.active_action().clone();
    on_event(RestackExecutionEvent::Completed(&action))?;

    if let Some(parent_change) = restack::finalize_action(&mut session.state, &action)? {
        record_branch_reparented(
            session,
            parent_change.branch_id,
            parent_change.branch_name,
            parent_change.old_parent,
            parent_change.new_parent,
            parent_change.old_base_ref,
            parent_change.new_base_ref,
        )?;
    }

    let mut completed_branches = pending_operation.completed_branches().to_vec();
    let (preview, next_pending_operation) = pending_operation.advance_after_success();
    completed_branches.push(preview);

    match next_pending_operation {
        Some(mut pending_operation) => {
            run_pending_restack_operation(session, &mut pending_operation, on_event)
        }
        None => {
            clear_operation(&session.paths)?;

            Ok(ResumableRestackExecutionOutcome {
                status: git::success_status()?,
                restacked_branches: completed_branches,
                failure_output: None,
                paused: false,
            })
        }
    }
}

fn run_pending_restack_operation<F>(
    session: &mut StoreSession,
    pending_operation: &mut PendingOperationState,
    on_event: &mut F,
) -> io::Result<ResumableRestackExecutionOutcome>
where
    F: for<'a> FnMut(RestackExecutionEvent<'a>) -> io::Result<()>,
{
    loop {
        save_operation(&session.paths, pending_operation)?;

        let action = pending_operation.active_action().clone();
        on_event(RestackExecutionEvent::Started(&action))?;

        let outcome = restack::apply_action(&mut session.state, &action, |progress| {
            on_event(RestackExecutionEvent::Progress {
                action: &action,
                progress,
            })
        })?;

        if !outcome.status.success() {
            return Ok(ResumableRestackExecutionOutcome {
                status: outcome.status,
                restacked_branches: pending_operation.completed_branches().to_vec(),
                failure_output: Some(outcome.stderr),
                paused: true,
            });
        }

        on_event(RestackExecutionEvent::Completed(&action))?;

        if let Some(parent_change) = outcome.parent_change {
            record_branch_reparented(
                session,
                parent_change.branch_id,
                parent_change.branch_name,
                parent_change.old_parent,
                parent_change.new_parent,
                parent_change.old_base_ref,
                parent_change.new_base_ref,
            )?;
        }

        let mut completed_branches = pending_operation.completed_branches().to_vec();
        let (preview, next_pending_operation) = pending_operation.clone().advance_after_success();
        completed_branches.push(preview);

        match next_pending_operation {
            Some(next_pending_operation) => {
                *pending_operation = next_pending_operation;
                save_operation(&session.paths, pending_operation)?;
            }
            None => {
                clear_operation(&session.paths)?;

                return Ok(ResumableRestackExecutionOutcome {
                    status: outcome.status,
                    restacked_branches: completed_branches,
                    failure_output: None,
                    paused: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::execute_resumable_restack_operation;
    use crate::core::git;
    use crate::core::restack;
    use crate::core::store::{
        PendingCommitOperation, PendingOperationKind, dig_paths, load_operation, open_initialized,
    };
    use crate::core::test_support::{
        commit_file, create_tracked_branch, git_ok, initialize_main_repo, with_temp_repo,
    };

    #[test]
    fn persists_pending_operation_when_restack_conflicts() {
        with_temp_repo("dig-workflow", |repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "shared.txt", "base\n", "feat: auth");
            create_tracked_branch("feat/auth-ui");
            commit_file(repo, "shared.txt", "child\n", "feat: child change");
            git_ok(repo, &["checkout", "feat/auth"]);

            let old_head = git::ref_oid("HEAD").unwrap();
            let state_before = fs::read_to_string(repo.join(".git/dig/state.json")).unwrap();
            let events_before = fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap();

            commit_file(repo, "shared.txt", "parent\n", "feat: parent follow-up");

            let repo_context = git::resolve_repo_context().unwrap();
            let paths = dig_paths(&repo_context.git_dir);
            let state = crate::core::store::load_state(&paths).unwrap();
            let node = state.find_branch_by_name("feat/auth").unwrap();
            let actions =
                restack::plan_after_branch_advance(&state, node.id, &node.branch_name, &old_head)
                    .unwrap();
            let mut session = open_initialized("dig is not initialized").unwrap();

            let outcome = execute_resumable_restack_operation(
                &mut session,
                PendingOperationKind::Commit(PendingCommitOperation {
                    current_branch: "feat/auth".into(),
                    summary_line: Some("1 file changed".into()),
                    recent_commits: Vec::new(),
                }),
                &actions,
                &mut |_| Ok(()),
            )
            .unwrap();

            assert!(outcome.paused);
            assert!(!outcome.status.success());
            assert!(outcome.restacked_branches.is_empty());
            assert!(
                outcome
                    .failure_output
                    .as_deref()
                    .unwrap()
                    .contains("could not apply")
            );

            let pending_operation = load_operation(&paths).unwrap().unwrap();
            assert_eq!(pending_operation.origin.command_name(), "commit");
            assert_eq!(
                pending_operation.active_action().branch_name,
                "feat/auth-ui"
            );
            assert_eq!(
                fs::read_to_string(repo.join(".git/dig/state.json")).unwrap(),
                state_before
            );
            assert_eq!(
                fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
                events_before
            );
        });
    }
}
