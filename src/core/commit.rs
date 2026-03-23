use std::io;
use std::process::{Command, ExitStatus};

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
    pub title: String,
}

#[derive(Debug)]
pub struct CommitOutcome {
    pub status: ExitStatus,
    pub summary_line: Option<String>,
    pub recent_commits: Vec<CommitEntry>,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(options: &CommitOptions) -> io::Result<CommitOutcome> {
    let output = Command::new("git")
        .args(build_git_commit_args(options))
        .output()?;

    let status = output.status;
    let stdout =
        String::from_utf8(output.stdout).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let stderr =
        String::from_utf8(output.stderr).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let summary_line = if status.success() {
        extract_commit_summary_line(&stdout)
    } else {
        None
    };

    let recent_commits = if status.success() {
        load_recent_commits(RECENT_COMMITS_LIMIT).unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(CommitOutcome {
        status,
        summary_line,
        recent_commits,
        stdout,
        stderr,
    })
}

fn build_git_commit_args(options: &CommitOptions) -> Vec<String> {
    let mut git_args = vec!["commit".to_string()];

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

fn load_recent_commits(limit: usize) -> io::Result<Vec<CommitEntry>> {
    let output = Command::new("git")
        .args(["log", "--oneline", "-n", &limit.to_string()])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other("git log --oneline failed"));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

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

            let (hash, title) = trimmed.split_once(' ').unwrap_or((trimmed, ""));

            Some(CommitEntry {
                hash: hash.to_string(),
                title: title.to_string(),
            })
        })
        .collect()
}

fn extract_commit_summary_line(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();

        if trimmed.contains(" file changed")
            || trimmed.contains(" files changed")
            || trimmed.contains(" insertion(+)")
            || trimmed.contains(" insertions(+)")
            || trimmed.contains(" deletion(-)")
            || trimmed.contains(" deletions(-)")
        {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_git_commit_args, extract_commit_summary_line, parse_git_log_output, CommitEntry,
        CommitOptions,
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

        assert_eq!(build_git_commit_args(&options), vec!["commit"]);
    }

    #[test]
    fn parses_git_log_output_into_commit_entries() {
        let log = "abc1234 first commit\n987fedc second commit\n";

        assert_eq!(
            parse_git_log_output(log),
            vec![
                CommitEntry {
                    hash: "abc1234".into(),
                    title: "first commit".into(),
                },
                CommitEntry {
                    hash: "987fedc".into(),
                    title: "second commit".into(),
                },
            ]
        );
    }

    #[test]
    fn extracts_commit_summary_line_from_git_commit_output() {
        let stdout = "[main abc1234] feat: sample\n 10 files changed, 2245 insertions(+)\n create mode 100644 src/main.rs\n";

        assert_eq!(
            extract_commit_summary_line(stdout),
            Some("10 files changed, 2245 insertions(+)".into())
        );
    }
}
