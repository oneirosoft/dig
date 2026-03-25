use super::plan::{parent_commit_mentions_all_branch_commits, plan as build_plan};
use super::{BlockedBranch, CleanBlockReason, CleanOptions, CleanReason, apply};
use crate::core::git::{self, CommitMetadata};
use crate::core::store::{ParentRef, dig_paths, load_state};
use crate::core::test_support::{
    append_file, commit_file, create_tracked_branch, git_ok, initialize_main_repo,
    squash_merge_branch_with_commit_listing, with_temp_repo,
};

#[test]
fn reports_non_integrated_branch_reason() {
    let blocked = BlockedBranch {
        branch_name: "feat/auth".into(),
        reason: CleanBlockReason::NotIntegrated {
            parent_branch: "main".into(),
        },
    };

    assert_eq!(
        blocked.reason,
        CleanBlockReason::NotIntegrated {
            parent_branch: "main".into()
        }
    );
}

#[test]
fn tracks_integrated_clean_reason() {
    let reason = CleanReason::IntegratedIntoParent {
        parent_branch: "main".into(),
    };

    assert_eq!(
        reason,
        CleanReason::IntegratedIntoParent {
            parent_branch: "main".into()
        }
    );
}

#[test]
fn detects_squash_commit_message_that_mentions_branch_commits() {
    assert!(parent_commit_mentions_all_branch_commits(
        &CommitMetadata {
            sha: "parent".into(),
            subject: "feat: stacked branch creation".into(),
            body: concat!(
                "feat: stacked branch creation\n\n",
                "commit 1de9e06d1174402332fdd5343a387249b0a5ef66\n",
                "    feat: new parent flag to specify the parent mannualy of a branch\n\n",
                "commit 2099fdf424816e61eceff1a98db2d00fee0f76ac\n",
                "    feat: stacked-branches\n"
            )
            .into(),
        },
        &[
            CommitMetadata {
                sha: "2099fdf424816e61eceff1a98db2d00fee0f76ac".into(),
                subject: "feat: stacked-branches".into(),
                body: String::new(),
            },
            CommitMetadata {
                sha: "1de9e06d1174402332fdd5343a387249b0a5ef66".into(),
                subject: "feat: new parent flag to specify the parent mannualy of a branch".into(),
                body: String::new(),
            },
        ]
    ));
}

#[test]
fn cleans_squash_merged_parent_and_restacks_descendants() {
    with_temp_repo("dig-clean", |repo| {
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
        create_tracked_branch("feat/auth-api-tests");
        commit_file(
            repo,
            "auth-api-tests.txt",
            "tests\n",
            "feat: auth api tests",
        );

        squash_merge_branch_with_commit_listing(repo, "main", "feat/auth", "feat: merge auth");
        git_ok(repo, &["checkout", "feat/auth"]);

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].branch_name, "feat/auth");
        assert_eq!(
            plan.candidates[0]
                .restack_plan
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec![
                "feat/auth-api->main".to_string(),
                "feat/auth-api-tests->feat/auth-api".to_string(),
            ]
        );

        let outcome = apply(&plan).unwrap();

        assert!(outcome.status.success());
        assert_eq!(outcome.switched_to_trunk_from.as_deref(), Some("feat/auth"));
        assert_eq!(outcome.restored_original_branch, None);
        assert_eq!(outcome.deleted_branches, vec!["feat/auth".to_string()]);
        assert_eq!(
            outcome
                .restacked_branches
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec![
                "feat/auth-api->main".to_string(),
                "feat/auth-api-tests->feat/auth-api".to_string(),
            ]
        );

        assert!(!git::branch_exists("feat/auth").unwrap());
        assert!(git::branch_exists("feat/auth-api").unwrap());
        assert!(git::branch_exists("feat/auth-api-tests").unwrap());

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let restacked_child = state.find_branch_by_name("feat/auth-api").unwrap();
        let grandchild = state.find_branch_by_name("feat/auth-api-tests").unwrap();

        assert_eq!(restacked_child.parent, ParentRef::Trunk);
        assert_eq!(restacked_child.base_ref, "main");
        assert_eq!(
            grandchild.parent,
            ParentRef::Branch {
                node_id: restacked_child.id
            }
        );
        assert_eq!(grandchild.base_ref, "feat/auth-api");
        assert!(
            state
                .nodes
                .iter()
                .any(|node| node.branch_name == "feat/auth" && node.archived)
        );
    });
}

#[test]
fn returns_to_original_branch_after_cleaning_from_another_checkout() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

        squash_merge_branch_with_commit_listing(repo, "main", "feat/auth", "feat: merge auth");
        git_ok(repo, &["checkout", "main"]);

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        let outcome = apply(&plan).unwrap();

        assert!(outcome.status.success());
        assert_eq!(outcome.switched_to_trunk_from, None);
        assert_eq!(outcome.restored_original_branch.as_deref(), Some("main"));
        assert_eq!(git::current_branch_name().unwrap(), "main");
        assert!(!git::branch_exists("feat/auth").unwrap());
    });
}

#[test]
fn full_clean_plan_only_lists_deepest_cleanable_branches() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

        git_ok(repo, &["checkout", "feat/auth"]);
        git_ok(repo, &["merge", "--squash", "feat/auth-api"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth api"]);
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);

        let plan = build_plan(&CleanOptions::default()).unwrap();

        assert_eq!(
            plan.candidates
                .iter()
                .map(|candidate| candidate.branch_name.clone())
                .collect::<Vec<_>>(),
            vec!["feat/auth-api".to_string()]
        );
    });
}
