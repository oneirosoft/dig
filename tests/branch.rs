mod support;

use std::path::{Path, PathBuf};

use support::{
    dgr_ok, dgr_ok_with_env, find_node, initialize_main_repo, install_fake_executable,
    load_state_json, path_with_prepend, strip_ansi, with_temp_repo,
};

fn install_fake_gh(repo: &Path, script: &str) -> (PathBuf, String) {
    let bin_dir = repo.join("fake-bin");
    install_fake_executable(&bin_dir, "gh", script);

    let path = path_with_prepend(&bin_dir);

    (bin_dir, path)
}

#[test]
fn branch_command_renders_marked_lineage_and_tracks_parent() {
    with_temp_repo("dgr-branch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);

        let output = dgr_ok(repo, &["branch", "feat/auth"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created and switched to 'feat/auth'."));
        assert!(stdout.contains("✓ feat/auth\n│ \n* main"));

        let state = load_state_json(repo);
        let node = find_node(&state, "feat/auth").unwrap();
        assert_eq!(node["base_ref"], "main");
        assert_eq!(node["parent"]["kind"], "trunk");
    });
}

#[test]
fn init_reuses_marked_lineage_output_for_current_branch() {
    with_temp_repo("dgr-branch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let output = dgr_ok(repo, &["init"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Using existing Git repository."));
        assert!(stdout.contains("Dagger is already initialized."));
        assert!(stdout.contains("✓ feat/auth\n│ \n* main"));
    });
}

#[test]
fn init_lineage_shows_tracked_pull_request_numbers() {
    with_temp_repo("dgr-branch-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);

        let (_, path) = install_fake_gh(
            repo,
            r#"#!/bin/sh
set -eu
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
        );

        dgr_ok_with_env(repo, &["pr"], &[("PATH", path.as_str())]);

        let output = dgr_ok(repo, &["init"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("✓ feat/auth (#123)\n│ \n* main"));
    });
}
