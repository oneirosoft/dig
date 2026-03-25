#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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

pub fn append_to_file(repo: &Path, file_name: &str, contents: &str) {
    let path = repo.join(file_name);
    let mut existing = fs::read_to_string(&path).unwrap();
    existing.push_str(contents);
    fs::write(path, existing).unwrap();
}

pub fn write_file(repo: &Path, file_name: &str, contents: &str) {
    fs::write(repo.join(file_name), contents).unwrap();
}

pub fn dig(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dig"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}

pub fn dig_with_input(repo: &Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dig"))
        .current_dir(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();

    child.wait_with_output().unwrap()
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

pub fn load_operation_json(repo: &Path) -> Option<Value> {
    let path = repo.join(".git/dig/operation.json");
    if !path.exists() {
        return None;
    }

    Some(serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap())
}

pub fn find_node<'a>(state: &'a Value, branch_name: &str) -> Option<&'a Value> {
    state["nodes"].as_array().and_then(|nodes| {
        nodes.iter().find(|node| {
            node["branch_name"].as_str() == Some(branch_name)
                && node["archived"].as_bool() == Some(false)
        })
    })
}

pub fn find_archived_node<'a>(state: &'a Value, branch_name: &str) -> Option<&'a Value> {
    state["nodes"].as_array().and_then(|nodes| {
        nodes.iter().find(|node| {
            node["branch_name"].as_str() == Some(branch_name)
                && node["archived"].as_bool() == Some(true)
        })
    })
}

pub fn load_events_json(repo: &Path) -> Vec<Value> {
    fs::read_to_string(repo.join(".git/dig/events.ndjson"))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
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

pub fn active_rebase_head_name(repo: &Path) -> String {
    for relative_path in ["rebase-merge/head-name", "rebase-apply/head-name"] {
        let path = repo.join(".git").join(relative_path);
        if path.exists() {
            return fs::read_to_string(path).unwrap();
        }
    }

    panic!("expected an active rebase head-name file");
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_basic_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[32mhello\u{1b}[0m"), "hello");
    }
}
