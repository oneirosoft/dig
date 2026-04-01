use std::env;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::MutexGuard;

use crate::core::branch::{self, BranchOptions};

pub(crate) fn with_temp_repo(prefix: &str, test: impl FnOnce(&Path)) {
    let guard: MutexGuard<'_, ()> = crate::core::test_cwd_lock().lock().unwrap();
    let original_dir = env::current_dir().unwrap();
    let repo_dir = env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&repo_dir).unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        env::set_current_dir(&repo_dir).unwrap();
        test(&repo_dir);
    }));

    env::set_current_dir(original_dir).unwrap();
    fs::remove_dir_all(&repo_dir).unwrap();
    drop(guard);

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

pub(crate) fn initialize_main_repo(repo: &Path) {
    git_ok(repo, &["init", "--quiet"]);
    git_ok(repo, &["checkout", "-b", "main"]);
    git_ok(repo, &["config", "user.name", "Dagger Test"]);
    git_ok(repo, &["config", "user.email", "dagger@example.com"]);
    git_ok(repo, &["config", "commit.gpgsign", "false"]);
    commit_file(repo, "README.md", "root\n", "chore: init");
}

pub(crate) fn create_tracked_branch(branch_name: &str) {
    branch::run(&BranchOptions {
        name: branch_name.into(),
        parent_branch_name: None,
    })
    .unwrap();
}

pub(crate) fn commit_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
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

pub(crate) fn append_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
    let path = repo.join(file_name);
    let mut existing = fs::read_to_string(&path).unwrap();
    existing.push_str(contents);
    fs::write(&path, existing).unwrap();
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

pub(crate) fn squash_merge_branch_with_commit_listing(
    repo: &Path,
    target_branch: &str,
    source_branch: &str,
    message: &str,
) {
    git_ok(repo, &["checkout", target_branch]);
    git_ok(repo, &["merge", "--squash", source_branch]);

    let merge_base = git_output(repo, &["merge-base", target_branch, source_branch]);
    let commits = git_output(
        repo,
        &[
            "log",
            "--format=commit %H%n    %s",
            &format!("{merge_base}..{source_branch}"),
        ],
    );

    let commit_message = if commits.trim().is_empty() {
        message.to_string()
    } else {
        format!("{message}\n\n{commits}")
    };

    git_ok(repo, &["commit", "--quiet", "-m", &commit_message]);
}

pub(crate) fn git_ok(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .unwrap();

    assert!(status.success(), "git {:?} failed", args);
}

pub(crate) fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();

    assert!(output.status.success(), "git {:?} failed", args);

    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub(crate) fn synthetic_exit_status(success: bool) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if success {
            ExitStatus::from_raw(0)
        } else {
            ExitStatus::from_raw(1 << 8)
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;

        if success {
            ExitStatus::from_raw(0)
        } else {
            ExitStatus::from_raw(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{git_output, initialize_main_repo, synthetic_exit_status, with_temp_repo};

    #[test]
    fn git_output_trims_trailing_newlines() {
        with_temp_repo("dgr-core-test-support", |repo| {
            initialize_main_repo(repo);

            assert_eq!(git_output(repo, &["branch", "--show-current"]), "main");
        });
    }

    #[test]
    fn synthetic_failure_status_is_non_zero() {
        assert!(synthetic_exit_status(true).success());
        assert!(!synthetic_exit_status(false).success());
    }
}
