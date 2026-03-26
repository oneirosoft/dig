use std::io;

use clap::{Args, Subcommand};

use crate::core::git;
use crate::core::pr::{
    self, PrMergeOutcome, PrOptions, PrOutcomeKind, RetargetedPullRequest,
    TrackedPullRequestListNode, TrackedPullRequestListView,
};

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone)]
pub struct PrArgs {
    #[command(subcommand)]
    pub command: Option<PrCommand>,

    /// Title for the pull request
    #[arg(long = "title", value_name = "TITLE")]
    pub title: Option<String>,

    /// Body for the pull request
    #[arg(long = "body", value_name = "BODY")]
    pub body: Option<String>,

    /// Mark the pull request as a draft
    #[arg(long = "draft")]
    pub draft: bool,

    /// Open the pull request in the browser
    #[arg(long = "view")]
    pub view: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PrCommand {
    /// List open pull requests that are tracked by dig
    List(PrListArgs),

    /// Merge the current tracked pull request on GitHub
    Merge(PrMergeArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct PrListArgs {
    /// Open each listed pull request in the browser
    #[arg(long = "view")]
    pub view: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct PrMergeArgs {}

pub fn execute(args: PrArgs) -> io::Result<CommandOutcome> {
    match args.command.clone() {
        Some(PrCommand::List(list_args)) => execute_list(list_args),
        Some(PrCommand::Merge(_)) => execute_merge(),
        None => execute_current(args),
    }
}

fn execute_current(args: PrArgs) -> io::Result<CommandOutcome> {
    let create_requested = args.title.is_some() || args.body.is_some() || args.draft;
    if args.view && !create_requested {
        pr::open_current_pull_request_in_browser()?;
        return Ok(CommandOutcome {
            status: git::success_status()?,
        });
    }

    let mut options: PrOptions = args.clone().into();
    if let Some(push_target) = pr::current_branch_push_target_for_create()? {
        let confirmed = common::confirm_yes_no(&format!(
            "Branch '{}' is not pushed to '{}'. Push it and create the pull request? [y/N] ",
            push_target.branch_name, push_target.remote_name
        ))?;

        if !confirmed {
            println!(
                "Did not create pull request because '{}' is not pushed to '{}'.",
                push_target.branch_name, push_target.remote_name
            );
            return Ok(CommandOutcome {
                status: git::success_status()?,
            });
        }

        options.push_if_needed = true;
    }

    let outcome = pr::run(&options)?;
    match outcome.kind {
        PrOutcomeKind::AlreadyTracked => {
            println!(
                "Branch '{}' already tracks pull request #{}.",
                outcome.branch_name, outcome.pull_request.number
            );
        }
        PrOutcomeKind::Created => {
            println!(
                "Created pull request #{} for '{}' into '{}'.",
                outcome.pull_request.number, outcome.branch_name, outcome.base_branch_name
            );
        }
        PrOutcomeKind::Adopted => {
            println!(
                "Tracking existing pull request #{} for '{}' into '{}'.",
                outcome.pull_request.number, outcome.branch_name, outcome.base_branch_name
            );
        }
    }

    if args.view {
        pr::open_pull_request_in_browser(outcome.pull_request.number)?;
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

fn execute_list(args: PrListArgs) -> io::Result<CommandOutcome> {
    let outcome = pr::list_open_tracked_pull_requests()?;

    if outcome.pull_requests.is_empty() {
        println!("No open tracked pull requests.");
    } else {
        println!("{}", render_pull_request_list(&outcome.view));
    }

    if args.view {
        pr::open_pull_requests_in_browser(&outcome.pull_requests)?;
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

fn execute_merge() -> io::Result<CommandOutcome> {
    let outcome = pr::merge_current_pull_request()?;
    let output = format_pr_merge_output(&outcome);
    if !output.is_empty() {
        println!("{output}");
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

fn render_pull_request_list(view: &TrackedPullRequestListView) -> String {
    common::render_tree(
        view.root_label.clone(),
        &view.roots,
        &format_pull_request_label,
        &|node| node.children.as_slice(),
    )
}

fn format_pull_request_label(node: &TrackedPullRequestListNode) -> String {
    format!(
        "#{}: {} - {}",
        node.pull_request.number, node.pull_request.title, node.pull_request.url
    )
}

fn format_pr_merge_output(outcome: &PrMergeOutcome) -> String {
    let mut sections = Vec::new();

    let retargeted = format_retargeted_pull_requests(&outcome.retargeted_pull_requests);
    if !retargeted.is_empty() {
        sections.push(retargeted);
    }

    sections.push(format!(
        "Merged pull request #{} for '{}' into '{}'.",
        outcome.pull_request_number, outcome.branch_name, outcome.base_branch_name
    ));

    common::join_sections(&sections)
}

fn format_retargeted_pull_requests(retargeted: &[RetargetedPullRequest]) -> String {
    if retargeted.is_empty() {
        return String::new();
    }

    let mut lines = vec!["Retargeted child pull requests:".to_string()];
    for pull_request in retargeted {
        lines.push(format!(
            "- #{} for {} to {}",
            pull_request.pull_request_number,
            pull_request.branch_name,
            pull_request.new_base_branch_name
        ));
    }

    lines.join("\n")
}

impl From<PrArgs> for PrOptions {
    fn from(args: PrArgs) -> Self {
        Self {
            title: args.title,
            body: args.body,
            draft: args.draft,
            push_if_needed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PrArgs, PrCommand, PrListArgs, PrMergeArgs, RetargetedPullRequest, format_pr_merge_output,
        render_pull_request_list,
    };
    use crate::core::git;
    use crate::core::pr::PrOptions;
    use crate::core::pr::{PrMergeOutcome, TrackedPullRequestListNode, TrackedPullRequestListView};

    #[test]
    fn converts_cli_args_into_core_pr_options() {
        let options = PrOptions::from(PrArgs {
            command: None,
            title: Some("feat: auth".into()),
            body: Some("Implements auth.".into()),
            draft: true,
            view: true,
        });

        assert_eq!(options.title.as_deref(), Some("feat: auth"));
        assert_eq!(options.body.as_deref(), Some("Implements auth."));
        assert!(options.draft);
    }

    #[test]
    fn preserves_pr_list_subcommand_args() {
        match (PrArgs {
            command: Some(PrCommand::List(PrListArgs { view: true })),
            title: None,
            body: None,
            draft: false,
            view: false,
        })
        .command
        .unwrap()
        {
            PrCommand::List(args) => assert!(args.view),
            PrCommand::Merge(_) => unreachable!(),
        }
    }

    #[test]
    fn preserves_pr_merge_subcommand_args() {
        match (PrArgs {
            command: Some(PrCommand::Merge(PrMergeArgs::default())),
            title: None,
            body: None,
            draft: false,
            view: false,
        })
        .command
        .unwrap()
        {
            PrCommand::Merge(_) => {}
            _ => unreachable!(),
        }
    }

    #[test]
    fn renders_pull_request_list_as_tree() {
        let rendered = render_pull_request_list(&TrackedPullRequestListView {
            root_label: Some("main".into()),
            roots: vec![TrackedPullRequestListNode {
                pull_request: crate::core::gh::PullRequestDetails {
                    number: 123,
                    title: "Auth".into(),
                    url: "https://github.com/acme/dig/pull/123".into(),
                },
                children: vec![TrackedPullRequestListNode {
                    pull_request: crate::core::gh::PullRequestDetails {
                        number: 124,
                        title: "Auth UI".into(),
                        url: "https://github.com/acme/dig/pull/124".into(),
                    },
                    children: vec![],
                }],
            }],
        });

        assert_eq!(
            rendered,
            concat!(
                "main\n",
                "└── #123: Auth - https://github.com/acme/dig/pull/123\n",
                "    └── #124: Auth UI - https://github.com/acme/dig/pull/124"
            )
        );
    }

    #[test]
    fn formats_pr_merge_output_with_retargeted_children() {
        let output = format_pr_merge_output(&PrMergeOutcome {
            status: git::success_status().unwrap(),
            branch_name: "feat/auth".into(),
            base_branch_name: "main".into(),
            pull_request_number: 123,
            retargeted_pull_requests: vec![RetargetedPullRequest {
                branch_name: "feat/auth-ui".into(),
                pull_request_number: 124,
                new_base_branch_name: "main".into(),
            }],
        });

        assert_eq!(
            output,
            concat!(
                "Retargeted child pull requests:\n",
                "- #124 for feat/auth-ui to main\n\n",
                "Merged pull request #123 for 'feat/auth' into 'main'."
            )
        );
    }
}
