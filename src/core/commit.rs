use std::io;
use std::process::{Command, ExitStatus};

use crate::core::git::{self, RepoContext};
use crate::core::restack::{self, RestackPreview};
use crate::core::store::{
    PendingCommitEntry, PendingCommitOperation, PendingOperationKind, PendingOperationState,
    StoreSession, dig_paths, load_config, load_state, open_initialized,
};
use crate::core::workflow;

pub const RECENT_COMMITS_LIMIT: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub all: bool,
    pub messages: Vec<String>,
    pub no_edit: bool,
    pub amend: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEntry {
    pub hash: String,
    pub refs: Vec<String>,
    pub is_head: bool,
    pub title: String,
}

#[derive(Debug)]
pub struct CommitOutcome {
    pub status: ExitStatus,
    pub commit_succeeded: bool,
    pub summary_line: Option<String>,
    pub recent_commits: Vec<CommitEntry>,
    pub restacked_branches: Vec<RestackPreview>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

pub fn run(options: &CommitOptions) -> io::Result<CommitOutcome> {
    ensure_no_pending_commit_operation()?;
    let pre_commit_context = resolve_pre_commit_context()?;
    let status = Command::new("git")
        .args(build_git_commit_args(options))
        .status()?;
    let commit_succeeded = status.success();

    let summary_line = if commit_succeeded {
        load_commit_summary_line().unwrap_or_default()
    } else {
        None
    };

    let recent_commits = if commit_succeeded {
        load_recent_commits(RECENT_COMMITS_LIMIT).unwrap_or_default()
    } else {
        Vec::new()
    };
    let post_commit = if commit_succeeded {
        maybe_restack_after_commit(
            pre_commit_context.as_ref(),
            summary_line.clone(),
            recent_commits.clone(),
        )
    } else {
        PostCommitRestackOutcome::default()
    };

    Ok(CommitOutcome {
        status: post_commit.status_override.unwrap_or(status),
        commit_succeeded,
        summary_line,
        recent_commits,
        restacked_branches: post_commit.restacked_branches,
        failure_output: post_commit.failure_output,
        paused: post_commit.paused,
    })
}

impl From<CommitEntry> for PendingCommitEntry {
    fn from(entry: CommitEntry) -> Self {
        Self {
            hash: entry.hash,
            refs: entry.refs,
            is_head: entry.is_head,
            title: entry.title,
        }
    }
}

impl From<PendingCommitEntry> for CommitEntry {
    fn from(entry: PendingCommitEntry) -> Self {
        Self {
            hash: entry.hash,
            refs: entry.refs,
            is_head: entry.is_head,
            title: entry.title,
        }
    }
}

fn build_git_commit_args(options: &CommitOptions) -> Vec<String> {
    let mut git_args = vec!["commit".to_string(), "--quiet".to_string()];

    if options.all {
        git_args.push("-a".to_string());
    }

    for message in &options.messages {
        git_args.push("-m".to_string());
        git_args.push(message.clone());
    }

    if options.no_edit {
        git_args.push("--no-edit".to_string());
    }

    if options.amend {
        git_args.push("--amend".to_string());
    }

    git_args
}

fn load_commit_summary_line() -> io::Result<Option<String>> {
    let output = Command::new("git")
        .args([
            "diff-tree",
            "--shortstat",
            "--no-commit-id",
            "--root",
            "HEAD",
        ])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other("git diff-tree --shortstat failed"));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }))
}

fn load_recent_commits(limit: usize) -> io::Result<Vec<CommitEntry>> {
    let output = Command::new("git")
        .args([
            "log",
            "--decorate=short",
            "--oneline",
            "-n",
            &limit.to_string(),
        ])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(
            "git log --decorate=short --oneline failed",
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(parse_git_log_output(&stdout))
}

fn parse_git_log_output(stdout: &str) -> Vec<CommitEntry> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            let (hash, remainder) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
            let (is_head, refs, title) = parse_commit_metadata(remainder);

            Some(CommitEntry {
                hash: hash.to_string(),
                refs,
                is_head,
                title,
            })
        })
        .collect()
}

fn parse_commit_metadata(remainder: &str) -> (bool, Vec<String>, String) {
    let trimmed = remainder.trim_start();

    if let Some(decorated) = trimmed.strip_prefix('(') {
        if let Some((decorations, title)) = decorated.split_once(") ") {
            let (is_head, refs) = parse_decorations(decorations);
            return (is_head, refs, title.to_string());
        }
    }

    (false, Vec::new(), trimmed.to_string())
}

fn parse_decorations(decorations: &str) -> (bool, Vec<String>) {
    let mut is_head = false;
    let mut refs = Vec::new();

    for decoration in decorations.split(',') {
        let trimmed = decoration.trim();

        if let Some(branch) = trimmed.strip_prefix("HEAD -> ") {
            is_head = true;
            refs.push(branch.to_string());
        } else if trimmed == "HEAD" {
            is_head = true;
        } else if !trimmed.is_empty() {
            refs.push(trimmed.to_string());
        }
    }

    (is_head, refs)
}

#[derive(Debug)]
struct PreCommitContext {
    repo: RepoContext,
    current_branch: Option<String>,
    old_head_oid: Option<String>,
}

#[derive(Debug, Default)]
struct PostCommitRestackOutcome {
    status_override: Option<ExitStatus>,
    restacked_branches: Vec<RestackPreview>,
    failure_output: Option<String>,
    paused: bool,
}

impl PostCommitRestackOutcome {
    fn failure(restacked_branches: Vec<RestackPreview>, failure_output: String) -> Self {
        Self {
            status_override: Some(synthetic_failure_status()),
            restacked_branches,
            failure_output: Some(failure_output),
            paused: false,
        }
    }
}

fn ensure_no_pending_commit_operation() -> io::Result<()> {
    let Some(repo) = git::try_resolve_repo_context()? else {
        return Ok(());
    };

    let paths = dig_paths(&repo.git_dir);
    if load_config(&paths)?.is_none() {
        return Ok(());
    }

    workflow::ensure_no_pending_operation(&paths, "commit")
}

fn resolve_pre_commit_context() -> io::Result<Option<PreCommitContext>> {
    let Some(repo) = git::try_resolve_repo_context()? else {
        return Ok(None);
    };

    Ok(Some(PreCommitContext {
        current_branch: git::current_branch_name_if_any()?,
        old_head_oid: git::ref_oid("HEAD").ok(),
        repo,
    }))
}

fn maybe_restack_after_commit(
    context: Option<&PreCommitContext>,
    summary_line: Option<String>,
    recent_commits: Vec<CommitEntry>,
) -> PostCommitRestackOutcome {
    match maybe_restack_after_commit_inner(context, summary_line, recent_commits) {
        Ok(outcome) => outcome,
        Err(err) => PostCommitRestackOutcome::failure(Vec::new(), err.to_string()),
    }
}

fn maybe_restack_after_commit_inner(
    context: Option<&PreCommitContext>,
    summary_line: Option<String>,
    recent_commits: Vec<CommitEntry>,
) -> io::Result<PostCommitRestackOutcome> {
    let Some(context) = context else {
        return Ok(PostCommitRestackOutcome::default());
    };
    let Some(current_branch) = context.current_branch.as_deref() else {
        return Ok(PostCommitRestackOutcome::default());
    };
    let Some(old_head_oid) = context.old_head_oid.as_deref() else {
        return Ok(PostCommitRestackOutcome::default());
    };

    let store_paths = dig_paths(&context.repo.git_dir);
    let Some(config) = load_config(&store_paths)? else {
        return Ok(PostCommitRestackOutcome::default());
    };

    if current_branch == config.trunk_branch {
        return Ok(PostCommitRestackOutcome::default());
    }

    let state = load_state(&store_paths)?;
    let Some(node) = state.find_branch_by_name(current_branch).cloned() else {
        return Ok(PostCommitRestackOutcome::default());
    };

    let actions =
        restack::plan_after_branch_advance(&state, node.id, &node.branch_name, old_head_oid)?;
    let mut session = StoreSession {
        repo: context.repo.clone(),
        paths: store_paths,
        config,
        state,
    };
    let restack_outcome = match workflow::execute_resumable_restack_operation(
        &mut session,
        PendingOperationKind::Commit(PendingCommitOperation {
            current_branch: current_branch.to_string(),
            summary_line,
            recent_commits: recent_commits.into_iter().map(Into::into).collect(),
        }),
        &actions,
        &mut |_| Ok(()),
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            return Ok(PostCommitRestackOutcome::failure(
                Vec::new(),
                err.to_string(),
            ));
        }
    };

    if !restack_outcome.status.success() {
        return Ok(PostCommitRestackOutcome {
            status_override: Some(restack_outcome.status),
            restacked_branches: restack_outcome.restacked_branches,
            failure_output: restack_outcome.failure_output,
            paused: restack_outcome.paused,
        });
    }

    let restacked_branches = restack_outcome.restacked_branches;

    if !restacked_branches.is_empty() {
        let checkout = workflow::checkout_branch_if_needed(current_branch)?;
        if checkout.switched_from.is_some() {
            let status = checkout.status;
            return Ok(PostCommitRestackOutcome {
                status_override: Some(status),
                restacked_branches,
                failure_output: (!status.success()).then(|| {
                    format!(
                        "commit succeeded, but failed to return to '{}' after restack",
                        current_branch
                    )
                }),
                paused: false,
            });
        }
    }

    Ok(PostCommitRestackOutcome {
        status_override: None,
        restacked_branches,
        failure_output: None,
        paused: false,
    })
}

pub(crate) fn resume_after_sync(
    pending_operation: PendingOperationState,
    payload: PendingCommitOperation,
) -> io::Result<CommitOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let restack_outcome = workflow::continue_resumable_restack_operation(
        &mut session,
        pending_operation,
        &mut |_| Ok(()),
    )?;

    let mut failure_output = restack_outcome.failure_output;
    let status = if restack_outcome.paused {
        restack_outcome.status
    } else if !restack_outcome.restacked_branches.is_empty() {
        let checkout = workflow::checkout_branch_if_needed(&payload.current_branch)?;
        if checkout.switched_from.is_some() && !checkout.status.success() {
            failure_output = Some(format!(
                "commit succeeded, but failed to return to '{}' after restack",
                payload.current_branch
            ));
        }
        checkout.status
    } else {
        restack_outcome.status
    };

    Ok(CommitOutcome {
        status,
        commit_succeeded: true,
        summary_line: payload.summary_line,
        recent_commits: payload.recent_commits.into_iter().map(Into::into).collect(),
        restacked_branches: restack_outcome.restacked_branches,
        failure_output,
        paused: restack_outcome.paused,
    })
}

#[cfg(unix)]
fn synthetic_failure_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;

    ExitStatus::from_raw(1 << 8)
}

#[cfg(windows)]
fn synthetic_failure_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;

    ExitStatus::from_raw(1)
}

#[cfg(test)]
mod tests {
    use super::{
        CommitEntry, CommitOptions, build_git_commit_args, parse_decorations, parse_git_log_output,
    };

    #[test]
    fn builds_commit_command_with_supported_passthrough_flags() {
        let options = CommitOptions {
            all: true,
            messages: vec!["first".into(), "second".into()],
            no_edit: true,
            amend: true,
        };

        assert_eq!(
            build_git_commit_args(&options),
            vec![
                "commit",
                "--quiet",
                "-a",
                "-m",
                "first",
                "-m",
                "second",
                "--no-edit",
                "--amend",
            ]
        );
    }

    #[test]
    fn builds_minimal_commit_command_when_no_flags_are_set() {
        let options = CommitOptions::default();

        assert_eq!(build_git_commit_args(&options), vec!["commit", "--quiet"]);
    }

    #[test]
    fn parses_git_log_output_into_commit_entries() {
        let log = "abc1234 (HEAD -> main, tag: v0.1.0) first commit\n987fedc second commit\n";

        assert_eq!(
            parse_git_log_output(log),
            vec![
                CommitEntry {
                    hash: "abc1234".into(),
                    refs: vec!["main".into(), "tag: v0.1.0".into()],
                    is_head: true,
                    title: "first commit".into(),
                },
                CommitEntry {
                    hash: "987fedc".into(),
                    refs: Vec::new(),
                    is_head: false,
                    title: "second commit".into(),
                },
            ]
        );
    }

    #[test]
    fn parses_head_and_tag_decorations() {
        assert_eq!(
            parse_decorations("HEAD -> main, tag: v0.1.0, origin/main"),
            (
                true,
                vec!["main".into(), "tag: v0.1.0".into(), "origin/main".into()]
            )
        );
    }

    #[test]
    fn synthetic_failure_status_is_non_zero() {
        assert!(!super::synthetic_failure_status().success());
    }
}
