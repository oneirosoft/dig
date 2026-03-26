mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use support::{
    active_rebase_head_name, commit_file, dig, dig_ok, dig_with_env, dig_with_input,
    dig_with_input_and_env, find_archived_node, find_node, git_ok, git_stdout,
    initialize_main_repo, install_fake_executable, load_events_json, load_operation_json,
    load_state_json, overwrite_file, path_with_prepend, strip_ansi, with_temp_repo, write_file,
};

fn initialize_origin_remote(repo: &Path) {
    git_ok(repo, &["init", "--bare", ".git/origin.git"]);
    git_ok(repo, &["remote", "add", "origin", ".git/origin.git"]);
    git_ok(repo, &["push", "-u", "origin", "main"]);
    git_ok(
        repo,
        &[
            "--git-dir=.git/origin.git",
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
}

fn clone_origin(repo: &Path, clone_name: &str) -> PathBuf {
    let clone_dir = repo.join(".git").join(clone_name);
    let clone_path = clone_dir.to_string_lossy().into_owned();
    git_ok(repo, &["clone", ".git/origin.git", &clone_path]);
    git_ok(&clone_dir, &["config", "user.name", "Dig Remote"]);
    git_ok(&clone_dir, &["config", "user.email", "remote@example.com"]);
    git_ok(&clone_dir, &["config", "commit.gpgsign", "false"]);
    clone_dir
}

fn install_fake_gh(repo: &Path, script: &str) -> (String, String) {
    let bin_dir = repo.join(".git").join("fake-bin");
    install_fake_executable(&bin_dir, "gh", script);

    let path = path_with_prepend(&bin_dir);
    let log_path = repo.join(".git").join("gh.log");
    fs::write(&log_path, "").unwrap();

    (path, log_path.display().to_string())
}

fn install_remote_update_logger(repo: &Path) -> String {
    let hooks_dir = repo.join(".git").join("origin.git").join("hooks");
    let log_path = repo.join(".git").join("origin-updates.log");
    let script = format!(
        "#!/bin/sh\nset -eu\nprintf '%s %s %s\\n' \"$1\" \"$2\" \"$3\" >> \"{}\"\n",
        log_path.display()
    );
    install_fake_executable(&hooks_dir, "update", &script);
    fs::write(&log_path, "").unwrap();

    log_path.display().to_string()
}

fn count_remote_ref_updates(log_path: &str, ref_name: &str) -> usize {
    fs::read_to_string(log_path)
        .unwrap()
        .lines()
        .filter(|line| line.split_whitespace().next() == Some(ref_name))
        .count()
}

fn track_pull_request_number(repo: &Path, branch_name: &str, number: u64) {
    let state_path = repo.join(".git/dig/state.json");
    let mut state = load_state_json(repo);
    let nodes = state["nodes"].as_array_mut().unwrap();
    let node = nodes
        .iter_mut()
        .find(|node| {
            node["branch_name"].as_str() == Some(branch_name)
                && node["archived"].as_bool() == Some(false)
        })
        .unwrap();
    node["pull_request"] = json!({ "number": number });
    fs::write(state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

fn set_branch_archived(repo: &Path, branch_name: &str, archived: bool) {
    let state_path = repo.join(".git/dig/state.json");
    let mut state = load_state_json(repo);
    let node = state["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["branch_name"].as_str() == Some(branch_name))
        .unwrap();
    node["archived"] = json!(archived);
    fs::write(state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

fn setup_remotely_merged_root_branch_with_local_trunk_advance(repo: &Path) {
    initialize_main_repo(repo);
    initialize_origin_remote(repo);
    dig_ok(repo, &["init"]);
    dig_ok(repo, &["branch", "feat/auth"]);
    overwrite_file(repo, "shared.txt", "feature\n", "feat: auth");
    git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
    dig_ok(repo, &["branch", "feat/auth-ui"]);
    commit_file(repo, "ui.txt", "ui\n", "feat: ui");
    git_ok(repo, &["checkout", "main"]);
    overwrite_file(
        repo,
        "shared.txt",
        "local trunk\n",
        "feat: local trunk follow-up",
    );

    let remote_repo = clone_origin(repo, "origin-worktree");
    git_ok(&remote_repo, &["checkout", "main"]);
    git_ok(&remote_repo, &["merge", "--squash", "origin/feat/auth"]);
    git_ok(
        &remote_repo,
        &["commit", "--quiet", "-m", "feat: merge auth"],
    );
    git_ok(&remote_repo, &["push", "origin", "main"]);
    git_ok(&remote_repo, &["push", "origin", "--delete", "feat/auth"]);
}

fn setup_remotely_merged_root_branch_with_children(
    repo: &Path,
    children: &[(&str, &str, &str, &str)],
) {
    initialize_main_repo(repo);
    initialize_origin_remote(repo);
    dig_ok(repo, &["init"]);
    dig_ok(repo, &["branch", "feat/auth"]);
    overwrite_file(repo, "shared.txt", "feature\n", "feat: auth");
    git_ok(repo, &["push", "-u", "origin", "feat/auth"]);

    for (index, (branch_name, file_name, contents, message)) in children.iter().enumerate() {
        if index > 0 {
            git_ok(repo, &["checkout", "feat/auth"]);
        }

        dig_ok(repo, &["branch", branch_name]);
        commit_file(repo, file_name, contents, message);
        git_ok(repo, &["push", "-u", "origin", branch_name]);
    }

    git_ok(repo, &["checkout", "main"]);
    overwrite_file(
        repo,
        "shared.txt",
        "local trunk\n",
        "feat: local trunk follow-up",
    );

    let remote_repo = clone_origin(repo, "origin-worktree");
    git_ok(&remote_repo, &["checkout", "main"]);
    git_ok(&remote_repo, &["merge", "--squash", "origin/feat/auth"]);
    git_ok(
        &remote_repo,
        &["commit", "--quiet", "-m", "feat: merge auth"],
    );
    git_ok(&remote_repo, &["push", "origin", "main"]);
    git_ok(&remote_repo, &["push", "origin", "--delete", "feat/auth"]);
}

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
fn sync_cleans_branch_merged_by_tracked_pull_request_number_without_rebasing() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: add GitHub PR workflows");
        commit_file(
            repo,
            "tree.txt",
            "tree\n",
            "fix(tree): show PR numbers in lineage views",
        );
        track_pull_request_number(repo, "feat/auth", 2);

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

        let output = dig_with_input(repo, &["sync"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(stdout.contains("Merged branches ready to clean:"));
        assert!(stdout.contains("- feat/auth merged into main"));
        assert!(stdout.contains("Delete 1 merged branch? [y/N]"));
        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(!stderr.contains("dig sync --continue"));
        assert!(!git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));
        assert!(load_operation_json(repo).is_none());
        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());
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

#[test]
fn sync_aborts_before_local_restack_when_fetch_fails() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);
        commit_file(repo, "README.md", "root\nmain\n", "feat: trunk follow-up");
        git_ok(repo, &["checkout", "feat/auth"]);
        git_ok(repo, &["remote", "add", "origin", "/does/not/exist"]);

        let output = dig(repo, &["sync"]);
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stderr.contains("git fetch --prune 'origin' failed"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_ne!(
            git_stdout(repo, &["merge-base", "main", "feat/auth"]),
            git_stdout(repo, &["rev-parse", "main"])
        );
        assert!(load_operation_json(repo).is_none());
    });
}

#[test]
fn sync_cleans_root_branch_merged_remotely_and_restacks_child_onto_fetched_remote_parent() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_local_trunk_advance(repo);

        let output = dig_with_input(repo, &["sync"], "y\nn\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Merged branches ready to clean:"));
        assert!(stdout.contains("- feat/auth merged into main"));
        assert!(stdout.contains("Delete 1 merged branch? [y/N]"));
        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(!stdout.contains("- feat/auth onto main"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert!(stdout.contains("Remote branches to update:"));
        assert!(stdout.contains("- create feat/auth-ui on origin"));
        assert!(!stdout.contains("- create feat/auth on origin"));
        assert!(stdout.contains("Push these remote updates? [y/N]"));
        assert!(stdout.contains("Skipped remote updates."));
        assert_eq!(
            git_stdout(repo, &["merge-base", "origin/main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "origin/main"])
        );
        assert_ne!(
            git_stdout(repo, &["rev-parse", "main"]),
            git_stdout(repo, &["rev-parse", "origin/main"])
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());
    });
}

#[test]
fn sync_skips_recreating_remotely_merged_root_branch_when_cleanup_is_declined() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_local_trunk_advance(repo);

        let output = dig_with_input(repo, &["sync"], "n\nn\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Merged branches ready to clean:"));
        assert!(stdout.contains("- feat/auth merged into main"));
        assert!(stdout.contains("Delete 1 merged branch? [y/N]"));
        assert!(stdout.contains("Skipped cleanup."));
        assert!(stdout.contains("Remote branches to update:"));
        assert!(stdout.contains("- create feat/auth-ui on origin"));
        assert!(!stdout.contains("- create feat/auth on origin"));
        assert!(!stdout.contains("- force-push feat/auth on origin"));
        assert!(load_operation_json(repo).is_none());
        assert!(git_stdout(repo, &["branch", "--list", "feat/auth"]).contains("feat/auth"));

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth").is_some());
        assert!(find_archived_node(&state, "feat/auth").is_none());
    });
}

#[test]
fn sync_repairs_closed_child_pull_request_after_remote_parent_branch_deletion() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_children(
            repo,
            &[("feat/auth-ui", "ui.txt", "ui\n", "feat: ui")],
        );
        track_pull_request_number(repo, "feat/auth-ui", 234);

        let (path, log_path) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "234" ]; then
  printf '{"number":234,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/234"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "reopen" ] && [ "$3" = "234" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "ready" ] && [ "$3" = "234" ] && [ "$4" = "--undo" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$3" = "234" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

        let output = dig_with_input_and_env(
            repo,
            &["sync"],
            "y\nn\n",
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(stdout.contains("Recovered pull requests:"));
        assert!(stdout.contains(
            "- feat/auth-ui (#234): reopened as draft and retargeted from feat/auth to main"
        ));
        assert!(stdout.contains("Merged branches ready to clean:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(gh_log.contains(
            "pr view 234 --json number,state,mergedAt,baseRefName,headRefName,headRefOid,isDraft,url"
        ));
        assert!(gh_log.contains("pr reopen 234"));
        assert!(gh_log.contains("pr ready 234 --undo"));
        assert!(gh_log.contains("pr edit 234 --base main"));

        assert_eq!(
            git_stdout(
                repo,
                &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"]
            ),
            ""
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(find_archived_node(&state, "feat/auth").is_some());
    });
}

#[test]
fn sync_repairs_multiple_child_pull_requests_with_one_temporary_parent_restore() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_children(
            repo,
            &[
                ("feat/auth-api", "api.txt", "api\n", "feat: api"),
                ("feat/auth-ui", "ui.txt", "ui\n", "feat: ui"),
            ],
        );
        track_pull_request_number(repo, "feat/auth-api", 111);
        track_pull_request_number(repo, "feat/auth-ui", 222);
        let remote_update_log = install_remote_update_logger(repo);

        let (path, gh_log_path) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "111" ]; then
  printf '{"number":111,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-api","isDraft":false,"url":"https://github.com/acme/dig/pull/111"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "222" ]; then
  printf '{"number":222,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/222"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "reopen" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "ready" ] && [ "$4" = "--undo" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

        let output = dig_with_input_and_env(
            repo,
            &["sync"],
            "y\nn\n",
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", gh_log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(stdout.contains("Recovered pull requests:"));
        assert_eq!(
            count_remote_ref_updates(&remote_update_log, "refs/heads/feat/auth"),
            2
        );

        let gh_log = fs::read_to_string(gh_log_path).unwrap();
        assert_eq!(gh_log.matches("pr reopen ").count(), 2);
        assert_eq!(gh_log.matches("pr ready ").count(), 2);
        assert_eq!(gh_log.matches("pr edit ").count(), 2);
        assert_eq!(
            git_stdout(
                repo,
                &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"]
            ),
            ""
        );
    });
}

#[test]
fn sync_skips_pull_request_repair_for_open_merged_or_retargeted_children() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_children(
            repo,
            &[
                ("feat/auth-api", "api.txt", "api\n", "feat: api"),
                ("feat/auth-ui", "ui.txt", "ui\n", "feat: ui"),
                ("feat/auth-tests", "tests.txt", "tests\n", "feat: tests"),
            ],
        );
        track_pull_request_number(repo, "feat/auth-api", 301);
        track_pull_request_number(repo, "feat/auth-ui", 302);
        track_pull_request_number(repo, "feat/auth-tests", 303);
        let remote_update_log = install_remote_update_logger(repo);

        let (path, gh_log_path) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "301" ]; then
  printf '{"number":301,"state":"OPEN","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-api","isDraft":false,"url":"https://github.com/acme/dig/pull/301"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "302" ]; then
  printf '{"number":302,"state":"CLOSED","mergedAt":"2026-03-26T12:00:00Z","baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/302"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "303" ]; then
  printf '{"number":303,"state":"CLOSED","mergedAt":null,"baseRefName":"main","headRefName":"feat/auth-tests","isDraft":false,"url":"https://github.com/acme/dig/pull/303"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$3" = "301" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

        let output = dig_with_input_and_env(
            repo,
            &["sync"],
            "y\nn\n",
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", gh_log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(!stdout.contains("Recovered pull requests:"));

        let gh_log = fs::read_to_string(gh_log_path).unwrap();
        assert!(gh_log.contains("pr view 301"));
        assert!(gh_log.contains("pr view 302"));
        assert!(gh_log.contains("pr view 303"));
        assert!(!gh_log.contains("pr reopen"));
        assert!(!gh_log.contains("pr ready"));
        assert_eq!(gh_log.matches("pr edit ").count(), 1);
        assert!(gh_log.contains("pr edit 301 --base main"));
        assert_eq!(
            count_remote_ref_updates(&remote_update_log, "refs/heads/feat/auth"),
            0
        );
    });
}

#[test]
fn sync_repairs_closed_child_pull_request_when_parent_branch_is_missing_locally() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/root"]);
        commit_file(repo, "root.txt", "root\n", "feat: root");
        git_ok(repo, &["push", "-u", "origin", "feat/root"]);
        track_pull_request_number(repo, "feat/root", 101);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        track_pull_request_number(repo, "feat/auth", 102);
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["push", "-u", "origin", "feat/auth-ui"]);
        track_pull_request_number(repo, "feat/auth-ui", 103);

        let parent_head_oid = git_stdout(repo, &["rev-parse", "feat/auth"]);
        let remote_repo = clone_origin(repo, "origin-worktree-missing-parent");
        git_ok(&remote_repo, &["checkout", "main"]);
        git_ok(&remote_repo, &["merge", "--squash", "origin/feat/root"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: merge root"],
        );
        git_ok(&remote_repo, &["push", "origin", "main"]);
        git_ok(&remote_repo, &["push", "origin", "--delete", "feat/root"]);
        git_ok(&remote_repo, &["push", "origin", "--delete", "feat/auth"]);

        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/root"]);
        git_ok(repo, &["branch", "-D", "feat/auth"]);
        set_branch_archived(repo, "feat/root", true);

        let remote_update_log = install_remote_update_logger(repo);
        let (path, gh_log_path) = install_fake_gh(
            repo,
            &format!(
                r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "102" ]; then
  printf '{{"number":102,"state":"MERGED","mergedAt":"2026-03-26T12:00:00Z","baseRefName":"feat/root","headRefName":"feat/auth","headRefOid":"{parent_head_oid}","isDraft":false,"url":"https://github.com/acme/dig/pull/102"}}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "103" ]; then
  printf '{{"number":103,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/103"}}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "reopen" ] && [ "$3" = "103" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "ready" ] && [ "$3" = "103" ] && [ "$4" = "--undo" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$3" = "103" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#
            ),
        );

        let output = dig_with_input_and_env(
            repo,
            &["sync"],
            "n\n",
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", gh_log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(stdout.contains("Recovered pull requests:"));
        assert!(stdout.contains(
            "- feat/auth-ui (#103): reopened as draft and retargeted from feat/auth to main"
        ));
        assert!(stdout.contains("Deleted locally and no longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));

        let gh_log = fs::read_to_string(gh_log_path).unwrap();
        assert!(gh_log.contains("pr view 102 --json"));
        assert!(gh_log.contains("pr view 103 --json"));
        assert!(gh_log.contains("pr reopen 103"));
        assert!(gh_log.contains("pr ready 103 --undo"));
        assert!(gh_log.contains("pr edit 103 --base main"));
        assert_eq!(
            count_remote_ref_updates(&remote_update_log, "refs/heads/feat/auth"),
            2
        );
        assert_eq!(
            git_stdout(
                repo,
                &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"]
            ),
            ""
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/root").is_some());
        assert!(find_archived_node(&state, "feat/auth").is_some());
    });
}

#[test]
fn sync_removes_local_parent_branch_after_repair_when_parent_was_merged_upstream() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/root"]);
        commit_file(repo, "root.txt", "root\n", "feat: root");
        git_ok(repo, &["push", "-u", "origin", "feat/root"]);
        track_pull_request_number(repo, "feat/root", 101);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        track_pull_request_number(repo, "feat/auth", 102);
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: ui");
        git_ok(repo, &["push", "-u", "origin", "feat/auth-ui"]);
        track_pull_request_number(repo, "feat/auth-ui", 103);

        let parent_head_oid = git_stdout(repo, &["rev-parse", "feat/auth"]);
        let remote_repo = clone_origin(repo, "origin-worktree-local-parent");
        git_ok(&remote_repo, &["checkout", "main"]);
        git_ok(&remote_repo, &["merge", "--squash", "origin/feat/root"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: merge root"],
        );
        git_ok(&remote_repo, &["push", "origin", "main"]);
        git_ok(&remote_repo, &["push", "origin", "--delete", "feat/root"]);
        git_ok(&remote_repo, &["push", "origin", "--delete", "feat/auth"]);

        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["branch", "-D", "feat/root"]);
        set_branch_archived(repo, "feat/root", true);

        let remote_update_log = install_remote_update_logger(repo);
        let (path, gh_log_path) = install_fake_gh(
            repo,
            &format!(
                r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "102" ]; then
  printf '{{"number":102,"state":"MERGED","mergedAt":"2026-03-26T12:00:00Z","baseRefName":"feat/root","headRefName":"feat/auth","headRefOid":"{parent_head_oid}","isDraft":false,"url":"https://github.com/acme/dig/pull/102"}}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "103" ]; then
  printf '{{"number":103,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/103"}}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "reopen" ] && [ "$3" = "103" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "ready" ] && [ "$3" = "103" ] && [ "$4" = "--undo" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$3" = "103" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#
            ),
        );

        let output = dig_with_input_and_env(
            repo,
            &["sync"],
            "n\n",
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", gh_log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(
            output.status.success(),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(stdout.contains("Recovered pull requests:"));
        assert!(stdout.contains("Deleted locally and no longer tracked by dig:"));
        assert!(stdout.contains("- feat/auth"));
        assert_eq!(git_stdout(repo, &["branch", "--list", "feat/auth"]), "");

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_archived_node(&state, "feat/auth").is_some());
        assert_eq!(
            count_remote_ref_updates(&remote_update_log, "refs/heads/feat/auth"),
            2
        );
    });
}

#[test]
fn sync_aborts_before_local_cleanup_when_pull_request_repair_fails() {
    with_temp_repo("dig-sync-cli", |repo| {
        setup_remotely_merged_root_branch_with_children(
            repo,
            &[("feat/auth-ui", "ui.txt", "ui\n", "feat: ui")],
        );
        track_pull_request_number(repo, "feat/auth-ui", 234);

        let (path, gh_log_path) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DIG_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "234" ]; then
  printf '{"number":234,"state":"CLOSED","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","isDraft":false,"url":"https://github.com/acme/dig/pull/234"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "reopen" ] && [ "$3" = "234" ]; then
  echo "boom" >&2
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        );

        let output = dig_with_env(
            repo,
            &["sync"],
            &[
                ("PATH", path.as_str()),
                ("DIG_TEST_GH_LOG", gh_log_path.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stderr.contains("failed to reopen tracked pull request #234 for 'feat/auth-ui'"));
        assert!(!stdout.contains("Merged branches ready to clean:"));
        assert!(!stdout.contains("Restacked:"));
        assert!(load_operation_json(repo).is_none());

        let state = load_state_json(repo);
        let parent = find_node(&state, "feat/auth").unwrap();
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["parent"]["kind"], "branch");
        assert_eq!(child["parent"]["node_id"], parent["id"]);
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
    });
}

#[test]
fn sync_cleans_middle_branch_merged_remotely_and_excludes_it_from_remote_pushes() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "api.txt", "api\n", "feat: auth api");
        git_ok(repo, &["push", "-u", "origin", "feat/auth-api"]);
        dig_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(repo, "tests.txt", "tests\n", "feat: tests");

        let remote_repo = clone_origin(repo, "origin-worktree");
        let tracking_ref = "origin/feat/auth";
        git_ok(&remote_repo, &["checkout", "-b", "feat/auth", tracking_ref]);
        git_ok(&remote_repo, &["merge", "--squash", "origin/feat/auth-api"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: merge auth api"],
        );
        git_ok(&remote_repo, &["push", "origin", "feat/auth"]);

        let output = dig_with_input(repo, &["sync"], "y\nn\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("- feat/auth-api merged into feat/auth"));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth"));
        assert!(stdout.contains("Remote branches to update:"));
        assert!(stdout.contains("- create feat/auth-api-tests on origin"));
        assert!(!stdout.contains("- create feat/auth-api on origin"));
        assert_eq!(
            git_stdout(
                repo,
                &["merge-base", "origin/feat/auth", "feat/auth-api-tests"]
            ),
            git_stdout(repo, &["rev-parse", "origin/feat/auth"])
        );

        let state = load_state_json(repo);
        let tests_branch = find_node(&state, "feat/auth-api-tests").unwrap();
        let parent = find_node(&state, "feat/auth").unwrap();
        assert_eq!(tests_branch["base_ref"], "feat/auth");
        assert_eq!(tests_branch["parent"]["kind"], "branch");
        assert_eq!(tests_branch["parent"]["node_id"], parent["id"]);
        assert!(find_node(&state, "feat/auth-api").is_none());
        assert!(find_archived_node(&state, "feat/auth-api").is_some());
    });
}

#[test]
fn sync_prompts_to_push_missing_remote_branch_after_local_sync() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");

        let output = dig_with_input(repo, &["sync"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Local stacks are already in sync."));
        assert!(stdout.contains("Remote branches to update:"));
        assert!(stdout.contains("- create feat/auth on origin"));
        assert!(stdout.contains("Push these remote updates? [y/N]"));
        assert!(stdout.contains("Updated remote branches:"));
        assert!(stdout.contains("- created feat/auth on origin"));
        assert!(
            git_stdout(
                repo,
                &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"]
            )
            .contains("refs/heads/feat/auth")
        );
    });
}

#[test]
fn sync_prompts_to_push_active_branch_ahead_of_remote() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth v2\n", "feat: auth follow-up");

        let output = dig_with_input(repo, &["sync"], "y\n");
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(output.status.success());
        assert!(stdout.contains("Local stacks are already in sync."));
        assert!(stdout.contains("Remote branches to update:"));
        assert!(stdout.contains("- push feat/auth on origin"));
        assert!(stdout.contains("Push these remote updates? [y/N]"));
        assert!(stdout.contains("Updated remote branches:"));
        assert!(stdout.contains("- pushed feat/auth on origin"));
        assert_eq!(
            git_stdout(repo, &["rev-parse", "feat/auth"]),
            git_stdout(repo, &["rev-parse", "origin/feat/auth"])
        );
    });
}

#[test]
fn sync_continues_paused_remote_cleanup_with_stored_remote_rebase_target() {
    with_temp_repo("dig-sync-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: parent");
        git_ok(repo, &["push", "-u", "origin", "feat/auth"]);
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "conflict.txt", "child\n", "feat: child");

        let remote_repo = clone_origin(repo, "origin-worktree");
        git_ok(&remote_repo, &["checkout", "main"]);
        git_ok(&remote_repo, &["merge", "--squash", "origin/feat/auth"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: merge auth"],
        );
        std::fs::write(remote_repo.join("conflict.txt"), "remote\n").unwrap();
        git_ok(&remote_repo, &["add", "conflict.txt"]);
        git_ok(
            &remote_repo,
            &["commit", "--quiet", "-m", "feat: remote main follow-up"],
        );
        git_ok(&remote_repo, &["push", "origin", "main"]);

        let paused = dig_with_input(repo, &["sync"], "y\n");
        let stderr = String::from_utf8(paused.stderr).unwrap();

        assert!(!paused.status.success());
        assert!(stderr.contains("dig sync --continue"));

        let operation = load_operation_json(repo).unwrap();
        assert_eq!(operation["origin"]["type"].as_str(), Some("clean"));
        assert_eq!(
            operation["origin"]["current_candidate"]["kind"]["kind"].as_str(),
            Some("integrated_into_parent")
        );
        assert_eq!(
            operation["origin"]["current_candidate"]["kind"]["parent_base"]["new_base_branch_name"]
                .as_str(),
            Some("main")
        );
        assert_eq!(
            operation["origin"]["current_candidate"]["kind"]["parent_base"]["new_base_ref"]
                .as_str(),
            Some("origin/main")
        );

        std::fs::write(repo.join("conflict.txt"), "resolved\n").unwrap();
        git_ok(repo, &["add", "conflict.txt"]);

        let resumed = dig_with_input(repo, &["sync", "--continue"], "n\n");
        let stdout = strip_ansi(&String::from_utf8(resumed.stdout).unwrap());

        assert!(resumed.status.success());
        assert!(stdout.contains("Deleted:"));
        assert!(stdout.contains("- feat/auth"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto main"));
        assert_eq!(
            git_stdout(repo, &["merge-base", "origin/main", "feat/auth-ui"]),
            git_stdout(repo, &["rev-parse", "origin/main"])
        );

        let state = load_state_json(repo);
        let child = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(child["base_ref"], "main");
        assert_eq!(child["parent"]["kind"], "trunk");
        assert!(find_node(&state, "feat/auth").is_none());
        assert!(load_operation_json(repo).is_none());
    });
}
