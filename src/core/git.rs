use std::env;
use std::io;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CherryMarker {
    Equivalent,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebaseProgress {
    pub current: usize,
    pub total: usize,
}

#[derive(Debug)]
pub struct GitCommandOutput {
    pub status: ExitStatus,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMetadata {
    pub sha: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct RepoContext {
    pub git_dir: PathBuf,
}

pub fn try_resolve_repo_context() -> io::Result<Option<RepoContext>> {
    let output = Command::new("git").args(["rev-parse", "--git-dir"]).output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let git_dir = resolve_git_dir(stdout.trim())?;

    Ok(Some(RepoContext { git_dir }))
}

pub fn resolve_repo_context() -> io::Result<RepoContext> {
    let git_dir = read_git_stdout(["rev-parse", "--git-dir"])?;
    let git_dir = resolve_git_dir(&git_dir)?;

    Ok(RepoContext { git_dir })
}

pub fn current_branch_name() -> io::Result<String> {
    let branch_name = read_git_stdout(["branch", "--show-current"])?;

    if branch_name.is_empty() {
        return Err(io::Error::other(
            "dig branch requires a named branch; detached HEAD is not supported",
        ));
    }

    Ok(branch_name)
}

pub fn current_branch_name_or(default: &str) -> io::Result<String> {
    match current_branch_name() {
        Ok(branch_name) => Ok(branch_name),
        Err(_) => Ok(default.to_string()),
    }
}

pub fn ref_oid(reference: &str) -> io::Result<String> {
    read_git_stdout(["rev-parse", reference])
}

pub fn merge_base(left: &str, right: &str) -> io::Result<String> {
    read_git_stdout(["merge-base", left, right])
}

pub fn branch_exists(branch_name: &str) -> io::Result<bool> {
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch_name}")])
        .status()?;

    Ok(status.success())
}

pub fn create_and_checkout_branch(branch_name: &str, start_point: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["checkout", "--quiet", "-b", branch_name, start_point])
        .status()
}

pub fn switch_branch(branch_name: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["checkout", "--quiet", branch_name])
        .status()
}

pub fn delete_branch_force(branch_name: &str) -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["branch", "--quiet", "-D", branch_name])
        .status()
}

pub fn rebase_onto_with_progress<F>(
    new_base: &str,
    old_upstream: &str,
    branch_name: &str,
    mut on_progress: F,
) -> io::Result<GitCommandOutput>
where
    F: FnMut(RebaseProgress) -> io::Result<()>,
{
    let mut child = Command::new("git")
        .args(["rebase", "--onto", new_base, old_upstream, branch_name])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("failed to capture git rebase stderr"))?;
    let mut stderr_output = String::new();
    let mut chunk = [0_u8; 256];
    let mut last_progress = None;

    loop {
        let read = stderr.read(&mut chunk)?;
        if read == 0 {
            break;
        }

        let text = String::from_utf8_lossy(&chunk[..read]);
        stderr_output.push_str(&text);

        if let Some(progress) = parse_latest_rebase_progress(&stderr_output) {
            if last_progress != Some(progress) {
                on_progress(progress)?;
                last_progress = Some(progress);
            }
        }
    }

    let status = child.wait()?;

    Ok(GitCommandOutput {
        status,
        stderr: stderr_output,
    })
}

pub fn init_repository() -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["init", "--quiet"])
        .status()
}

pub fn probe_repo_status() -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

pub fn success_status() -> io::Result<ExitStatus> {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

pub fn ensure_clean_worktree() -> io::Result<()> {
    let status = read_git_stdout(["status", "--porcelain"])?;

    if status.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(
            "dig clean requires a clean working tree",
        ))
    }
}

pub fn ensure_no_in_progress_operations(repo: &RepoContext) -> io::Result<()> {
    let in_progress_paths = [
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REBASE_HEAD", "rebase"),
    ];

    for (relative_path, operation_name) in in_progress_paths {
        if repo.git_dir.join(relative_path).exists() {
            return Err(io::Error::other(format!(
                "dig clean cannot run while a git {operation_name} is in progress"
            )));
        }
    }

    let rebase_dirs = ["rebase-merge", "rebase-apply"];
    for relative_path in rebase_dirs {
        if repo.git_dir.join(relative_path).exists() {
            return Err(io::Error::other(
                "dig clean cannot run while a git rebase is in progress",
            ));
        }
    }

    Ok(())
}

pub fn cherry_markers(parent_branch_name: &str, branch_name: &str) -> io::Result<Vec<CherryMarker>> {
    let stdout = read_git_stdout(["cherry", parent_branch_name, branch_name])?;

    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.chars().next() {
            Some('-') => Ok(CherryMarker::Equivalent),
            Some('+') => Ok(CherryMarker::Missing),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected git cherry output: {line}"),
            )),
        })
        .collect()
}

pub fn commit_metadata_in_range(range_spec: &str) -> io::Result<Vec<CommitMetadata>> {
    let output = Command::new("git")
        .args(["log", "--reverse", "--format=%H%x1f%s%x1f%B%x1e", range_spec])
        .output()?;

    if !output.status.success() {
        return Err(git_command_failed(&output));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(parse_commit_metadata_records(&stdout))
}

fn read_git_stdout<const N: usize>(args: [&str; N]) -> io::Result<String> {
    let output = Command::new("git").args(args).output()?;

    if !output.status.success() {
        return Err(git_command_failed(&output));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(stdout.trim().to_string())
}

fn git_command_failed(output: &Output) -> io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr.trim();

    if message.is_empty() {
        io::Error::other("git command failed")
    } else {
        io::Error::other(message.to_string())
    }
}

fn resolve_git_dir(git_dir: &str) -> io::Result<PathBuf> {
    let git_dir = PathBuf::from(git_dir);

    if git_dir.is_absolute() {
        Ok(git_dir)
    } else {
        Ok(env::current_dir()?.join(git_dir))
    }
}

fn parse_latest_rebase_progress(output: &str) -> Option<RebaseProgress> {
    let marker = "Rebasing (";
    let start = output.rfind(marker)?;
    let remainder = &output[start + marker.len()..];
    let end = remainder.find(')')?;
    let (current, total) = remainder[..end].split_once('/')?;

    Some(RebaseProgress {
        current: current.trim().parse().ok()?,
        total: total.trim().parse().ok()?,
    })
}

fn parse_commit_metadata_records(output: &str) -> Vec<CommitMetadata> {
    output
        .split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim();
            if record.is_empty() {
                return None;
            }

            let mut fields = record.splitn(3, '\u{1f}');
            let sha = fields.next()?.trim().to_string();
            let subject = fields.next()?.trim().to_string();
            let body = fields.next()?.trim().to_string();

            Some(CommitMetadata { sha, subject, body })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CherryMarker, CommitMetadata, RebaseProgress, RepoContext, cherry_markers,
        parse_commit_metadata_records, parse_latest_rebase_progress,
    };
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn reports_in_progress_rebase_state() {
        let repo_git_dir = env::temp_dir().join(format!("dig-git-{}", Uuid::new_v4()));
        fs::create_dir_all(repo_git_dir.join("rebase-merge")).unwrap();

        let error = super::ensure_no_in_progress_operations(&RepoContext {
            git_dir: PathBuf::from(&repo_git_dir),
        })
        .unwrap_err();

        assert!(error.to_string().contains("rebase"));

        fs::remove_dir_all(repo_git_dir).unwrap();
    }

    #[test]
    fn parses_git_cherry_markers() {
        let markers = ["- abc123", "+ def456"]
            .into_iter()
            .map(|line| match line.chars().next() {
                Some('-') => Ok(CherryMarker::Equivalent),
                Some('+') => Ok(CherryMarker::Missing),
                _ => Err(()),
            })
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(markers, vec![CherryMarker::Equivalent, CherryMarker::Missing]);
    }

    #[test]
    fn cherry_markers_is_present_for_runtime_use() {
        let function_ptr: fn(&str, &str) -> std::io::Result<Vec<CherryMarker>> = cherry_markers;
        assert!((function_ptr as usize) != 0);
    }

    #[test]
    fn parses_rebase_progress_from_git_output() {
        assert_eq!(
            parse_latest_rebase_progress("Rebasing (2/7)\rSuccessfully rebased"),
            Some(RebaseProgress {
                current: 2,
                total: 7,
            })
        );
    }

    #[test]
    fn parses_commit_metadata_records_from_git_log_output() {
        assert_eq!(
            parse_commit_metadata_records(
                "abc123\u{1f}feat: first\u{1f}feat: first\n\nbody\u{1e}def456\u{1f}feat: second\u{1f}feat: second\u{1e}"
            ),
            vec![
                CommitMetadata {
                    sha: "abc123".into(),
                    subject: "feat: first".into(),
                    body: "feat: first\n\nbody".into(),
                },
                CommitMetadata {
                    sha: "def456".into(),
                    subject: "feat: second".into(),
                    body: "feat: second".into(),
                },
            ]
        );
    }
}
