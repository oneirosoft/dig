mod support;

use support::{
    active_rebase_head_name, commit_file, dgr, dgr_ok, events_contain_type, find_node, git_ok,
    git_stdout, initialize_main_repo, load_events_json, load_operation_json, load_state_json,
    overwrite_file, strip_ansi, with_temp_repo,
};

#[test]
fn reparents_current_branch_to_trunk_and_records_event() {
    with_temp_repo("dgr-reparent-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");

        let output = dgr_ok(repo, &["reparent", "-p", "main"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Reparented 'feat/auth-ui' onto 'main'."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert!(stdout.contains("* main\n└── ✓ feat/auth-ui"));
        assert_eq!(
            git_stdout(repo, &["branch", "--show-current"]),
            "feat/auth-ui"
        );
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "main"])
        );

        let state = load_state_json(repo);
        let branch = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(branch["base_ref"], "main");
        assert_eq!(branch["parent"]["kind"], "trunk");

        let events = load_events_json(repo);
        assert!(events.iter().any(|event| {
            event["type"].as_str() == Some("branch_reparented")
                && event["branch_name"].as_str() == Some("feat/auth-ui")
                && event["old_base_ref"].as_str() == Some("feat/auth")
                && event["new_base_ref"].as_str() == Some("main")
                && event["old_parent"]["kind"].as_str() == Some("branch")
                && event["new_parent"]["kind"].as_str() == Some("trunk")
        }));
    });
}

#[test]
fn reparents_named_branch_to_tracked_parent_and_restores_original_branch() {
    with_temp_repo("dgr-reparent-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");
        dgr_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: tests");
        git_ok(repo, &["checkout", "main"]);
        dgr_ok(repo, &["branch", "feat/platform"]);
        commit_file(repo, "platform.txt", "platform\n", "feat: platform");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok(repo, &["reparent", "feat/auth-api", "-p", "feat/platform"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Reparented 'feat/auth-api' onto 'feat/platform'."));
        assert!(stdout.contains("Returned to 'main' after reparenting."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api onto feat/platform"));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth-api"));
        assert!(stdout.contains("✓ main\n└── * feat/platform\n    └── * feat/auth-api\n        └── * feat/auth-api-tests"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/platform", "feat/auth-api"]),
            git_stdout(repo, &["rev-parse", "feat/platform"])
        );
        assert_eq!(
            git_stdout(
                repo,
                &["merge-base", "feat/auth-api", "feat/auth-api-tests"]
            ),
            git_stdout(repo, &["rev-parse", "feat/auth-api"])
        );

        let state = load_state_json(repo);
        let api = find_node(&state, "feat/auth-api").unwrap();
        let tests = find_node(&state, "feat/auth-api-tests").unwrap();
        let platform = find_node(&state, "feat/platform").unwrap();
        assert_eq!(api["base_ref"], "feat/platform");
        assert_eq!(api["parent"]["kind"], "branch");
        assert_eq!(api["parent"]["node_id"], platform["id"]);
        assert_eq!(tests["base_ref"], "feat/auth-api");
        assert_eq!(tests["parent"]["kind"], "branch");
        assert_eq!(tests["parent"]["node_id"], api["id"]);
    });
}

#[test]
fn reparents_named_branch_and_shows_unrelated_checked_out_branch_below_tree() {
    with_temp_repo("dgr-reparent-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");
        dgr_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: tests");
        git_ok(repo, &["checkout", "main"]);
        dgr_ok(repo, &["branch", "feat/platform"]);
        commit_file(repo, "platform.txt", "platform\n", "feat: platform");
        git_ok(repo, &["checkout", "main"]);
        dgr_ok(repo, &["branch", "feat/billing"]);
        commit_file(repo, "billing.txt", "billing\n", "feat: billing");
        git_ok(repo, &["checkout", "feat/billing"]);

        let output = dgr_ok(repo, &["reparent", "feat/auth-api", "-p", "feat/platform"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Reparented 'feat/auth-api' onto 'feat/platform'."));
        assert!(stdout.contains("Returned to 'feat/billing' after reparenting."));
        assert!(stdout.contains("- feat/auth-api onto feat/platform"));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth-api"));
        assert!(stdout.contains(
            "* main\n└── * feat/platform\n    └── * feat/auth-api\n        └── * feat/auth-api-tests\n\n✓ feat/billing"
        ));
        assert_eq!(
            git_stdout(repo, &["branch", "--show-current"]),
            "feat/billing"
        );
    });
}

#[test]
fn rejects_reparenting_onto_descendant() {
    with_temp_repo("dgr-reparent-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: api");

        let output = dgr(repo, &["reparent", "feat/auth", "-p", "feat/auth-api"]);
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stderr.contains("cannot reparent 'feat/auth' onto descendant 'feat/auth-api'"));
    });
}

#[test]
fn leaves_rebase_open_when_reparent_conflicts_and_sync_continues() {
    with_temp_repo("dgr-reparent-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "auth\n", "feat: auth change");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["checkout", "main"]);
        dgr_ok(repo, &["branch", "feat/platform"]);
        overwrite_file(repo, "shared.txt", "platform\n", "feat: platform change");
        git_ok(repo, &["checkout", "main"]);

        let paused = dgr(repo, &["reparent", "feat/auth", "-p", "feat/platform"]);
        let stderr = String::from_utf8(paused.stderr).unwrap();

        assert!(!paused.status.success());
        assert!(stderr.contains("dgr sync --continue"));
        assert_eq!(
            load_operation_json(repo).unwrap()["origin"]["type"].as_str(),
            Some("reparent")
        );
        assert!(
            active_rebase_head_name(repo).contains("feat/auth"),
            "expected rebase head-name to reference feat/auth"
        );
        assert!(
            !events_contain_type(repo, "branch_reparented"),
            "did not expect branch_reparented event before conflict resolution"
        );

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dgr_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("Reparented 'feat/auth' onto 'feat/platform'."));
        assert!(stdout.contains("Returned to 'main' after reparenting."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth onto feat/platform"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/platform", "feat/auth"]),
            git_stdout(repo, &["rev-parse", "feat/platform"])
        );
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );

        let state = load_state_json(repo);
        let auth = find_node(&state, "feat/auth").unwrap();
        let platform = find_node(&state, "feat/platform").unwrap();
        let ui = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(auth["base_ref"], "feat/platform");
        assert_eq!(auth["parent"]["kind"], "branch");
        assert_eq!(auth["parent"]["node_id"], platform["id"]);
        assert_eq!(ui["base_ref"], "feat/auth");
        assert_eq!(ui["parent"]["node_id"], auth["id"]);
        assert!(load_operation_json(repo).is_none());
    });
}
