mod support;

use support::{
    commit_file, dig_ok, dig_with_input, find_archived_node, find_node, git_ok, git_stdout,
    initialize_main_repo, load_operation_json, load_state_json, overwrite_file, strip_ansi,
    with_temp_repo,
};

#[test]
fn clean_untracks_deleted_middle_branch_and_restacks_descendants() {
    with_temp_repo("dig-clean-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/users"]);
        commit_file(repo, "users.txt", "users\n", "feat: users");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/auth"]);

        let output = dig_with_input(repo, &["clean"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Tracked branches missing locally and ready to stop tracking:"));
        assert!(stdout.contains("Stop tracking 1 missing branch? [y/N]"));
        assert!(stdout.contains("No longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/users onto main"));

        let state = load_state_json(repo);
        let users = find_node(&state, "feat/users").unwrap();
        assert_eq!(users["base_ref"], "main");
        assert_eq!(users["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert_eq!(
            git_stdout(repo, &["merge-base", "main", "feat/users"]),
            git_stdout(repo, &["rev-parse", "main"])
        );
    });
}

#[test]
fn clean_reconciles_multi_level_missing_ancestors() {
    with_temp_repo("dig-clean-cli", |repo| {
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

        let output = dig_with_input(repo, &["clean"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Stop tracking 2 missing branches? [y/N]"));
        assert!(stdout.contains("No longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("- feat/auth-api"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api-tests onto main"));

        let state = load_state_json(repo);
        let tests_branch = find_node(&state, "feat/auth-api-tests").unwrap();
        assert_eq!(tests_branch["base_ref"], "main");
        assert_eq!(tests_branch["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
    });
}

#[test]
fn clean_untracks_missing_branches_before_cleaning_newly_unblocked_merged_parents() {
    with_temp_repo("dig-clean-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/users"]);
        commit_file(repo, "users.txt", "users\n", "feat: users");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["merge", "--squash", "feat/auth"]);
        git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);
        git_ok(repo, &["branch", "-D", "feat/users"]);

        let output = dig_with_input(repo, &["clean"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(
            stdout.contains("Delete 1 merged branch and stop tracking 1 missing branch? [y/N]")
        );
        assert!(stdout.contains("No longer tracked by dig:"));
        assert!(stdout.contains("- feat/users"));
        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_node(&state, "feat/users").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(find_archived_node(&state, "feat/users").is_some());
        assert!(!git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
    });
}

#[test]
fn clean_branch_scopes_missing_branch_reconciliation() {
    with_temp_repo("dig-clean-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/users"]);
        commit_file(repo, "users.txt", "users\n", "feat: users");
        git_ok(repo, &["checkout", "main"]);
        dig_ok(repo, &["branch", "feat/billing"]);
        commit_file(repo, "billing.txt", "billing\n", "feat: billing");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/auth"]);
        git_ok(repo, &["branch", "-D", "feat/billing"]);

        let output = dig_with_input(repo, &["clean", "--branch", "feat/auth"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("- feat/auth no longer exists locally"));
        assert!(!stdout.contains("feat/billing"));

        let state = load_state_json(repo);
        let users = find_node(&state, "feat/users").unwrap();
        assert_eq!(users["base_ref"], "main");
        assert_eq!(users["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(find_node(&state, "feat/billing").is_some());
    });
}

#[test]
fn clean_continues_paused_deleted_local_restack() {
    with_temp_repo("dig-clean-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        commit_file(repo, "shared.txt", "base\n", "chore: base");
        dig_ok(repo, &["branch", "feat/auth"]);
        dig_ok(repo, &["branch", "feat/users"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child");
        git_ok(repo, &["checkout", "main"]);
        overwrite_file(repo, "shared.txt", "main\n", "feat: trunk");
        git_ok(repo, &["branch", "-D", "feat/auth"]);

        let paused = dig_with_input(repo, &["clean"], "y\n");
        let stderr = String::from_utf8(paused.stderr).unwrap();

        assert!(!paused.status.success());
        assert!(stderr.contains("dig sync --continue"));
        let operation = load_operation_json(repo).unwrap();
        assert_eq!(operation["origin"]["type"].as_str(), Some("clean"));
        assert_eq!(
            operation["origin"]["current_candidate"]["kind"].as_str(),
            Some("deleted_locally")
        );

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "shared.txt"]);

        let resumed = dig_ok(repo, &["sync", "--continue"]);
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(stdout.contains("No longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/users onto main"));

        let state = load_state_json(repo);
        let users = find_node(&state, "feat/users").unwrap();
        assert_eq!(users["base_ref"], "main");
        assert_eq!(users["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert!(load_operation_json(repo).is_none());
    });
}
