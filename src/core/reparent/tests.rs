use std::io;

use super::{ReparentOptions, apply, plan};
use crate::core::git;
use crate::core::store::{ParentRef, dig_paths, load_state};
use crate::core::test_support::{
    commit_file, create_tracked_branch, git_ok, initialize_main_repo, with_temp_repo,
};

#[test]
fn defaults_to_current_branch_when_name_is_omitted() {
    with_temp_repo("dig-reparent", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        create_tracked_branch("feat/auth-ui");

        let plan = plan(&ReparentOptions {
            branch_name: None,
            parent_branch_name: "main".into(),
        })
        .unwrap();

        assert_eq!(plan.branch_name, "feat/auth-ui");
        assert_eq!(plan.parent_branch_name, "main");
    });
}

#[test]
fn rejects_reparenting_to_current_parent() {
    with_temp_repo("dig-reparent", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        create_tracked_branch("feat/auth-ui");

        let error = plan(&ReparentOptions {
            branch_name: Some("feat/auth-ui".into()),
            parent_branch_name: "feat/auth".into(),
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "branch 'feat/auth-ui' is already parented to 'feat/auth'"
        );
    });
}

#[test]
fn rejects_reparenting_onto_descendant() {
    with_temp_repo("dig-reparent", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        create_tracked_branch("feat/auth-api");
        create_tracked_branch("feat/auth-api-tests");

        let error = plan(&ReparentOptions {
            branch_name: Some("feat/auth".into()),
            parent_branch_name: "feat/auth-api".into(),
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "cannot reparent 'feat/auth' onto descendant 'feat/auth-api'"
        );
    });
}

#[test]
fn reparents_tracked_branch_and_restacks_descendants() {
    with_temp_repo("dig-reparent", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-ui");
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "main"]);
        create_tracked_branch("feat/platform");
        commit_file(repo, "platform.txt", "platform\n", "feat: platform");
        git_ok(repo, &["checkout", "main"]);

        let plan = plan(&ReparentOptions {
            branch_name: Some("feat/auth".into()),
            parent_branch_name: "feat/platform".into(),
        })
        .unwrap();
        let outcome = apply(&plan).unwrap();

        assert!(outcome.status.success());
        assert_eq!(
            outcome
                .restacked_branches
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec![
                "feat/auth->feat/platform".to_string(),
                "feat/auth-ui->feat/auth".to_string(),
            ]
        );
        assert_eq!(outcome.restored_original_branch.as_deref(), Some("main"));
        assert_eq!(git::current_branch_name().unwrap(), "main");

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let auth = state.find_branch_by_name("feat/auth").unwrap();
        let platform = state.find_branch_by_name("feat/platform").unwrap();
        let ui = state.find_branch_by_name("feat/auth-ui").unwrap();

        assert_eq!(
            auth.parent,
            ParentRef::Branch {
                node_id: platform.id
            }
        );
        assert_eq!(auth.base_ref, "feat/platform");
        assert_eq!(ui.parent, ParentRef::Branch { node_id: auth.id });
        assert_eq!(ui.base_ref, "feat/auth");
    });
}
