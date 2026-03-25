use std::io;

use super::{OrphanOptions, apply, plan};
use crate::core::git;
use crate::core::store::{ParentRef, dig_paths, load_state};
use crate::core::test_support::{
    commit_file, create_tracked_branch, git_ok, initialize_main_repo, with_temp_repo,
};

#[test]
fn rejects_orphaning_trunk_branch() {
    with_temp_repo("dig-orphan", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/bootstrap");

        let error = plan(&OrphanOptions {
            branch_name: Some("main".into()),
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "cannot orphan trunk branch 'main'");
    });
}

#[test]
fn orphans_tracked_branch_and_restacks_descendants() {
    with_temp_repo("dig-orphan", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-ui");
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "feat/auth"]);

        let plan = plan(&OrphanOptions {
            branch_name: Some("feat/auth".into()),
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
            vec!["feat/auth-ui->main".to_string()]
        );
        assert_eq!(
            outcome.restored_original_branch.as_deref(),
            Some("feat/auth")
        );
        assert_eq!(git::current_branch_name().unwrap(), "feat/auth");

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let child = state.find_branch_by_name("feat/auth-ui").unwrap();

        assert_eq!(child.parent, ParentRef::Trunk);
        assert_eq!(child.base_ref, "main");
        assert!(
            state
                .nodes
                .iter()
                .any(|node| node.branch_name == "feat/auth" && node.archived)
        );
    });
}
