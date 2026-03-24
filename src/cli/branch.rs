use std::io;

use clap::Args;

use crate::core::branch::{self, BranchOptions};

use super::CommandOutcome;

#[derive(Args, Debug, Clone)]
pub struct BranchArgs {
    /// The name of the branch to create from the current branch
    pub name: String,

    /// Override the tracked dig parent branch
    #[arg(short = 'p', long = "parent", value_name = "BRANCH")]
    pub parent_branch_name: Option<String>,
}

pub fn execute(args: BranchArgs) -> io::Result<CommandOutcome> {
    let outcome = branch::run(&args.clone().into())?;

    if outcome.status.success() {
        if let Some(node) = &outcome.created_node {
            println!("Created and switched to '{}'.", node.branch_name);
            println!();
            println!("{}", super::tree::render_branch_lineage(&outcome.lineage));
        }
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<BranchArgs> for BranchOptions {
    fn from(args: BranchArgs) -> Self {
        Self {
            name: args.name,
            parent_branch_name: args.parent_branch_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BranchArgs;
    use crate::core::branch::BranchOptions;

    #[test]
    fn converts_cli_args_into_core_branch_options() {
        let args = BranchArgs {
            name: "feature/api".into(),
            parent_branch_name: Some("main".into()),
        };

        let options = BranchOptions::from(args);

        assert_eq!(options.name, "feature/api");
        assert_eq!(options.parent_branch_name.as_deref(), Some("main"));
    }
}
