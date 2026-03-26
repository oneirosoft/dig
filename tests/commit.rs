mod support;

use std::fs;

use support::{
    active_rebase_head_name, append_to_file, commit_file, dig, dig_ok, git_ok, git_stdout,
    initialize_main_repo, load_operation_json, load_state_json, strip_ansi, with_temp_repo,
    write_file,
};

#[test]
fn restacks_tracked_descendants_after_commit_and_preserves_store_files() {
    with_temp_repo("dig-commit-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-api"]);
        commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");
        dig_ok(repo, &["branch", "feat/auth-api-tests"]);
        commit_file(
            repo,
            "auth-api-tests.txt",
            "tests\n",
            "feat: auth api tests",
        );
        git_ok(repo, &["checkout", "feat/auth"]);

        let events_before = fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap();

        append_to_file(repo, "auth.txt", "follow-up\n");
        git_ok(repo, &["add", "auth.txt"]);

        let output = dig_ok(repo, &["commit", "--message", "feat: auth follow-up"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("feat: auth follow-up"));
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-api onto feat/auth"));
        assert!(stdout.contains("- feat/auth-api-tests onto feat/auth-api"));
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-api"]),
            git_stdout(repo, &["rev-parse", "feat/auth"])
        );
        assert_eq!(
            git_stdout(
                repo,
                &["merge-base", "feat/auth-api", "feat/auth-api-tests"]
            ),
            git_stdout(repo, &["rev-parse", "feat/auth-api"])
        );
        let state = load_state_json(repo);
        assert_eq!(
            state["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["branch_name"].as_str() == Some("feat/auth"))
                .unwrap()["divergence_state"]["kind"]
                .as_str(),
            Some("diverged")
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}

#[test]
fn restacks_child_after_amend_using_old_head_oid() {
    with_temp_repo("dig-commit-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "feat/auth"]);

        let events_before = fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap();
        let old_head = git_stdout(repo, &["rev-parse", "feat/auth"]);

        append_to_file(repo, "auth.txt", "amend\n");
        git_ok(repo, &["add", "auth.txt"]);

        let output = dig_ok(repo, &["commit", "--amend", "--no-edit"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let new_head = git_stdout(repo, &["rev-parse", "feat/auth"]);

        assert_ne!(old_head, new_head);
        assert!(stdout.contains("Restacked:"));
        assert!(stdout.contains("- feat/auth-ui onto feat/auth"));
        assert_eq!(
            git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]),
            new_head
        );
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");
        let state = load_state_json(repo);
        assert_eq!(
            state["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["branch_name"].as_str() == Some("feat/auth"))
                .unwrap()["divergence_state"]["kind"]
                .as_str(),
            Some("diverged")
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}

#[test]
fn leaves_rebase_open_on_conflicting_child_after_commit() {
    with_temp_repo("dig-commit-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "shared.txt", "base\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        write_file(repo, "shared.txt", "child\n");
        git_ok(repo, &["add", "shared.txt"]);
        git_ok(
            repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "feat: child change",
            ],
        );
        git_ok(repo, &["checkout", "feat/auth"]);

        let events_before = fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap();

        write_file(repo, "shared.txt", "parent\n");
        git_ok(repo, &["add", "shared.txt"]);

        let output = dig(repo, &["commit", "-m", "feat: parent follow-up"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stdout.contains("feat: parent follow-up"));
        assert!(stderr.contains("could not apply"));
        assert!(stderr.contains("dig sync --continue"));
        assert!(repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists());
        assert!(load_operation_json(repo).is_some());
        assert!(
            active_rebase_head_name(repo).contains("feat/auth-ui"),
            "expected rebase head-name to reference feat/auth-ui"
        );
        assert_eq!(
            git_stdout(repo, &["log", "-1", "--format=%s", "feat/auth"]),
            "feat: parent follow-up"
        );
        let state = load_state_json(repo);
        assert_eq!(
            state["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["branch_name"].as_str() == Some("feat/auth"))
                .unwrap()["divergence_state"]["kind"]
                .as_str(),
            Some("diverged")
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}
