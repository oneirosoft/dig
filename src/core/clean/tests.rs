use std::fs;
use std::path::PathBuf;

use super::plan::{
    parent_commit_mentions_all_branch_commits, parent_commit_mentions_tracked_pull_request,
    plan as build_plan, plan_for_sync as build_sync_plan,
};
use super::{BlockedBranch, CleanBlockReason, CleanOptions, CleanReason, apply};
use crate::core::git::{self, CommitMetadata};
use crate::core::restack::RestackBaseTarget;
use crate::core::store::{
    BranchDivergenceState, BranchPullRequestTrackedSource, ParentRef, TrackedPullRequest,
    dig_paths, load_state, open_initialized, record_branch_pull_request_tracked,
};
use crate::core::test_support::{
    append_file, commit_file, create_tracked_branch, git_ok, initialize_main_repo,
    squash_merge_branch_with_commit_listing, with_temp_repo,
};

fn initialize_origin_remote(repo: &std::path::Path) {
    git_ok(repo, &["init", "--bare", "origin.git"]);
    git_ok(repo, &["remote", "add", "origin", "origin.git"]);
    git_ok(repo, &["push", "-u", "origin", "main"]);
    git_ok(
        repo,
        &[
            "--git-dir=origin.git",
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
}

fn clone_origin(repo: &std::path::Path, clone_name: &str) -> PathBuf {
    let clone_dir = repo.join(clone_name);
    let clone_path = clone_dir.to_string_lossy().into_owned();
    git_ok(repo, &["clone", "origin.git", &clone_path]);
    git_ok(&clone_dir, &["config", "user.name", "Dig Remote"]);
    git_ok(&clone_dir, &["config", "user.email", "remote@example.com"]);
    git_ok(&clone_dir, &["config", "commit.gpgsign", "false"]);
    clone_dir
}

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
        parent_base: RestackBaseTarget::local("main"),
    };

    assert_eq!(
        reason,
        CleanReason::IntegratedIntoParent {
            parent_base: RestackBaseTarget::local("main")
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
fn detects_parent_commit_that_mentions_tracked_pull_request_number() {
    assert!(parent_commit_mentions_tracked_pull_request(
        &CommitMetadata {
            sha: "parent".into(),
            subject: "feat: add GitHub PR workflows (#2)".into(),
            body: "See https://github.com/acme/dig/pull/2 for details.".into(),
        },
        2,
    ));
    assert!(!parent_commit_mentions_tracked_pull_request(
        &CommitMetadata {
            sha: "parent".into(),
            subject: "feat: add GitHub PR workflows (#12)".into(),
            body: String::new(),
        },
        2,
    ));
}

#[test]
fn fresh_branch_is_not_cleanable_without_commits() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        assert!(plan.candidates.is_empty());
        assert_eq!(
            plan.blocked,
            vec![BlockedBranch {
                branch_name: "feat/auth".into(),
                reason: CleanBlockReason::NotIntegrated {
                    parent_branch: "main".into(),
                },
            }]
        );

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let branch = state.find_branch_by_name("feat/auth").unwrap();
        assert_eq!(
            branch.divergence_state,
            BranchDivergenceState::NeverDiverged {
                aligned_head_oid: git::ref_oid("feat/auth").unwrap(),
            }
        );
    });
}

#[test]
fn rebased_fresh_branch_remains_not_cleanable() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        git_ok(repo, &["checkout", "main"]);
        commit_file(repo, "README.md", "root\nmain\n", "feat: trunk follow-up");
        git_ok(repo, &["checkout", "feat/auth"]);
        git_ok(repo, &["rebase", "main"]);

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        assert!(plan.candidates.is_empty());
        assert_eq!(
            plan.blocked,
            vec![BlockedBranch {
                branch_name: "feat/auth".into(),
                reason: CleanBlockReason::NotIntegrated {
                    parent_branch: "main".into(),
                },
            }]
        );

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let branch = state.find_branch_by_name("feat/auth").unwrap();
        assert_eq!(
            branch.divergence_state,
            BranchDivergenceState::NeverDiverged {
                aligned_head_oid: git::ref_oid("feat/auth").unwrap(),
            }
        );
    });
}

#[test]
fn legacy_unknown_branch_with_fast_forwarded_manual_commit_is_cleanable() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--ff-only", "feat/auth"]);

        let state_path = repo.join(".git/dig/state.json");
        let mut state_json =
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&state_path).unwrap())
                .unwrap();
        state_json["nodes"][0]
            .as_object_mut()
            .unwrap()
            .remove("divergence_state");
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&state_json).unwrap(),
        )
        .unwrap();

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].branch_name, "feat/auth");

        let repo_context = git::resolve_repo_context().unwrap();
        let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
        let branch = state.find_branch_by_name("feat/auth").unwrap();
        assert_eq!(branch.divergence_state, BranchDivergenceState::Diverged);
    });
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
fn clean_plan_detects_local_integration_by_tracked_pull_request_number() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: add GitHub PR workflows");
        commit_file(
            repo,
            "tree.txt",
            "tree\n",
            "fix(tree): show PR numbers in lineage views",
        );

        let mut session = open_initialized("dig is not initialized").unwrap();
        let branch = session
            .state
            .find_branch_by_name("feat/auth")
            .unwrap()
            .clone();
        record_branch_pull_request_tracked(
            &mut session,
            branch.id,
            branch.branch_name.clone(),
            TrackedPullRequest { number: 2 },
            BranchPullRequestTrackedSource::Created,
        )
        .unwrap();

        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(
            repo,
            &[
                "commit",
                "--quiet",
                "-m",
                "feat: add GitHub PR workflows (#2)",
                "-m",
                "Adds GitHub PR creation and tracking support.",
            ],
        );

        let plan = build_plan(&CleanOptions {
            branch_name: Some("feat/auth".into()),
        })
        .unwrap();

        assert_eq!(
            plan.candidates
                .iter()
                .map(|candidate| candidate.branch_name.clone())
                .collect::<Vec<_>>(),
            vec!["feat/auth".to_string()]
        );
        assert_eq!(
            plan.candidates[0].reason,
            CleanReason::IntegratedIntoParent {
                parent_base: RestackBaseTarget::local("main"),
            }
        );
    });
}

#[test]
fn sync_plan_detects_remote_only_integrated_branch() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        create_tracked_branch("feat/auth-ui");
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");

        let remote_repo = clone_origin(repo, "origin-worktree");
        git_ok(&remote_repo, &["checkout", "main"]);
        git_ok(&remote_repo, &["merge", "--squash", "origin/feat/auth"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: merge auth"],
        );
        git_ok(&remote_repo, &["push", "origin", "main"]);
        git_ok(repo, &["fetch", "--prune", "origin"]);

        let local_plan = build_plan(&CleanOptions::default()).unwrap();
        assert!(local_plan.candidates.is_empty());

        let sync_plan = build_sync_plan().unwrap();
        assert_eq!(
            sync_plan
                .candidates
                .iter()
                .map(|candidate| candidate.branch_name.clone())
                .collect::<Vec<_>>(),
            vec!["feat/auth".to_string()]
        );
        assert_eq!(
            sync_plan.candidates[0]
                .restack_plan
                .iter()
                .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                .collect::<Vec<_>>(),
            vec!["feat/auth-ui->main".to_string()]
        );
        assert_eq!(
            sync_plan.candidates[0].reason,
            CleanReason::IntegratedIntoParent {
                parent_base: RestackBaseTarget::with_rebase_ref("main", "origin/main"),
            }
        );
    });
}

#[test]
fn sync_plan_ignores_remote_integration_when_parent_remote_ref_is_missing() {
    with_temp_repo("dig-clean", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        create_tracked_branch("feat/auth");
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        create_tracked_branch("feat/auth-api");
        commit_file(repo, "api.txt", "api\n", "feat: auth api");

        let sync_plan = build_sync_plan().unwrap();

        assert!(sync_plan.candidates.is_empty());
        assert!(sync_plan.blocked.iter().any(|blocked| {
            blocked.branch_name == "feat/auth-api"
                && blocked.reason
                    == CleanBlockReason::NotIntegrated {
                        parent_branch: "feat/auth".into(),
                    }
        }));
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
