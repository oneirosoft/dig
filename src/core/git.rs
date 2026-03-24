use std::env;
use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};

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

pub fn init_repository() -> io::Result<ExitStatus> {
    Command::new("git").args(["init", "--quiet"]).status()
}

pub fn probe_repo_status() -> io::Result<ExitStatus> {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
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
