mod interactive;

use std::io;
use std::io::IsTerminal;

use clap::Args;

use crate::core::git;
use crate::core::switch::{self, SwitchDisposition, SwitchOptions};

use super::CommandOutcome;

#[derive(Args, Debug, Clone, Default)]
pub struct SwitchArgs {
    /// Switch directly to the named local branch
    pub branch_name: Option<String>,
}

pub fn execute(args: SwitchArgs) -> io::Result<CommandOutcome> {
    match args
        .branch_name
        .as_deref()
        .map(str::trim)
        .filter(|branch_name| !branch_name.is_empty())
    {
        Some(branch_name) => execute_direct(branch_name),
        None => execute_interactive(),
    }
}

fn execute_direct(branch_name: &str) -> io::Result<CommandOutcome> {
    let outcome = switch::run(&SwitchOptions {
        branch_name: branch_name.to_string(),
    })?;

    if outcome.status.success() {
        match outcome.disposition {
            SwitchDisposition::Switched => println!("Switched to '{}'.", outcome.branch_name),
            SwitchDisposition::AlreadyCurrent => println!("Already on '{}'.", outcome.branch_name),
        }
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

fn execute_interactive() -> io::Result<CommandOutcome> {
    let view = switch::load_interactive_tree_view()?;

    let outcome = match interactive::scripted_events_from_env()? {
        Some(events) => interactive::run_scripted(&view, &events)?,
        None => {
            if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
                return Err(io::Error::other(
                    "dgr switch interactive mode requires an interactive terminal; pass a branch name to switch directly",
                ));
            }

            interactive::run(&view)?
        }
    };

    match outcome {
        interactive::InteractiveOutcome::Selected(branch_name) => execute_direct(&branch_name),
        interactive::InteractiveOutcome::Cancelled => Ok(CommandOutcome {
            status: git::success_status()?,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::SwitchArgs;

    #[test]
    fn preserves_optional_branch_name() {
        assert_eq!(
            SwitchArgs {
                branch_name: Some("feat/auth".into()),
            }
            .branch_name
            .as_deref(),
            Some("feat/auth")
        );
    }
}
