mod render;
mod rows;

use std::io;

use clap::Args;

use crate::core::tree::{self, TreeOptions};

use super::CommandOutcome;

pub(super) use render::{render_branch_lineage, render_stack_tree};
pub(super) use rows::{StackTreeRow, stack_tree_rows};

#[derive(Args, Debug, Clone, Default)]
pub struct TreeArgs {
    /// Show only the selected tracked branch stack
    #[arg(long = "branch", value_name = "BRANCH")]
    pub branch_name: Option<String>,
}

pub fn execute(args: TreeArgs) -> io::Result<CommandOutcome> {
    let outcome = tree::run(&args.clone().into())?;

    println!("{}", render::render_stack_tree(&outcome.view));

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<TreeArgs> for TreeOptions {
    fn from(args: TreeArgs) -> Self {
        Self {
            branch_name: args.branch_name,
        }
    }
}

pub(super) fn render_focused_context_tree(
    branch_name: &str,
    suffix_for_current_branch: Option<(&str, &str)>,
) -> io::Result<String> {
    let mut view = tree::focused_context_view(branch_name)?;

    if let Some((current_branch_name, suffix)) = suffix_for_current_branch {
        if view.current_branch_name.as_deref() == Some(current_branch_name) {
            view.current_branch_suffix = Some(suffix.to_string());
        }
    }

    Ok(render::render_stack_tree(&view))
}

#[cfg(test)]
mod tests {
    use super::TreeArgs;
    use crate::core::tree::TreeOptions;

    #[test]
    fn converts_cli_args_into_core_tree_options() {
        let options = TreeOptions::from(TreeArgs {
            branch_name: Some("feat/auth".into()),
        });

        assert_eq!(options.branch_name.as_deref(), Some("feat/auth"));
    }
}
