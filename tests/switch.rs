mod support;

use support::{
    commit_file, dgr, dgr_ok, dgr_ok_with_env, git_ok, git_stdout, initialize_main_repo,
    load_state_json, with_temp_repo,
};

#[test]
fn switch_directly_to_tracked_branch_preserves_dagger_state() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);

        let state_before = load_state_json(repo);
        let output = dgr_ok(repo, &["switch", "feat/auth"]);
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert_eq!(stdout.trim_end(), "Switched to 'feat/auth'.");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(load_state_json(repo), state_before);
    });
}

#[test]
fn switch_directly_to_untracked_branch_without_dagger_init() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);
        git_ok(repo, &["checkout", "-b", "scratch"]);
        commit_file(repo, "scratch.txt", "scratch\n", "feat: scratch");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok(repo, &["switch", "scratch"]);
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert_eq!(stdout.trim_end(), "Switched to 'scratch'.");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "scratch");
    });
}

#[test]
fn switch_reports_when_branch_is_already_checked_out() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);

        let output = dgr_ok(repo, &["switch", "main"]);
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert_eq!(stdout.trim_end(), "Already on 'main'.");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
    });
}

#[test]
fn switch_direct_mode_reports_missing_branch() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);

        let output = dgr(repo, &["switch", "missing"]);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stdout.is_empty(), "unexpected stdout:\n{stdout}");
        assert!(stderr.contains("branch 'missing' was not found"));
    });
}

#[test]
fn switch_interactive_script_can_confirm_selection() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok_with_env(
            repo,
            &["switch"],
            &[("DGR_SWITCH_TEST_EVENTS", "down,enter")],
        );
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert_eq!(stdout.trim_end(), "Switched to 'feat/auth'.");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
    });
}

#[test]
fn switch_interactive_script_can_cancel_without_switching() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok_with_env(repo, &["switch"], &[("DGR_SWITCH_TEST_EVENTS", "q")]);
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert!(stdout.is_empty(), "unexpected stdout:\n{stdout}");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
    });
}

#[test]
fn switch_interactive_requires_a_tty_without_scripted_input() {
    with_temp_repo("dgr-switch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);

        let output = dgr(repo, &["switch"]);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stdout.is_empty(), "unexpected stdout:\n{stdout}");
        assert!(stderr.contains("dgr switch interactive mode requires an interactive terminal"));
        assert!(stderr.contains("pass a branch name to switch directly"));
    });
}
