mod adopt;
mod branch;
mod clean;
mod commit;
mod common;
mod init;
mod merge;
mod operation;
mod orphan;
mod sync;
mod tree;

use std::io;
use std::process::ExitCode;
use std::process::ExitStatus;

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
    /// Adopt an existing local branch under a tracked dig parent
    Adopt(adopt::AdoptArgs),

    /// Create a new branch from the currently checked out branch and track it in dig
    Branch(branch::BranchArgs),

    /// Clean merged tracked branches and restack their descendants
    Clean(clean::CleanArgs),

    /// Initialize the current directory as a git repository
    Init(init::InitArgs),

    /// Wrap git commit with limited passthrough flags
    Commit(commit::CommitArgs),

    /// Merge a tracked branch into its tracked base and restack descendants
    Merge(merge::MergeArgs),

    /// Stop tracking a branch in dig while keeping the local branch
    Orphan(orphan::OrphanArgs),

    /// Continue a paused restack sequence
    Sync(sync::SyncArgs),

    /// Print the tracked branch stacks as a shared tree from trunk
    Tree(tree::TreeArgs),
}

#[derive(Debug)]
pub struct CommandOutcome {
    pub status: ExitStatus,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Adopt(args) => adopt::execute(args),
        Commands::Branch(args) => branch::execute(args),
        Commands::Clean(args) => clean::execute(args),
        Commands::Init(args) => init::execute(args),
        Commands::Commit(args) => commit::execute(args),
        Commands::Merge(args) => merge::execute(args),
        Commands::Orphan(args) => orphan::execute(args),
        Commands::Sync(args) => sync::execute(args),
        Commands::Tree(args) => tree::execute(args),
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
