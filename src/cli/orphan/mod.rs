use std::io;

use clap::Args;

use crate::core::orphan::{self, OrphanOptions, OrphanOutcome};
use crate::core::tree;

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone, Default)]
pub struct OrphanArgs {
    /// The tracked branch to stop tracking in dig
    pub branch_name: Option<String>,
}

pub fn execute(args: OrphanArgs) -> io::Result<CommandOutcome> {
    let plan = orphan::plan(&args.clone().into())?;
    let outcome = orphan::apply(&plan)?;

    if outcome.status.success() {
        let view = tree::focused_context_view(&outcome.parent_branch_name)?;
        let rendered_tree = super::tree::render_stack_tree(&view);
        let output = format_orphan_success_output(&outcome, &rendered_tree);
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

impl From<OrphanArgs> for OrphanOptions {
    fn from(args: OrphanArgs) -> Self {
        Self {
            branch_name: args.branch_name,
        }
    }
}

pub(crate) fn format_orphan_success_output(outcome: &OrphanOutcome, rendered_tree: &str) -> String {
    let mut sections = Vec::new();
    let mut summary_lines = vec![format!(
        "Orphaned '{}'. It is no longer tracked by dig.",
        outcome.branch_name
    )];

    if let Some(original_branch) = &outcome.restored_original_branch {
        summary_lines.push(format!(
            "Returned to '{}' after orphaning.",
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
    use super::{OrphanArgs, format_orphan_success_output};
    use crate::core::git;
    use crate::core::orphan::{OrphanOptions, OrphanOutcome};
    use crate::core::restack::RestackPreview;

    #[test]
    fn converts_cli_args_into_core_orphan_options() {
        let options = OrphanOptions::from(OrphanArgs {
            branch_name: Some("feat/auth".into()),
        });

        assert_eq!(options.branch_name.as_deref(), Some("feat/auth"));
    }

    #[test]
    fn formats_orphan_success_output_with_tree_context() {
        let output = format_orphan_success_output(
            &OrphanOutcome {
                status: git::success_status().unwrap(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "main".into(),
                restacked_branches: vec![RestackPreview {
                    branch_name: "feat/auth-ui".into(),
                    onto_branch: "main".into(),
                    parent_changed: true,
                }],
                restored_original_branch: Some("feat/auth".into()),
                failure_output: None,
                paused: false,
            },
            "main\n└── feat/auth-ui",
        );

        assert_eq!(
            output,
            concat!(
                "Orphaned 'feat/auth'. It is no longer tracked by dig.\n",
                "Returned to 'feat/auth' after orphaning.\n\n",
                "Restacked:\n",
                "- feat/auth-ui onto main\n\n",
                "main\n",
                "└── feat/auth-ui"
            )
        );
    }
}
