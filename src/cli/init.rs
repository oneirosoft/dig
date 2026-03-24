use std::io;

use clap::Args;

use crate::core::init::{self, InitOptions};

use super::CommandOutcome;

#[derive(Args, Debug, Clone, Default)]
pub struct InitArgs {}

pub fn execute(args: InitArgs) -> io::Result<CommandOutcome> {
    let outcome = init::run(&args.into())?;

    if outcome.created_git_repo {
        println!("Initialized Git repository.");
    } else {
        println!("Using existing Git repository.");
    }

    if outcome.store_initialization.created_anything() {
        println!("Initialized dig.");
    } else {
        println!("Dig is already initialized.");
    }

    println!();
    println!("{}", super::tree::render_branch_lineage(&outcome.lineage));

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<InitArgs> for InitOptions {
    fn from(_: InitArgs) -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::InitArgs;
    use crate::core::init::InitOptions;

    #[test]
    fn converts_cli_args_into_core_init_options() {
        assert_eq!(InitOptions::from(InitArgs::default()), InitOptions::default());
    }
}
