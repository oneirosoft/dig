use super::apply::build_squash_commit_message;
use super::plan::plan as build_plan;
use super::{MergeMode, MergeOptions, delete_merged_branch};
use crate::core::git;
use crate::core::store::{ParentRef, dig_paths, load_state};
use crate::core::test_support::{
    append_file, commit_file, create_tracked_branch, git_ok, git_output, initialize_main_repo,
    with_temp_repo,
};
use std::fs;

#[test]
fn builds_default_squash_commit_message_with_commit_listing() {
    let message = build_squash_commit_message(
        "feat/auth",
        "main",
        &[],
        &[
            crate::core::git::CommitMetadata {
                sha: "abc123".into(),
                subject: "feat: auth".into(),
                body: String::new(),
            },
            crate::core::git::CommitMetadata {
                sha: "def456".into(),
                subject: "feat: auth api".into(),
                body: String::new(),
            },
        ],
    );

    assert_eq!(
        message,
        concat!(
            "merge feat/auth into main\n\n",
            "commit abc123\n",
            "    feat: auth\n\n",
            "commit def456\n",
            "    feat: auth api"
        )
    );
}

#[test]
fn appends_commit_listing_after_user_supplied_squash_message() {
    let message = build_squash_commit_message(
        "feat/auth",
        "main",
        &["custom subject".into(), "extra context".into()],
        &[crate::core::git::CommitMetadata {
            sha: "abc123".into(),
            subject: "feat: auth".into(),
            body: String::new(),
        }],
    );

    assert_eq!(
        message,
        concat!(
            "custom subject\n\n",
            "extra context\n\n",
            "commit abc123\n",
            "    feat: auth"
        )
    );
}

#[test]
fn merges_child_into_parent_and_restacks_descendants() {
    with_temp_repo("dig-merge", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");
        create_tracked_branch("feat/auth-api-tests");
        commit_file(
            repo,
            "auth-api-tests.txt",
            "tests\n",
            "feat: auth api tests",
        );

        git_ok(repo, &["checkout", "feat/auth-api"]);

        let merge_plan = build_plan(&MergeOptions {
            branch_name: "feat/auth-api".into(),
            mode: MergeMode::Normal,
            messages: vec![],
        })
        .unwrap();

        let outcome = super::apply(&merge_plan).unwrap();

        assert!(outcome.status.success());
        assert_eq!(
            outcome.switched_to_target_from.as_deref(),
            Some("feat/auth-api")
        );
        assert_eq!(
            outcome
                .restacked_branches
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec!["feat/auth-api-tests->feat/auth".to_string()]
        );
        assert_eq!(git::current_branch_name().unwrap(), "feat/auth");

        let state = load_state(&dig_paths(&git::resolve_repo_context().unwrap().git_dir)).unwrap();
        let restacked_child = state.find_branch_by_name("feat/auth-api-tests").unwrap();
        assert_eq!(
            restacked_child.parent,
            ParentRef::Branch {
                node_id: state.find_branch_by_name("feat/auth").unwrap().id
            }
        );
        assert_eq!(restacked_child.base_ref, "feat/auth");

        let delete_outcome = delete_merged_branch(&merge_plan).unwrap();
        assert!(delete_outcome.status.success());
        assert!(!git::branch_exists("feat/auth-api").unwrap());
    });
}

#[test]
fn squash_merges_into_trunk_and_keeps_branch_when_delete_is_declined() {
    with_temp_repo("dig-merge", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        append_file(
            repo,
            "auth.txt",
            "auth second line\n",
            "feat: auth follow-up",
        );
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

        git_ok(repo, &["checkout", "feat/auth"]);

        let merge_plan = build_plan(&MergeOptions {
            branch_name: "feat/auth".into(),
            mode: MergeMode::Squash,
            messages: vec!["custom merge".into()],
        })
        .unwrap();

        let outcome = super::apply(&merge_plan).unwrap();

        assert!(outcome.status.success());
        assert_eq!(
            outcome.switched_to_target_from.as_deref(),
            Some("feat/auth")
        );
        assert_eq!(
            outcome
                .restacked_branches
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec!["feat/auth-api->main".to_string()]
        );
        assert_eq!(git::current_branch_name().unwrap(), "main");
        assert!(git::branch_exists("feat/auth").unwrap());

        let log_message = git_output(repo, &["log", "-1", "--format=%B"]);
        assert!(log_message.contains("custom merge"));
        assert!(log_message.contains("commit "));
        assert!(log_message.contains("feat: auth"));

        let state = load_state(&dig_paths(&git::resolve_repo_context().unwrap().git_dir)).unwrap();
        let restacked_child = state.find_branch_by_name("feat/auth-api").unwrap();
        assert_eq!(restacked_child.parent, ParentRef::Trunk);
        assert_eq!(restacked_child.base_ref, "main");
        assert!(state.find_branch_by_name("feat/auth").is_some());
    });
}

#[test]
fn blocks_merge_when_tracked_descendant_is_missing_locally() {
    with_temp_repo("dig-merge", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/auth-api"]);

        let error = build_plan(&MergeOptions {
            branch_name: "feat/auth".into(),
            mode: MergeMode::Normal,
            messages: vec![],
        })
        .unwrap_err();

        assert!(error.to_string().contains("missing locally"));
    });
}

#[test]
fn leaves_state_unchanged_when_merge_conflicts() {
    with_temp_repo("dig-merge", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "shared.txt", "source branch\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);
        fs::write(repo.join("shared.txt"), "main branch\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);
        git_ok(
            repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "feat: main",
            ],
        );

        let merge_plan = build_plan(&MergeOptions {
            branch_name: "feat/auth".into(),
            mode: MergeMode::Normal,
            messages: vec![],
        })
        .unwrap();

        let outcome = super::apply(&merge_plan).unwrap();

        assert!(!outcome.status.success());

        let state = load_state(&dig_paths(&git::resolve_repo_context().unwrap().git_dir)).unwrap();
        let node = state.find_branch_by_name("feat/auth").unwrap();
        assert_eq!(node.parent, ParentRef::Trunk);
        assert!(git::branch_exists("feat/auth").unwrap());
        assert_eq!(git::current_branch_name().unwrap(), "main");
    });
}
