mod support;

use support::{
    active_rebase_head_name, commit_file, dig, dig_ok, find_archived_node, find_node, git_ok,
    git_stdout, initialize_main_repo, load_events_json, load_operation_json, load_state_json,
    overwrite_file, strip_ansi, with_temp_repo,
};

#[test]
fn orphans_current_branch_without_descendants_and_keeps_local_branch() {
    with_temp_repo("dig-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");

        let output = dig_ok(repo, &["orphan"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dig."));
        assert_eq!(
            stdout.trim_end(),
            "Orphaned 'feat/auth'. It is no longer tracked by dig.\n\nmain"
        );
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());

        let events = load_events_json(repo);
        assert!(events.iter().any(|event| {
            event["type"].as_str() == Some("branch_archived")
                && event["branch_name"].as_str() == Some("feat/auth")
                && event["reason"]["kind"].as_str() == Some("orphaned")
        }));
    });
}

#[test]
fn orphans_named_branch_restacks_descendants_to_trunk_and_restores_original_branch() {
    with_temp_repo("dig-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "main"]);

        let output = dig_ok(repo, &["orphan", "feat/auth"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dig."));
        assert!(stdout.contains("Returned to 'main' after orphaning."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert!(stdout.contains("main\n└── feat/auth-ui"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "main"])
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
    });
}

#[test]
fn orphans_named_branch_restacks_descendants_to_tracked_parent() {
    with_temp_repo("dig-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");
        dig_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: auth api tests");
        git_ok(repo, &["checkout", "feat/auth"]);

        let output = dig_ok(repo, &["orphan", "feat/auth-api"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth-api'. It is no longer tracked by dig."));
        assert!(stdout.contains("Returned to 'feat/auth' after orphaning."));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth"));
        assert!(stdout.contains("main\n└── ✓ feat/auth\n    └── feat/auth-api-tests"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-api-tests"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-api-tests").unwrap();
        let parent = find_node(&state, "feat/auth").unwrap();
        assert_eq!(child["base_ref"], "feat/auth");
        assert_eq!(child["parent"]["kind"], "branch");
        assert_eq!(child["parent"]["node_id"], parent["id"]);
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
    });
}

#[test]
fn sync_continues_paused_orphan_operation() {
    with_temp_repo("dig-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "main"]);
        overwrite_file(repo, "shared.txt", "main\n", "feat: trunk");
        git_ok(repo, &["checkout", "feat/auth"]);

        let paused = dig(repo, &["orphan"]);
        let stderr = String::from_utf8(paused.stderr).unwrap();

        assert!(!paused.status.success());
        assert!(stderr.contains("dig sync --continue"));
        assert_eq!(
            load_operation_json(repo).unwrap()["origin"]["type"].as_str(),
            Some("orphan")
        );
        assert!(
            active_rebase_head_name(repo).contains("feat/auth-ui"),
            "expected rebase head-name to reference feat/auth-ui"
        );

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dig."));
        assert!(stdout.contains("Returned to 'feat/auth' after orphaning."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "main"])
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}
