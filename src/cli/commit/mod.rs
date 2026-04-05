use std::io;

use clap::{ArgAction, Args};

use crate::core::commit::{self, CommitEntry, CommitOptions, CommitOutcome};
use crate::core::restack::RestackPreview;
use crate::ui::markers;
use crate::ui::palette::Accent;

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone)]
pub struct CommitArgs {
    /// Pass through git commit -a
    #[arg(short = 'a')]
    pub all: bool,

    /// Pass through git commit -m/--message
    #[arg(short = 'm', long = "message", value_name = "MESSAGE", action = ArgAction::Append)]
    pub messages: Vec<String>,

    /// Pass through git commit --no-edit
    #[arg(long = "no-edit")]
    pub no_edit: bool,

    /// Pass through git commit --amend
    #[arg(long = "amend")]
    pub amend: bool,
}

pub fn execute(args: CommitArgs) -> io::Result<CommandOutcome> {
    let outcome = commit::run(&args.into())?;

    if outcome.commit_succeeded {
        let output = format_commit_success_output(&outcome);

        if !output.is_empty() {
            println!("{output}");
        }
    }

    if !outcome.status.success() {
        if outcome.paused {
            common::print_restack_pause_guidance(outcome.failure_output.as_deref());
        } else {
            common::print_trimmed_stderr(outcome.failure_output.as_deref());
        }
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<CommitArgs> for CommitOptions {
    fn from(args: CommitArgs) -> Self {
        Self {
            all: args.all,
            messages: args.messages,
            no_edit: args.no_edit,
            amend: args.amend,
        }
    }
}

pub(crate) fn format_commit_success_output(outcome: &CommitOutcome) -> String {
    let mut sections = Vec::new();

    if let Some(summary_line) = &outcome.summary_line {
        sections.push(summary_line.clone());
    }

    if !outcome.recent_commits.is_empty() {
        sections.push(format_recent_commits(&outcome.recent_commits));
    }

    if !outcome.restacked_branches.is_empty() {
        sections.push(format_restacked_branches(&outcome.restacked_branches));
    }

    common::join_sections(&sections)
}

fn format_recent_commits(commits: &[CommitEntry]) -> String {
    commits
        .iter()
        .map(|commit| {
            let prefix = if commit.is_head {
                Accent::HeadMarker.paint_ansi(markers::HEAD)
            } else {
                markers::LIST_ITEM.to_string()
            };
            let refs = format_commit_refs(commit);

            format!(
                "{prefix} {}{refs}: {}",
                Accent::CommitHash.paint_ansi(&commit.hash),
                commit.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_commit_refs(commit: &CommitEntry) -> String {
    if commit.refs.is_empty() {
        String::new()
    } else {
        let refs = commit
            .refs
            .iter()
            .map(|reference| format_reference(reference))
            .collect::<Vec<_>>()
            .join(", ");

        format!(" ({refs})")
    }
}

fn format_reference(reference: &str) -> String {
    if reference.starts_with("tag: ") {
        Accent::TagRef.paint_ansi(reference)
    } else {
        Accent::BranchRef.paint_ansi(reference)
    }
}

fn format_restacked_branches(branches: &[RestackPreview]) -> String {
    common::format_restacked_branches(branches)
}

#[cfg(test)]
mod tests {
    use super::{CommitArgs, format_commit_success_output, format_recent_commits};
    use crate::core::commit::{CommitEntry, CommitOptions, CommitOutcome};
    use crate::core::restack::RestackPreview;
    use clap::FromArgMatches;
    use std::process::ExitStatus;

    /// Create an `ExitStatus` representing a successful (code 0) process.
    fn exit_status_success() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
    }

    /// Create an `ExitStatus` representing a failed (non-zero) process.
    fn exit_status_failure() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(1 << 8) // encodes exit code 1
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(1)
        }
    }

    #[test]
    fn converts_cli_args_into_core_commit_options() {
        let args = CommitArgs {
            all: true,
            messages: vec!["message".into()],
            no_edit: true,
            amend: true,
        };

        assert_eq!(
            CommitOptions::from(args),
            CommitOptions {
                all: true,
                messages: vec!["message".into()],
                no_edit: true,
                amend: true,
            }
        );
    }

    #[test]
    fn parses_short_and_long_message_flags_in_order() {
        let matches = <CommitArgs as clap::Args>::augment_args(clap::Command::new("commit"))
            .try_get_matches_from(["commit", "-m", "first", "--message", "second"])
            .unwrap();
        let args = CommitArgs::from_arg_matches(&matches).unwrap();

        assert_eq!(
            args.messages,
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn formats_recent_commits_for_cli_output() {
        let formatted = format_recent_commits(&[
            CommitEntry {
                hash: "abc1234".into(),
                refs: vec!["main".into(), "tag: v0.1.0".into()],
                is_head: true,
                title: "latest commit".into(),
            },
            CommitEntry {
                hash: "def5678".into(),
                refs: vec!["tag: v0.0.9".into()],
                is_head: false,
                title: "older commit".into(),
            },
        ]);

        assert_eq!(
            formatted,
            "\u{1b}[34m→\u{1b}[0m \u{1b}[33mabc1234\u{1b}[0m (\u{1b}[32mmain\u{1b}[0m, \u{1b}[33mtag: v0.1.0\u{1b}[0m): latest commit\n* \u{1b}[33mdef5678\u{1b}[0m (\u{1b}[33mtag: v0.0.9\u{1b}[0m): older commit"
        );
    }

    #[test]
    fn formats_commit_output_with_summary_and_blank_line_before_log() {
        let outcome = CommitOutcome {
            status: exit_status_success(),
            commit_succeeded: true,
            summary_line: Some("10 files changed, 2245 insertions(+)".into()),
            recent_commits: vec![CommitEntry {
                hash: "abc1234".into(),
                refs: vec!["main".into()],
                is_head: true,
                title: "latest commit".into(),
            }],
            restacked_branches: Vec::new(),
            failure_output: None,
            paused: false,
        };

        assert_eq!(
            format_commit_success_output(&outcome),
            "10 files changed, 2245 insertions(+)\n\n\u{1b}[34m→\u{1b}[0m \u{1b}[33mabc1234\u{1b}[0m (\u{1b}[32mmain\u{1b}[0m): latest commit"
        );
    }

    #[test]
    fn formats_commit_output_with_restack_section() {
        let outcome = CommitOutcome {
            status: exit_status_failure(),
            commit_succeeded: true,
            summary_line: Some("1 file changed, 1 insertion(+)".into()),
            recent_commits: vec![CommitEntry {
                hash: "abc1234".into(),
                refs: vec!["feat/auth".into()],
                is_head: true,
                title: "latest commit".into(),
            }],
            restacked_branches: vec![
                RestackPreview {
                    branch_name: "feat/auth-api".into(),
                    onto_branch: "feat/auth".into(),
                    parent_changed: false,
                },
                RestackPreview {
                    branch_name: "feat/auth-api-tests".into(),
                    onto_branch: "feat/auth-api".into(),
                    parent_changed: false,
                },
            ],
            failure_output: Some("conflict".into()),
            paused: true,
        };

        assert_eq!(
            format_commit_success_output(&outcome),
            concat!(
                "1 file changed, 1 insertion(+)\n\n",
                "\u{1b}[34m→\u{1b}[0m \u{1b}[33mabc1234\u{1b}[0m (\u{1b}[32mfeat/auth\u{1b}[0m): latest commit\n\n",
                "Restacked:\n",
                "- feat/auth-api onto feat/auth\n",
                "- feat/auth-api-tests onto feat/auth-api"
            )
        );
    }
}
