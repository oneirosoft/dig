use std::io;

use clap::Args;

use crate::core::reparent::{self, ReparentOptions, ReparentOutcome, ReparentPlan};

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone, Default)]
pub struct ReparentArgs {
    /// The tracked branch to reparent in dagger
    pub branch_name: Option<String>,

    /// The new tracked dagger parent branch
    #[arg(short = 'p', long = "parent", value_name = "BRANCH")]
    pub parent_branch_name: String,
}

pub fn execute(args: ReparentArgs) -> io::Result<CommandOutcome> {
    let plan: ReparentPlan = reparent::plan(&args.clone().into())?;
    let outcome = reparent::apply(&plan)?;

    if outcome.status.success() {
        let rendered_tree = super::tree::render_focused_context_tree(&outcome.branch_name, None)?;
        let output = format_reparent_success_output(&outcome, &rendered_tree);
        if !output.is_empty() {
            println!("{output}");
        }
    } else if outcome.paused {
        common::print_restack_pause_guidance(outcome.failure_output.as_deref());
    } else {
        common::print_trimmed_stderr(outcome.failure_output.as_deref());
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<ReparentArgs> for ReparentOptions {
    fn from(args: ReparentArgs) -> Self {
        Self {
            branch_name: args.branch_name,
            parent_branch_name: args.parent_branch_name,
        }
    }
}

pub(crate) fn format_reparent_success_output(
    outcome: &ReparentOutcome,
    rendered_tree: &str,
) -> String {
    let mut sections = Vec::new();
    let mut summary_lines = vec![format!(
        "Reparented '{}' onto '{}'.",
        outcome.branch_name, outcome.parent_branch_name
    )];

    if let Some(original_branch) = &outcome.restored_original_branch {
        summary_lines.push(format!(
            "Returned to '{}' after reparenting.",
            original_branch
        ));
    }

    sections.push(summary_lines.join("\n"));

    if !outcome.restacked_branches.is_empty() {
        sections.push(common::format_restacked_branches(
            &outcome.restacked_branches,
        ));
    }

    if !rendered_tree.trim().is_empty() {
        sections.push(rendered_tree.to_string());
    }

    common::join_sections(&sections)
}

#[cfg(test)]
mod tests {
    use super::{ReparentArgs, format_reparent_success_output};
    use crate::core::git;
    use crate::core::reparent::{ReparentOptions, ReparentOutcome};
    use crate::core::restack::RestackPreview;

    #[test]
    fn converts_cli_args_into_core_reparent_options() {
        let options = ReparentOptions::from(ReparentArgs {
            branch_name: Some("feat/auth-ui".into()),
            parent_branch_name: "feat/platform".into(),
        });

        assert_eq!(options.branch_name.as_deref(), Some("feat/auth-ui"));
        assert_eq!(options.parent_branch_name, "feat/platform");
    }

    #[test]
    fn formats_reparent_success_output_with_tree_context() {
        let output = format_reparent_success_output(
            &ReparentOutcome {
                status: git::success_status().unwrap(),
                branch_name: "feat/auth-ui".into(),
                parent_branch_name: "feat/platform".into(),
                restacked_branches: vec![
                    RestackPreview {
                        branch_name: "feat/auth-ui".into(),
                        onto_branch: "feat/platform".into(),
                        parent_changed: true,
                    },
                    RestackPreview {
                        branch_name: "feat/auth-ui-tests".into(),
                        onto_branch: "feat/auth-ui".into(),
                        parent_changed: false,
                    },
                ],
                restored_original_branch: Some("main".into()),
                failure_output: None,
                paused: false,
            },
            "main\n└── feat/platform\n    └── feat/auth-ui",
        );

        assert_eq!(
            output,
            concat!(
                "Reparented 'feat/auth-ui' onto 'feat/platform'.\n",
                "Returned to 'main' after reparenting.\n\n",
                "Restacked:\n",
                "- feat/auth-ui onto feat/platform\n",
                "- feat/auth-ui-tests onto feat/auth-ui\n\n",
                "main\n",
                "└── feat/platform\n",
                "    └── feat/auth-ui"
            )
        );
    }
}
