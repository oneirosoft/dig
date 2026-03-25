#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use uuid::Uuid;

pub fn with_temp_repo(prefix: &str, test: impl FnOnce(&Path)) {
    let repo_dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir_all(&repo_dir).unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        test(&repo_dir);
    }));

    fs::remove_dir_all(&repo_dir).unwrap();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

pub fn initialize_main_repo(repo: &Path) {
    git_ok(repo, &["init", "--quiet"]);
    git_ok(repo, &["checkout", "-b", "main"]);
    git_ok(repo, &["config", "user.name", "Dig Test"]);
    git_ok(repo, &["config", "user.email", "dig@example.com"]);
    git_ok(repo, &["config", "commit.gpgsign", "false"]);
    commit_file(repo, "README.md", "root\n", "chore: init");
}

pub fn commit_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
    fs::write(repo.join(file_name), contents).unwrap();
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

pub fn overwrite_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
    fs::write(repo.join(file_name), contents).unwrap();
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

pub fn dig(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dig"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}

pub fn dig_ok(repo: &Path, args: &[&str]) -> Output {
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

pub fn git_ok(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .unwrap();

    assert!(status.success(), "git {:?} failed", args);
}

pub fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();

    assert!(output.status.success(), "git {:?} failed", args);

    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub fn load_state_json(repo: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(repo.join(".git/dig/state.json")).unwrap()).unwrap()
}

pub fn find_node<'a>(state: &'a Value, branch_name: &str) -> Option<&'a Value> {
    state["nodes"].as_array().and_then(|nodes| {
        nodes.iter().find(|node| {
            node["branch_name"].as_str() == Some(branch_name)
                && node["archived"].as_bool() == Some(false)
        })
    })
}

pub fn events_contain_type(repo: &Path, event_type: &str) -> bool {
    fs::read_to_string(repo.join(".git/dig/events.ndjson"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .any(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|event| event["type"].as_str().map(str::to_string))
                .as_deref()
                == Some(event_type)
        })
}

pub fn strip_ansi(text: &str) -> String {
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
