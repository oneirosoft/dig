use std::io;
use std::process::ExitStatus;

use crate::core::git;
use crate::core::tree;
use crate::core::tree::TreeView;
use crate::core::workflow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOptions {
    pub branch_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchDisposition {
    Switched,
    AlreadyCurrent,
}

#[derive(Debug)]
pub struct SwitchOutcome {
    pub status: ExitStatus,
    pub branch_name: String,
    pub disposition: SwitchDisposition,
}

pub fn run(options: &SwitchOptions) -> io::Result<SwitchOutcome> {
    ensure_switch_allowed()?;

    let branch_name = options.branch_name.trim();
    if branch_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch name cannot be empty",
        ));
    }

    if !git::branch_exists(branch_name)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("branch '{branch_name}' was not found"),
        ));
    }

    if git::current_branch_name_if_any()?.as_deref() == Some(branch_name) {
        return Ok(SwitchOutcome {
            status: git::success_status()?,
            branch_name: branch_name.to_string(),
            disposition: SwitchDisposition::AlreadyCurrent,
        });
    }

    Ok(SwitchOutcome {
        status: git::switch_branch(branch_name)?,
        branch_name: branch_name.to_string(),
        disposition: SwitchDisposition::Switched,
    })
}

pub fn load_interactive_tree_view() -> io::Result<TreeView> {
    ensure_switch_allowed()?;
    tree::full_view(
        "dagger is not initialized; run 'dgr init' first or pass a branch name to switch directly",
    )
}

fn ensure_switch_allowed() -> io::Result<()> {
    workflow::ensure_no_pending_operation_for_command("switch")?;
    let repo = git::resolve_repo_context()?;
    git::ensure_no_in_progress_operations(&repo, "switch")
}
