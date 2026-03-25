use std::io;
use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::branch;
use crate::core::git;
use crate::core::restack::RestackAction;
use crate::core::store::{
    BranchNode, ParentRef, PendingAdoptOperation, PendingOperationKind, PendingOperationState,
    now_unix_timestamp_secs, open_initialized, open_or_initialize, record_branch_adopted,
};
use crate::core::workflow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptOptions {
    pub branch_name: Option<String>,
    pub parent_branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptPlan {
    pub trunk_branch: String,
    pub original_branch: String,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub parent: ParentRef,
    pub old_upstream_oid: String,
    pub requires_rebase: bool,
}

#[derive(Debug)]
pub struct AdoptOutcome {
    pub status: ExitStatus,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub restacked: bool,
    pub restored_original_branch: Option<String>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

pub fn plan(options: &AdoptOptions) -> io::Result<AdoptPlan> {
    let original_branch = git::current_branch_name()?;
    let (session, _) = open_or_initialize(&original_branch)?;
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
                "cannot adopt trunk branch '{}'",
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

    if session.state.find_branch_by_name(&branch_name).is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("branch '{}' is already tracked by dig", branch_name),
        ));
    }

    if !git::branch_exists(parent_branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("parent branch '{}' does not exist", parent_branch_name),
        ));
    }

    let parent = branch::resolve_parent_ref(&session.state, &session.config, parent_branch_name)?;
    let old_upstream_oid = git::merge_base(parent_branch_name, &branch_name)?;
    let parent_head_oid = git::ref_oid(parent_branch_name)?;

    Ok(AdoptPlan {
        trunk_branch: session.config.trunk_branch,
        original_branch,
        branch_name,
        parent_branch_name: parent_branch_name.to_string(),
        parent,
        old_upstream_oid: old_upstream_oid.clone(),
        requires_rebase: old_upstream_oid != parent_head_oid,
    })
}

pub fn apply(plan: &AdoptPlan) -> io::Result<AdoptOutcome> {
    let mut session = open_initialized("dig is not initialized")?;
    workflow::ensure_ready_for_operation(&session.repo, "adopt")?;
    workflow::ensure_no_pending_operation(&session.paths, "adopt")?;

    if session
        .state
        .find_branch_by_name(&plan.branch_name)
        .is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("branch '{}' is already tracked by dig", plan.branch_name),
        ));
    }

    let resolved_parent =
        branch::resolve_parent_ref(&session.state, &session.config, &plan.parent_branch_name)?;
    if resolved_parent != plan.parent {
        return Err(io::Error::other(format!(
            "tracked parent for '{}' changed while planning adopt",
            plan.parent_branch_name
        )));
    }

    let mut status = git::success_status()?;
    let mut restacked = false;

    if plan.requires_rebase {
        let rebase_outcome = workflow::execute_resumable_restack_operation(
            &mut session,
            PendingOperationKind::Adopt(PendingAdoptOperation {
                original_branch: plan.original_branch.clone(),
                branch_name: plan.branch_name.clone(),
                parent_branch_name: plan.parent_branch_name.clone(),
                parent: plan.parent.clone(),
            }),
            &[RestackAction {
                node_id: Uuid::nil(),
                branch_name: plan.branch_name.clone(),
                old_upstream_branch_name: plan.parent_branch_name.clone(),
                old_upstream_oid: plan.old_upstream_oid.clone(),
                new_base_branch_name: plan.parent_branch_name.clone(),
                new_parent: None,
            }],
            &mut |_| Ok(()),
        )?;
        status = rebase_outcome.status;

        if rebase_outcome.paused {
            return Ok(AdoptOutcome {
                status,
                branch_name: plan.branch_name.clone(),
                parent_branch_name: plan.parent_branch_name.clone(),
                restacked: false,
                restored_original_branch: None,
                failure_output: rebase_outcome.failure_output,
                paused: true,
            });
        }

        restacked = true;
    }

    let parent_head_oid = git::ref_oid(&plan.parent_branch_name)?;
    let branch_head_oid = git::ref_oid(&plan.branch_name)?;
    let adopted_node = BranchNode {
        id: Uuid::new_v4(),
        branch_name: plan.branch_name.clone(),
        parent: plan.parent.clone(),
        base_ref: plan.parent_branch_name.clone(),
        fork_point_oid: parent_head_oid,
        head_oid_at_creation: branch_head_oid,
        created_at_unix_secs: now_unix_timestamp_secs(),
        archived: false,
    };

    record_branch_adopted(&mut session, adopted_node)?;

    let restored_original_branch = restore_original_branch_if_needed(&plan.original_branch)?;

    Ok(AdoptOutcome {
        status,
        branch_name: plan.branch_name.clone(),
        parent_branch_name: plan.parent_branch_name.clone(),
        restacked,
        restored_original_branch,
        failure_output: None,
        paused: false,
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

fn restore_original_branch_if_needed(original_branch: &str) -> io::Result<Option<String>> {
    if let Some(outcome) = workflow::restore_original_branch_if_needed(original_branch)? {
        if !outcome.status.success() {
            return Err(io::Error::other(format!(
                "adopt completed, but failed to return to '{}'",
                original_branch
            )));
        }

        return Ok(Some(outcome.restored_branch));
    }

    Ok(None)
}

pub(crate) fn resume_after_sync(
    pending_operation: PendingOperationState,
    payload: PendingAdoptOperation,
) -> io::Result<AdoptOutcome> {
    let mut session = open_initialized("dig is not initialized")?;
    let rebase_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;

    if rebase_outcome.paused {
        return Ok(AdoptOutcome {
            status: rebase_outcome.status,
            branch_name: payload.branch_name,
            parent_branch_name: payload.parent_branch_name,
            restacked: false,
            restored_original_branch: None,
            failure_output: rebase_outcome.failure_output,
            paused: true,
        });
    }

    let parent_head_oid = git::ref_oid(&payload.parent_branch_name)?;
    let branch_head_oid = git::ref_oid(&payload.branch_name)?;
    let adopted_node = BranchNode {
        id: Uuid::new_v4(),
        branch_name: payload.branch_name.clone(),
        parent: payload.parent.clone(),
        base_ref: payload.parent_branch_name.clone(),
        fork_point_oid: parent_head_oid,
        head_oid_at_creation: branch_head_oid,
        created_at_unix_secs: now_unix_timestamp_secs(),
        archived: false,
    };

    record_branch_adopted(&mut session, adopted_node)?;
    let restored_original_branch = restore_original_branch_if_needed(&payload.original_branch)?;

    Ok(AdoptOutcome {
        status: rebase_outcome.status,
        branch_name: payload.branch_name,
        parent_branch_name: payload.parent_branch_name,
        restacked: true,
        restored_original_branch,
        failure_output: None,
        paused: false,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use super::{AdoptOptions, apply, plan, resolve_branch_name};
    use crate::core::git;
    use crate::core::store::{DigEvent, ParentRef, dig_paths, load_state};
    use crate::core::test_support::{
        commit_file, create_tracked_branch, git_ok, initialize_main_repo, with_temp_repo,
    };

    #[test]
    fn resolves_requested_branch_or_current_branch() {
        assert_eq!(
            resolve_branch_name("feat/current", Some("feat/other")).unwrap(),
            "feat/other"
        );
        assert_eq!(
            resolve_branch_name("feat/current", None).unwrap(),
            "feat/current"
        );
    }

    #[test]
    fn rejects_tracked_branch_adoption() {
        with_temp_repo("dig-adopt", |repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");

            let error = plan(&AdoptOptions {
                branch_name: Some("feat/auth".into()),
                parent_branch_name: "main".into(),
            })
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(
                error.to_string(),
                "branch 'feat/auth' is already tracked by dig"
            );
        });
    }

    #[test]
    fn rejects_adopting_trunk_branch() {
        with_temp_repo("dig-adopt", |repo| {
            initialize_main_repo(repo);

            let error = plan(&AdoptOptions {
                branch_name: Some("main".into()),
                parent_branch_name: "main".into(),
            })
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(error.to_string(), "cannot adopt trunk branch 'main'");
        });
    }

    #[test]
    fn rejects_untracked_parent_branch() {
        with_temp_repo("dig-adopt", |repo| {
            initialize_main_repo(repo);
            git_ok(repo, &["checkout", "-b", "feat/child"]);
            git_ok(repo, &["checkout", "main"]);

            let error = plan(&AdoptOptions {
                branch_name: Some("feat/child".into()),
                parent_branch_name: "feat/parent".into(),
            })
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::NotFound);
            assert_eq!(
                error.to_string(),
                "parent branch 'feat/parent' does not exist"
            );
        });
    }

    #[test]
    fn plans_rebase_for_sibling_branch_adoption() {
        with_temp_repo("dig-adopt", |repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["checkout", "main"]);
            git_ok(repo, &["checkout", "-b", "feat/auth-ui"]);
            commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
            git_ok(repo, &["checkout", "feat/auth"]);

            let plan = plan(&AdoptOptions {
                branch_name: Some("feat/auth-ui".into()),
                parent_branch_name: "feat/auth".into(),
            })
            .unwrap();

            assert!(plan.requires_rebase);
            assert_eq!(plan.original_branch, "feat/auth");
            assert_eq!(plan.branch_name, "feat/auth-ui");
            assert_eq!(plan.parent_branch_name, "feat/auth");
        });
    }

    #[test]
    fn adopts_branch_and_records_post_adopt_metadata() {
        with_temp_repo("dig-adopt", |repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            git_ok(repo, &["checkout", "main"]);
            git_ok(repo, &["checkout", "-b", "feat/auth-ui"]);
            commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
            git_ok(repo, &["checkout", "feat/auth"]);

            let plan = plan(&AdoptOptions {
                branch_name: Some("feat/auth-ui".into()),
                parent_branch_name: "feat/auth".into(),
            })
            .unwrap();
            let outcome = apply(&plan).unwrap();

            assert!(outcome.status.success());
            assert!(outcome.restacked);
            assert_eq!(
                outcome.restored_original_branch.as_deref(),
                Some("feat/auth")
            );
            assert_eq!(git::current_branch_name().unwrap(), "feat/auth");

            let repo_context = git::resolve_repo_context().unwrap();
            let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
            let parent = state.find_branch_by_name("feat/auth").unwrap();
            let adopted = state.find_branch_by_name("feat/auth-ui").unwrap();

            assert_eq!(adopted.parent, ParentRef::Branch { node_id: parent.id });
            assert_eq!(adopted.base_ref, "feat/auth");
            assert_eq!(adopted.fork_point_oid, git::ref_oid("feat/auth").unwrap());
            assert_eq!(
                adopted.head_oid_at_creation,
                git::ref_oid("feat/auth-ui").unwrap()
            );

            let events =
                fs::read_to_string(repo_context.git_dir.join("dig/events.ndjson")).unwrap();
            assert!(events.lines().any(|line| {
                serde_json::from_str::<DigEvent>(line)
                    .map(|event| matches!(event, DigEvent::BranchAdopted(_)))
                    .unwrap_or(false)
            }));
        });
    }
}
