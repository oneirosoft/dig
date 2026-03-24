mod branch;
mod init;
mod commit;
mod tree;

use std::io;
use std::process::ExitStatus;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "dig")]
#[command(about = "Git wrapper for stacked PR workflows")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new branch from the currently checked out branch and track it in dig
    Branch(branch::BranchArgs),

    /// Initialize the current directory as a git repository
    Init(init::InitArgs),

    /// Wrap git commit with limited passthrough flags
    Commit(commit::CommitArgs),
}

#[derive(Debug)]
pub struct CommandOutcome {
    pub status: ExitStatus,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Branch(args) => branch::execute(args),
        Commands::Init(args) => init::execute(args),
        Commands::Commit(args) => commit::execute(args),
    };

    exit_code_from_result(result)
}

fn exit_code_from_result(result: io::Result<CommandOutcome>) -> ExitCode {
    match result {
        Ok(outcome) if outcome.status.success() => ExitCode::SUCCESS,
        Ok(outcome) => ExitCode::from(outcome.status.code().unwrap_or(1) as u8),
        Err(err) => {
            eprintln!("dig: {err}");
            ExitCode::FAILURE
        }
    }
}
