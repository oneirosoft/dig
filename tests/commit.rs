use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use uuid::Uuid;

#[test]
fn restacks_tracked_descendants_after_commit_and_preserves_store_files() {
    with_temp_repo(|repo| {
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

        let state_before = fs::read_to_string(repo.join(".git/dig/state.json")).unwrap();
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
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/state.json")).unwrap(),
            state_before
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}

#[test]
fn restacks_child_after_amend_using_old_head_oid() {
    with_temp_repo(|repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dig_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "feat/auth"]);

        let state_before = fs::read_to_string(repo.join(".git/dig/state.json")).unwrap();
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
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/state.json")).unwrap(),
            state_before
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}

#[test]
fn leaves_rebase_open_on_conflicting_child_after_commit() {
    with_temp_repo(|repo| {
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

        let state_before = fs::read_to_string(repo.join(".git/dig/state.json")).unwrap();
        let events_before = fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap();

        write_file(repo, "shared.txt", "parent\n");
        git_ok(repo, &["add", "shared.txt"]);

        let output = dig(repo, &["commit", "-m", "feat: parent follow-up"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let stderr = String::from_utf8(output.stderr).unwrap();

        assert!(!output.status.success());
        assert!(stdout.contains("feat: parent follow-up"));
        assert!(stderr.contains("could not apply"));
        assert!(repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists());
        assert!(
            active_rebase_head_name(repo).contains("feat/auth-ui"),
            "expected rebase head-name to reference feat/auth-ui"
        );
        assert_eq!(
            git_stdout(repo, &["log", "-1", "--format=%s", "feat/auth"]),
            "feat: parent follow-up"
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/state.json")).unwrap(),
            state_before
        );
        assert_eq!(
            fs::read_to_string(repo.join(".git/dig/events.ndjson")).unwrap(),
            events_before
        );
    });
}

fn with_temp_repo(test: impl FnOnce(&Path)) {
    let repo_dir = std::env::temp_dir().join(format!("dig-commit-cli-{}", Uuid::new_v4()));
    fs::create_dir_all(&repo_dir).unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        test(&repo_dir);
    }));

    fs::remove_dir_all(&repo_dir).unwrap();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

fn initialize_main_repo(repo: &Path) {
    git_ok(repo, &["init", "--quiet"]);
    git_ok(repo, &["checkout", "-b", "main"]);
    git_ok(repo, &["config", "user.name", "Dig Test"]);
    git_ok(repo, &["config", "user.email", "dig@example.com"]);
    git_ok(repo, &["config", "commit.gpgsign", "false"]);
    commit_file(repo, "README.md", "root\n", "chore: init");
}

fn commit_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
    write_file(repo, file_name, contents);
    git_ok(repo, &["add", file_name]);
    git_ok(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

fn append_to_file(repo: &Path, file_name: &str, contents: &str) {
    let path = repo.join(file_name);
    let mut existing = fs::read_to_string(&path).unwrap();
    existing.push_str(contents);
    fs::write(path, existing).unwrap();
}

fn write_file(repo: &Path, file_name: &str, contents: &str) {
    fs::write(repo.join(file_name), contents).unwrap();
}

fn dig(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dig"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}

fn dig_ok(repo: &Path, args: &[&str]) -> Output {
    let output = dig(repo, args);
    assert!(
        output.status.success(),
        "dig {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_ok(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .unwrap();

    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();

    assert!(output.status.success(), "git {:?} failed", args);

    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn strip_ansi(text: &str) -> String {
    let mut stripped = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }

        stripped.push(ch);
    }

    stripped
}

fn active_rebase_head_name(repo: &Path) -> String {
    for relative_path in ["rebase-merge/head-name", "rebase-apply/head-name"] {
        let path = repo.join(".git").join(relative_path);
        if path.exists() {
            return fs::read_to_string(path).unwrap();
        }
    }

    panic!("expected an active rebase head-name file");
}
