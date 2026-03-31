use std::io;

use clap::Args;

use crate::core::adopt::{self, AdoptOptions, AdoptOutcome};

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone)]
pub struct AdoptArgs {
    /// The existing local branch to adopt under the provided parent
    pub branch_name: Option<String>,

    /// The tracked dagger parent branch to adopt under
    #[arg(short = 'p', long = "parent", value_name = "BRANCH")]
    pub parent_branch_name: String,
}

pub fn execute(args: AdoptArgs) -> io::Result<CommandOutcome> {
    let plan = adopt::plan(&args.clone().into())?;
    let outcome = adopt::apply(&plan)?;

    if outcome.status.success() {
        let rendered_tree = super::tree::render_focused_context_tree(&outcome.branch_name, None)?;
        let output = format_adopt_success_output(&outcome, &rendered_tree);
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

impl From<AdoptArgs> for AdoptOptions {
    fn from(args: AdoptArgs) -> Self {
        Self {
            branch_name: args.branch_name,
            parent_branch_name: args.parent_branch_name,
        }
    }
}

pub(crate) fn format_adopt_success_output(outcome: &AdoptOutcome, rendered_tree: &str) -> String {
    let mut sections = Vec::new();
    let mut summary_lines = vec![format!(
        "Adopted '{}' under '{}'.",
        outcome.branch_name, outcome.parent_branch_name
    )];

    if outcome.restacked {
        summary_lines.push(format!(
            "Restacked '{}' onto '{}'.",
            outcome.branch_name, outcome.parent_branch_name
        ));
    }

    if let Some(original_branch) = &outcome.restored_original_branch {
        summary_lines.push(format!("Returned to '{}' after adopt.", original_branch));
    }

    sections.push(summary_lines.join("\n"));

    if !rendered_tree.trim().is_empty() {
        sections.push(rendered_tree.to_string());
    }

    common::join_sections(&sections)
}

#[cfg(test)]
mod tests {
    use super::{AdoptArgs, format_adopt_success_output};
    use crate::core::adopt::{AdoptOptions, AdoptOutcome};
    use crate::core::git;

    #[test]
    fn converts_cli_args_into_core_adopt_options() {
        let args = AdoptArgs {
            branch_name: Some("feat/auth-ui".into()),
            parent_branch_name: "feat/auth".into(),
        };

        let options = AdoptOptions::from(args);

        assert_eq!(options.branch_name.as_deref(), Some("feat/auth-ui"));
        assert_eq!(options.parent_branch_name, "feat/auth");
    }

    #[test]
    fn formats_adopt_success_output_with_tree_context() {
        let output = format_adopt_success_output(
            &AdoptOutcome {
                status: git::success_status().unwrap(),
                branch_name: "feat/auth-ui".into(),
                parent_branch_name: "feat/auth".into(),
                restacked: true,
                restored_original_branch: Some("feat/auth".into()),
                failure_output: None,
                paused: false,
            },
            "main\n└── feat/auth\n    └── feat/auth-ui",
        );

        assert_eq!(
            output,
            concat!(
                "Adopted 'feat/auth-ui' under 'feat/auth'.\n",
                "Restacked 'feat/auth-ui' onto 'feat/auth'.\n",
                "Returned to 'feat/auth' after adopt.\n\n",
                "main\n",
                "└── feat/auth\n",
                "    └── feat/auth-ui"
            )
        );
    }
}
