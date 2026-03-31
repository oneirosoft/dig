mod support;

use support::{
    commit_file, dgr, dgr_ok, find_archived_node, find_node, git_ok, git_stdout,
    initialize_main_repo, load_operation_json, load_state_json, overwrite_file, strip_ansi,
    with_temp_repo, write_file,
};

#[test]
fn orphans_current_branch_without_descendants_and_keeps_local_branch() {
    with_temp_repo("dgr-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");

        let output = dgr_ok(repo, &["orphan"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dagger."));
        assert_eq!(
            stdout.trim_end(),
            "Orphaned 'feat/auth'. It is no longer tracked by dagger.\n\n* main\n\n✓ feat/auth (orphaned)"
        );
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn orphans_named_branch_restacks_descendants_to_trunk_and_restores_original_branch() {
    with_temp_repo("dgr-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok(repo, &["orphan", "feat/auth"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dagger."));
        assert!(stdout.contains("Returned to 'main' after orphaning."));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert!(stdout.contains("✓ main\n└── * feat/auth-ui"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "main"])
        );

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn orphans_named_branch_restacks_descendants_to_tracked_parent() {
    with_temp_repo("dgr-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: auth api");
        dgr_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: auth api tests");
        git_ok(repo, &["checkout", "feat/auth"]);

        let output = dgr_ok(repo, &["orphan", "feat/auth-api"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth-api'. It is no longer tracked by dagger."));
        assert!(stdout.contains("Returned to 'feat/auth' after orphaning."));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth"));
        assert!(stdout.contains("* main\n└── ✓ feat/auth\n    └── * feat/auth-api-tests"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-api-tests"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth-api").is_none());
        let child = find_node(&state, "feat/auth-api-tests").unwrap();
        assert_eq!(child["base_ref"], "feat/auth");
        assert_eq!(
            child["parent"]["node_id"],
            find_node(&state, "feat/auth").unwrap()["id"]
        );
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_orphan_operation() {
    with_temp_repo("dgr-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: auth ui");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr(repo, &["orphan", "feat/auth"]);
        assert!(!output.status.success());

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_some());

        overwrite_file(repo, "shared.txt", "resolved\n", "fix: resolve conflict");
        git_ok(repo, &["add", "shared.txt"]);

        let output = dgr_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dagger."));
        assert!(stdout.contains("Returned to 'main' after orphaning."));
        assert!(stdout.contains("- feat/auth-ui onto main"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_continues_paused_orphan_and_shows_orphaned_current_branch_below_tree() {
    with_temp_repo("dgr-orphan-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: auth ui");
        git_ok(repo, &["checkout", "main"]);
        overwrite_file(repo, "shared.txt", "main\n", "feat: trunk");
        git_ok(repo, &["checkout", "feat/auth"]);

        let output = dgr(repo, &["orphan"]);
        assert!(!output.status.success());
        assert!(load_operation_json(repo).is_some());

        write_file(repo, "shared.txt", "resolved\n");
        git_ok(repo, &["add", "shared.txt"]);

        let output = dgr_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Orphaned 'feat/auth'. It is no longer tracked by dagger."));
        assert!(stdout.contains("Returned to 'feat/auth' after orphaning."));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert!(stdout.contains("* main\n└── * feat/auth-ui\n\n✓ feat/auth (orphaned)"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn reproduces_issue_7_orphan_tree_shows_parent_as_checked_out_after_orphan() {
    with_temp_repo("dgr-orphan-issue-7", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/sync-progress"]);
        commit_file(repo, "progress.txt", "progress\n", "feat: sync progress");
        dgr_ok(repo, &["branch", "fix/sync-premature-deletion"]);
        commit_file(repo, "fix.txt", "fix\n", "fix: sync premature deletion");

        // We are on 'fix/sync-premature-deletion'. Now orphan it.
        let output = dgr_ok(repo, &["orphan"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        // The bug: parent 'feat/sync-progress' is marked as current (✓) but it should be '*'
        // and 'fix/sync-premature-deletion' should be at the bottom with '✓' and '(orphaned)' suffix.
        assert!(stdout.contains(
            "Orphaned 'fix/sync-premature-deletion'. It is no longer tracked by dagger."
        ));

        // This assertion is EXPECTED TO FAIL until the fix is applied.
        // Current (buggy) output likely looks like:
        // main
        // └── ✓ feat/sync-progress

        assert!(stdout.contains(
            "main\n└── * feat/sync-progress\n\n✓ fix/sync-premature-deletion (orphaned)"
        ));
    });
}
