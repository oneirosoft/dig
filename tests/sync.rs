mod support;

use support::{
    active_rebase_head_name, commit_file, dig, dig_ok, dig_with_input, find_archived_node,
    find_node, git_ok, git_stdout, initialize_main_repo, load_events_json, load_operation_json,
    load_state_json, overwrite_file, strip_ansi, with_temp_repo, write_file,
};

#[test]
fn sync_reports_noop_when_local_stacks_are_already_in_sync() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert_eq!(stdout.trim_end(), "Local stacks are already in sync.");
    });
}

#[test]
fn sync_restacks_root_stack_after_trunk_advances() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "main"]);
        commit_file(repo, "README.md", "root\nmain\n", "feat: trunk follow-up");
        git_ok(repo, &["checkout", "feat/auth-ui"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth onto main"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert_eq!(
            git_stdout(repo, &["branch", "--show-current"]),
            "feat/auth-ui"
        );
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth"]),
            git_stdout(repo, &["rev-parse", "main"])
        );
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
    });
}

#[test]
fn sync_restacks_mid_stack_after_tracked_parent_advances() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\nmore\n", "feat: parent follow-up");
        git_ok(repo, &["checkout", "main"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert!(!stdout.contains("- feat/auth onto main"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
    });
}

#[test]
fn sync_archives_deleted_leaf_branch_and_reports_it() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "feat/auth"]);
        git_ok(repo, &["branch", "-D", "feat/auth-ui"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Deleted locally and no longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth-ui"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth-ui").is_none());
        assert!(find_archived_node(&state, "feat/auth-ui").is_some());

        let events = load_events_json(repo);
        assert!(events.iter().any(|event| {
            event["type"].as_str() == Some("branch_archived")
                && event["branch_name"].as_str() == Some("feat/auth-ui")
                && event["reason"]["kind"].as_str() == Some("deleted_locally")
        }));
    });
}

#[test]
fn sync_promotes_descendants_when_deleted_middle_branch_is_tracked() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");
        dig_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: tests");
        git_ok(repo, &["checkout", "feat/auth"]);
        git_ok(repo, &["branch", "-D", "feat/auth-api"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Deleted locally and no longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth-api"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth"));

        let state = load_state_json(repo);
        let tests_branch = find_node(&state, "feat/auth-api-tests").unwrap();
        let parent = find_node(&state, "feat/auth").unwrap();
        assert_eq!(tests_branch["base_ref"], "feat/auth");
        assert_eq!(tests_branch["parent"]["kind"], "branch");
        assert_eq!(tests_branch["parent"]["node_id"], parent["id"]);
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-api-tests"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
    });
}

#[test]
fn sync_reconciles_multi_level_missing_ancestors_and_promotes_remaining_stack() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");
        dig_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: tests");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/auth-api"]);
        git_ok(repo, &["branch", "-D", "feat/auth"]);

        let output = dig_ok(repo, &["sync"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Deleted locally and no longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth-api"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api-tests onto main"));

        let state = load_state_json(repo);
        let tests_branch = find_node(&state, "feat/auth-api-tests").unwrap();
        assert_eq!(tests_branch["base_ref"], "main");
        assert_eq!(tests_branch["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth-api-tests"]),
            git_stdout(repo, &["rev-parse", "main"])
        );
    });
}

#[test]
fn sync_hands_off_to_cleanup_and_reuses_delete_prompt() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);

        let output = dig_with_input(repo, &["sync"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Merged branches ready to clean:"));
        assert!(stdout.contains("Delete 1 merged branch? [y/N]"));
        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api onto main"));

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-api").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_node(&state, "feat/auth").is_none());
    });
}

#[test]
fn sync_can_skip_cleanup_after_prompt_without_aborting() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);

        let output = dig_with_input(repo, &["sync"], "n\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Delete 1 merged branch? [y/N]"));
        assert!(stdout.contains("Skipped cleanup."));
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_some());
    });
}

#[test]
fn sync_continues_paused_commit_restack() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "shared.txt", "base\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "feat/auth"]);
        write_file(repo, "shared.txt", "parent\n");
        git_ok(repo, &["add", "shared.txt"]);

        let paused = dig(repo, &["commit", "-m", "feat: parent follow-up"]);
        assert!(!paused.status.success());
        assert!(load_operation_json(repo).is_some());

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("feat: parent follow-up"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_full_sync_rebase() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "main"]);
        overwrite_file(repo, "shared.txt", "main\n", "feat: trunk");
        git_ok(repo, &["checkout", "feat/auth"]);

        let paused = dig(repo, &["sync"]);
        let stderr = String::from_utf8(paused.stderr).unwrap();

        assert!(!paused.status.success());
        assert!(stderr.contains("dig sync --continue"));
        assert_eq!(
            load_operation_json(repo).unwrap()["origin"]["type"].as_str(),
            Some("sync")
        );
        assert!(
            active_rebase_head_name(repo).contains("feat/auth"),
            "expected rebase head-name to reference feat/auth"
        );

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth onto main"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth"]),
            git_stdout(repo, &["rev-parse", "main"])
        );
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_adopt_rebase() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["checkout", "-b", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "feat/auth"]);

        let paused = dig(repo, &["adopt", "feat/auth-ui", "-p", "feat/auth"]);
        assert!(!paused.status.success());
        assert!(load_operation_json(repo).is_some());

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("Adopted 'feat/auth-ui' under 'feat/auth'."));
        assert!(stdout.contains("Restacked 'feat/auth-ui' onto 'feat/auth'."));
        assert!(stdout.contains("Returned to 'feat/auth' after adopt."));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth-ui").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_merge_and_preserves_delete_prompt() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent");

        let paused = dig(repo, &["merge", "feat/auth"]);
        assert!(!paused.status.success());
        assert!(load_operation_json(repo).is_some());

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_with_input(repo, &["sync", "--continue"], "n\n");
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(resumed.status.success());
        assert!(stdout.contains("Merged 'feat/auth' into 'main'."));
        assert!(stdout.contains("Kept merged branch 'feat/auth'."));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_clean_operation() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        dig_ok(repo, &["branch", "feat/auth-api"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);
        git_ok(repo, &["checkout", "feat/auth"]);

        let paused = dig_with_input(repo, &["clean", "--branch", "feat/auth"], "y\n");
        assert!(!paused.status.success());
        assert!(load_operation_json(repo).is_some());

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-api").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_clears_stale_operation_after_rebase_abort() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "shared.txt", "base\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "feat/auth"]);
        write_file(repo, "shared.txt", "parent\n");
        git_ok(repo, &["add", "shared.txt"]);

        let paused = dig(repo, &["commit", "-m", "feat: parent follow-up"]);
        assert!(!paused.status.success());
        assert!(load_operation_json(repo).is_some());

        git_ok(repo, &["rebase", "--abort"]);

        let resumed = dig(repo, &["sync", "--continue"]);
        let stderr = String::from_utf8(resumed.stderr).unwrap();

        assert!(!resumed.status.success());
        assert!(stderr.contains("paused dig commit operation is stale"));
        assert!(load_operation_json(repo).is_none());
    });
}
