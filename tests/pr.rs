mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use support::{
    dgr_ok, dgr_ok_with_env, dgr_with_input_and_env, find_node, git_binary_path, git_ok,
    git_stdout, initialize_main_repo, install_fake_executable, load_events_json, load_state_json,
    path_with_prepend, strip_ansi, with_temp_repo,
};

fn install_fake_gh(
    repo: &Path,
    unix_script: &str,
    windows_script: &str,
) -> (PathBuf, String, String, String) {
    let bin_dir = repo.join("fake-bin");
    let script = if cfg!(windows) {
        windows_script
    } else {
        unix_script
    };
    install_fake_executable(&bin_dir, "gh", script);

    let path = path_with_prepend(&bin_dir);
    let log_path = repo.join("gh.log").display().to_string();
    let gh_bin = bin_dir
        .join(if cfg!(windows) { "gh.cmd" } else { "gh" })
        .display()
        .to_string();

    (bin_dir, path, log_path, gh_bin)
}

fn clear_log(path: &str) {
    fs::write(path, "").unwrap();
}

fn track_pull_request_number(repo: &Path, branch_name: &str, number: u64) {
    let state_path = repo.join(".git/.dagger/state.json");
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

fn initialize_origin_remote(repo: &Path) {
    git_ok(repo, &["init", "--bare", "origin.git"]);
    git_ok(repo, &["remote", "add", "origin", "origin.git"]);
}

#[test]
fn pr_creates_root_pull_request_tracks_number_and_updates_tree() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/123\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/123
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &[
                "pr",
                "--title",
                "feat-auth",
                "--body",
                "body-text",
                "--draft",
            ],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created pull request #123 for 'feat/auth' into 'main'."));
        assert_eq!(
            stdout
                .matches("https://github.com/oneirosoft/dagger/pull/123")
                .count(),
            1
        );

        let state = load_state_json(repo);
        let node = find_node(&state, "feat/auth").unwrap();
        assert_eq!(node["pull_request"]["number"], 123);

        let tree_output = dgr_ok(repo, &["tree"]);
        let tree_stdout = strip_ansi(&String::from_utf8(tree_output.stdout).unwrap());
        assert!(tree_stdout.contains("feat/auth (#123)"));

        let events = load_events_json(repo);
        assert!(events.iter().any(|event| {
            event["type"].as_str() == Some("branch_pull_request_tracked")
                && event["branch_name"].as_str() == Some("feat/auth")
                && event["pull_request"]["number"].as_u64() == Some(123)
                && event["source"].as_str() == Some("created")
        }));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(
            gh_log.contains("pr list --head feat/auth --state open --json number,baseRefName,url")
        );
        assert!(
            gh_log.contains("pr create --base main --title feat-auth --body body-text --draft")
        );
    });
}

#[test]
fn pr_merge_retargets_open_child_pull_request_before_merging_parent() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        track_pull_request_number(repo, "feat/auth", 123);
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        track_pull_request_number(repo, "feat/auth-ui", 124);
        git_ok(repo, &["checkout", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "124" ]; then
  printf '{"number":124,"state":"OPEN","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","headRefOid":"abc123","isDraft":false,"url":"https://github.com/oneirosoft/dagger/pull/124"}\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ] && [ "$3" = "124" ] && [ "$4" = "--base" ] && [ "$5" = "main" ]; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$3" = "123" ] && [ "$4" = "--squash" ] && [ "$5" = "--delete-branch" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="view" if "%3"=="124" (
  echo {"number":124,"state":"OPEN","mergedAt":null,"baseRefName":"feat/auth","headRefName":"feat/auth-ui","headRefOid":"abc123","isDraft":false,"url":"https://github.com/oneirosoft/dagger/pull/124"}
  exit /b 0
)
if "%1"=="pr" if "%2"=="edit" if "%3"=="124" if "%4"=="--base" if "%5"=="main" (
  exit /b 0
)
if "%1"=="pr" if "%2"=="merge" if "%3"=="123" if "%4"=="--squash" if "%5"=="--delete-branch" (
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr", "merge"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        let gh_log = fs::read_to_string(log_path).unwrap();

        assert!(stdout.contains("Retargeted child pull requests:"));
        assert!(stdout.contains("- #124 for feat/auth-ui to main"));
        assert!(stdout.contains("Merged pull request #123 for 'feat/auth' into 'main'."));

        let lines = gh_log.lines().collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                "pr view 124 --json number,state,mergedAt,baseRefName,headRefName,headRefOid,isDraft,url",
                "pr edit 124 --base main",
                "pr merge 123 --squash --delete-branch",
            ]
        );
    });
}

#[test]
fn pr_creates_child_pull_request_against_tracked_parent() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        dgr_ok(repo, &["branch", "feat/auth-api"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/234\n'
  exit 0
fi
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/234
  exit /b 0
)
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created pull request #234 for 'feat/auth-api' into 'feat/auth'."));
        assert_eq!(
            stdout
                .matches("https://github.com/oneirosoft/dagger/pull/234")
                .count(),
            1
        );

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(gh_log.contains("pr create --base feat/auth"));
    });
}

#[test]
fn pr_defaults_body_to_title_when_body_is_omitted() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/321\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/321
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr", "--title", "feat-auth", "--draft"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created pull request #321 for 'feat/auth' into 'main'."));
        assert_eq!(
            stdout
                .matches("https://github.com/oneirosoft/dagger/pull/321")
                .count(),
            1
        );

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(
            gh_log.contains("pr create --base main --title feat-auth --body feat-auth --draft")
        );
    });
}

#[test]
fn pr_adopts_matching_open_pull_request_without_creating_another() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[{"number":345,"baseRefName":"main","url":"https://github.com/oneirosoft/dagger/pull/345"}]\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo [{"number":345,"baseRefName":"main","url":"https://github.com/oneirosoft/dagger/pull/345"}]
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(
            stdout.contains("Tracking existing pull request #345 for 'feat/auth' into 'main'.")
        );

        let state = load_state_json(repo);
        let node = find_node(&state, "feat/auth").unwrap();
        assert_eq!(node["pull_request"]["number"], 345);

        let events = load_events_json(repo);
        assert!(events.iter().any(|event| {
            event["type"].as_str() == Some("branch_pull_request_tracked")
                && event["source"].as_str() == Some("adopted")
        }));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(!gh_log.contains("pr create"));
    });
}

#[test]
fn pr_is_idempotent_when_branch_already_tracks_pull_request() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (bin_dir, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/456\n'
  exit 0
fi
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/456
  exit /b 0
)
exit /b 1
"#,
        );

        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        install_fake_executable(
            &bin_dir,
            "gh",
            if cfg!(windows) {
                "@echo off\r\necho %* >> \"%DGR_TEST_GH_LOG%\"\r\necho gh should not have been called 1>&2\r\nexit /b 99\r\n"
            } else {
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$DGR_TEST_GH_LOG\"\necho \"gh should not have been called\" >&2\nexit 99\n"
            },
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Branch 'feat/auth' already tracks pull request #456."));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert_eq!(gh_log.lines().count(), 3);
    });
}

#[test]
fn pr_with_view_only_opens_tracked_pull_request_in_browser() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (bin_dir, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/456\n'
  exit 0
fi
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/456
  exit /b 0
)
exit /b 1
"#,
        );

        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        clear_log(&log_path);
        install_fake_executable(
            &bin_dir,
            "gh",
            if cfg!(windows) {
                "@echo off\r\necho %* >> \"%DGR_TEST_GH_LOG%\"\r\nif \"%1\"==\"pr\" if \"%2\"==\"view\" if \"%3\"==\"456\" if \"%4\"==\"--web\" (\r\n  exit /b 0\r\n)\r\necho unexpected gh args: %* 1>&2\r\nexit /b 1\r\n"
            } else {
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$DGR_TEST_GH_LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$3\" = \"456\" ] && [ \"$4\" = \"--web\" ]; then\n  exit 0\nfi\necho \"unexpected gh args: $*\" >&2\nexit 1\n"
            },
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr", "--view"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        assert!(String::from_utf8(output.stdout).unwrap().trim().is_empty());
        let gh_log = fs::read_to_string(log_path).unwrap();
        assert_eq!(gh_log.trim(), "pr view 456 --web");
    });
}

#[test]
fn pr_with_create_and_view_opens_browser_after_tracking() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/123\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "123" ] && [ "$4" = "--web" ]; then
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/123
  exit /b 0
)
if "%1"=="pr" if "%2"=="view" if "%3"=="123" if "%4"=="--web" (
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr", "--title", "feat-auth", "--view"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created pull request #123 for 'feat/auth' into 'main'."));
        assert_eq!(
            stdout
                .matches("https://github.com/oneirosoft/dagger/pull/123")
                .count(),
            1
        );

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert_eq!(
            gh_log.lines().collect::<Vec<_>>(),
            vec![
                "pr list --head feat/auth --state open --json number,baseRefName,url",
                "pr list --head feat/auth --state open --json number,baseRefName,url",
                "pr create --base main --title feat-auth --body feat-auth",
                "pr view 123 --web",
            ]
        );
    });
}

#[test]
fn pr_prompts_to_push_branch_before_creating_pull_request() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/777\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/777
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_with_input_and_env(
            repo,
            &["pr"],
            "y\n",
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        assert!(output.status.success());

        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        assert!(stdout.contains("Push it and create the pull request? [y/N]"));
        assert!(stdout.contains("Created pull request #777 for 'feat/auth' into 'main'."));
        assert_eq!(
            stdout
                .matches("https://github.com/oneirosoft/dagger/pull/777")
                .count(),
            1
        );

        let remote_ref = git_stdout(
            repo,
            &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"],
        );
        assert!(remote_ref.contains("refs/heads/feat/auth"));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert_eq!(
            gh_log.lines().collect::<Vec<_>>(),
            vec![
                "pr list --head feat/auth --state open --json number,baseRefName,url",
                "pr list --head feat/auth --state open --json number,baseRefName,url",
                "pr create --base main",
            ]
        );
    });
}

#[test]
fn pr_declining_push_skips_pull_request_creation() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        initialize_origin_remote(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = dgr_with_input_and_env(
            repo,
            &["pr"],
            "n\n",
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        assert!(output.status.success());

        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());
        assert!(stdout.contains(
            "Did not create pull request because 'feat/auth' is not pushed to 'origin'."
        ));

        assert!(
            git_stdout(
                repo,
                &["ls-remote", "--heads", "origin", "refs/heads/feat/auth"]
            )
            .is_empty()
        );
        assert_eq!(
            fs::read_to_string(log_path)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec!["pr list --head feat/auth --state open --json number,baseRefName,url"]
        );
    });
}

#[test]
fn pr_list_renders_open_tracked_pull_requests_in_lineage_order() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);

        let (bin_dir, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  current_branch="$(git branch --show-current)"
  if [ "$current_branch" = "feat/auth" ]; then
    printf 'https://github.com/oneirosoft/dagger/pull/101\n'
    exit 0
  fi
  if [ "$current_branch" = "feat/auth-ui" ]; then
    printf 'https://github.com/oneirosoft/dagger/pull/102\n'
    exit 0
  fi
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  for /f "delims=" %%b in ('git branch --show-current') do set "CURRENT_BRANCH=%%b"
  if "%CURRENT_BRANCH%"=="feat/auth" (
    echo https://github.com/oneirosoft/dagger/pull/101
    exit /b 0
  )
  if "%CURRENT_BRANCH%"=="feat/auth-ui" (
    echo https://github.com/oneirosoft/dagger/pull/102
    exit /b 0
  )
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        dgr_ok(repo, &["branch", "feat/auth"]);
        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        clear_log(&log_path);
        install_fake_executable(
            &bin_dir,
            "gh",
            if cfg!(windows) {
                "@echo off\r\necho %* >> \"%DGR_TEST_GH_LOG%\"\r\nif \"%1\"==\"pr\" if \"%2\"==\"list\" if \"%3\"==\"--state\" if \"%4\"==\"open\" (\r\n  echo [{\"number\":101,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/101\"},{\"number\":102,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/102\"},{\"number\":999,\"title\":\"External PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/999\"}]\r\n  exit /b 0\r\n)\r\necho unexpected gh args: %* 1>&2\r\nexit /b 1\r\n"
            } else {
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$DGR_TEST_GH_LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ] && [ \"$3\" = \"--state\" ] && [ \"$4\" = \"open\" ]; then\n  printf '[{\"number\":101,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/101\"},{\"number\":102,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/102\"},{\"number\":999,\"title\":\"External PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/999\"}]\\n'\n  exit 0\nfi\necho \"unexpected gh args: $*\" >&2\nexit 1\n"
            },
        );

        let output = dgr_ok_with_env(
            repo,
            &["pr", "list"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("main"));
        assert!(stdout.contains("#101: Auth PR - https://github.com/oneirosoft/dagger/pull/101"));
        assert!(
            stdout.contains("#102: Auth UI PR - https://github.com/oneirosoft/dagger/pull/102")
        );
        assert!(!stdout.contains("#999: External PR"));
    });
}

#[test]
fn pr_list_with_view_opens_each_listed_pull_request() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (bin_dir, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  printf 'https://github.com/oneirosoft/dagger/pull/301\n'
  exit 0
fi
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo https://github.com/oneirosoft/dagger/pull/301
  exit /b 0
)
exit /b 1
"#,
        );

        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        install_fake_executable(
            &bin_dir,
            "gh",
            if cfg!(windows) {
                "@echo off\r\necho %* >> \"%DGR_TEST_GH_LOG%\"\r\nif \"%1\"==\"pr\" if \"%2\"==\"list\" (\r\n  for /f \"delims=\" %%b in ('git branch --show-current') do set \"CURRENT_BRANCH=%%b\"\r\n  if \"%CURRENT_BRANCH%\"==\"feat/auth-ui\" if \"%3\"==\"--head\" (\r\n    echo []\r\n    exit /b 0\r\n  )\r\n  echo [{\"number\":301,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/301\"},{\"number\":302,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/302\"}]\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"pr\" if \"%2\"==\"create\" (\r\n  echo https://github.com/oneirosoft/dagger/pull/302\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"pr\" if \"%2\"==\"view\" if \"%4\"==\"--web\" (\r\n  exit /b 0\r\n)\r\necho unexpected gh args: %* 1>&2\r\nexit /b 1\r\n"
            } else {
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$DGR_TEST_GH_LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n  current_branch=\"$(git branch --show-current)\"\n  if [ \"$current_branch\" = \"feat/auth-ui\" ] && [ \"$3\" = \"--head\" ]; then\n    printf '[]\\n'\n    exit 0\n  fi\n  printf '[{\"number\":301,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/301\"},{\"number\":302,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/302\"}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then\n  printf 'https://github.com/oneirosoft/dagger/pull/302\\n'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$4\" = \"--web\" ]; then\n  exit 0\nfi\necho \"unexpected gh args: $*\" >&2\nexit 1\n"
            },
        );
        dgr_ok_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        clear_log(&log_path);
        install_fake_executable(
            &bin_dir,
            "gh",
            if cfg!(windows) {
                "@echo off\r\necho %* >> \"%DGR_TEST_GH_LOG%\"\r\nif \"%1\"==\"pr\" if \"%2\"==\"list\" if \"%3\"==\"--state\" if \"%4\"==\"open\" (\r\n  echo [{\"number\":301,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/301\"},{\"number\":302,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/302\"}]\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"pr\" if \"%2\"==\"view\" if \"%4\"==\"--web\" (\r\n  exit /b 0\r\n)\r\necho unexpected gh args: %* 1>&2\r\nexit /b 1\r\n"
            } else {
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$DGR_TEST_GH_LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ] && [ \"$3\" = \"--state\" ] && [ \"$4\" = \"open\" ]; then\n  printf '[{\"number\":301,\"title\":\"Auth PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/301\"},{\"number\":302,\"title\":\"Auth UI PR\",\"url\":\"https://github.com/oneirosoft/dagger/pull/302\"}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$4\" = \"--web\" ]; then\n  exit 0\nfi\necho \"unexpected gh args: $*\" >&2\nexit 1\n"
            },
        );

        dgr_ok_with_env(
            repo,
            &["pr", "list", "--view"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        assert_eq!(
            fs::read_to_string(log_path)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![
                "pr list --state open --json number,title,url",
                "pr view 301 --web",
                "pr view 302 --web",
            ]
        );
    });
}

#[test]
fn pr_rejects_existing_open_pull_request_with_wrong_base() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[{"number":567,"baseRefName":"develop","url":"https://github.com/oneirosoft/dagger/pull/567"}]\n'
  exit 0
fi
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo [{"number":567,"baseRefName":"develop","url":"https://github.com/oneirosoft/dagger/pull/567"}]
  exit /b 0
)
exit /b 1
"#,
        );

        let output = support::dgr_with_env(
            repo,
            &["pr"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("expects base 'main'"));

        let state = load_state_json(repo);
        let node = find_node(&state, "feat/auth").unwrap();
        assert!(node["pull_request"].is_null());
    });
}

#[test]
fn pr_reports_missing_gh_cli() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let bin_dir = repo.join("fake-bin");
        let git_path = git_binary_path();
        install_fake_executable(
            &bin_dir,
            "git",
            &if cfg!(windows) {
                format!("@echo off\r\n\"{}\" %*\r\n", git_path)
            } else {
                format!("#!/bin/sh\nset -eu\nexec \"{}\" \"$@\"\n", git_path)
            },
        );
        let path = bin_dir.display().to_string();

        let output = support::dgr_with_env(repo, &["pr"], &[("PATH", path.as_str())]);

        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("gh CLI is not installed or not found on PATH"));
    });
}

#[test]
fn pr_hides_gh_usage_output_when_create_fails() {
    with_temp_repo("dgr-pr-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path, log_path, gh_bin) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$DGR_TEST_GH_LOG"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  cat >&2 <<'EOF'
must provide `--title` and `--body` (or `--fill`)

Usage:  gh pr create [flags]

Flags:
  -b, --body string
EOF
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            r#"@echo off
echo %* >> "%DGR_TEST_GH_LOG%"
if "%1"=="pr" if "%2"=="list" (
  echo []
  exit /b 0
)
if "%1"=="pr" if "%2"=="create" (
  echo must provide `--title` and `--body` ^(or `--fill`^) 1>&2
  echo. 1>&2
  echo Usage:  gh pr create [flags] 1>&2
  echo. 1>&2
  echo Flags: 1>&2
  echo   -b, --body string 1>&2
  exit /b 1
)
echo unexpected gh args: %* 1>&2
exit /b 1
"#,
        );

        let output = support::dgr_with_env(
            repo,
            &["pr", "--title", "feat-auth", "--draft"],
            &[
                ("PATH", path.as_str()),
                ("DGR_TEST_GH_LOG", log_path.as_str()),
                ("DAGGER_GH_BIN", gh_bin.as_str()),
            ],
        );

        assert!(!output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().trim().is_empty());

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            stderr
                .matches("must provide `--title` and `--body`")
                .count(),
            1
        );
        assert!(stderr.contains("gh pr create failed: must provide `--title` and `--body`"));
        assert!(!stderr.contains("Usage:"));
        assert!(!stderr.contains("Flags:"));

        let gh_log = fs::read_to_string(log_path).unwrap();
        assert!(
            gh_log.contains("pr create --base main --title feat-auth --body feat-auth --draft")
        );
    });
}
