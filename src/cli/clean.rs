#[path = "clean/render.rs"]
mod render;

use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::thread;
use std::time::Duration;

use clap::Args;

use crate::core::clean::{self, CleanBlockReason, CleanOptions, CleanPlan, CleanReason};
use crate::core::git;

use super::CommandOutcome;

#[derive(Args, Debug, Clone, Default)]
pub struct CleanArgs {
    /// Limit cleanup to a single tracked branch
    #[arg(long = "branch", value_name = "BRANCH")]
    pub branch_name: Option<String>,
}

pub fn execute(args: CleanArgs) -> io::Result<CommandOutcome> {
    let plan = clean::plan(&args.clone().into())?;
    let animate = io::stdout().is_terminal();

    if plan.candidates.is_empty() {
        if let Some(blocked) = plan.blocked.first() {
            println!("{}", format_blocked_branch(blocked));
        } else {
            println!("No merged branches are ready to clean.");
        }

        return Ok(CommandOutcome {
            status: git::success_status()?,
        });
    }

    println!("{}", format_clean_plan(&plan));

    let branch_count = plan.candidates.len();
    if !confirm_cleanup(branch_count)? {
        println!("Aborted.");
        return Ok(CommandOutcome {
            status: git::success_status()?,
        });
    }

    println!();
    let outcome = if animate {
        execute_with_animation(&plan)?
    } else {
        execute_without_animation(&plan)?
    };

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<CleanArgs> for CleanOptions {
    fn from(args: CleanArgs) -> Self {
        Self {
            branch_name: args.branch_name,
        }
    }
}

fn format_clean_plan(plan: &CleanPlan) -> String {
    let mut lines = vec!["Merged branches ready to clean:".to_string()];

    for candidate in &plan.candidates {
        let parent_branch = match &candidate.reason {
            CleanReason::IntegratedIntoParent { parent_branch } => parent_branch,
        };

        lines.push(format!(
            "- {} merged into {}",
            candidate.branch_name, parent_branch
        ));

        for restack in &candidate.restack_plan {
            lines.push(format!("  restack {} onto {}", restack.branch_name, restack.onto_branch));
        }
    }

    if plan.targets_current_branch() && plan.current_branch != plan.trunk_branch {
        lines.push(String::new());
        lines.push(format!(
            "Will switch from '{}' to '{}' before cleanup.",
            plan.current_branch, plan.trunk_branch
        ));
    }

    lines.join("\n")
}

fn format_blocked_branch(blocked: &crate::core::clean::BlockedBranch) -> String {
    match &blocked.reason {
        CleanBlockReason::BranchNotTracked => {
            format!("'{}' is not tracked by dig.", blocked.branch_name)
        }
        CleanBlockReason::BranchMissingLocally => format!(
            "'{}' is tracked by dig but no longer exists locally.",
            blocked.branch_name
        ),
        CleanBlockReason::ParentMissingLocally { parent_branch } => format!(
            "'{}' cannot be cleaned because its parent '{}' does not exist locally.",
            blocked.branch_name, parent_branch
        ),
        CleanBlockReason::ParentMissingFromDig => format!(
            "'{}' cannot be cleaned because its tracked parent is missing from dig.",
            blocked.branch_name
        ),
        CleanBlockReason::NotIntegrated { parent_branch } => format!(
            "'{}' is not fully integrated into '{}'.",
            blocked.branch_name, parent_branch
        ),
        CleanBlockReason::DescendantsMissingLocally { branch_names } => format!(
            "'{}' cannot be cleaned because tracked descendants are missing locally: {}.",
            blocked.branch_name,
            branch_names.join(", ")
        ),
    }
}

fn confirm_cleanup(branch_count: usize) -> io::Result<bool> {
    let mut stdout = io::stdout();
    let label = if branch_count == 1 { "branch" } else { "branches" };

    write!(stdout, "Delete {branch_count} merged {label}? [y/N] ")?;
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

fn execute_with_animation(plan: &CleanPlan) -> io::Result<crate::core::clean::CleanApplyOutcome> {
    let mut animation = render::CleanAnimation::new(plan);
    let mut terminal = render::AnimationTerminal::start()?;
    terminal.render(&animation.render_active())?;

    let outcome = clean::apply_with_reporter(plan, &mut |event| {
        if animation.apply_event(&event) {
            terminal.render(&animation.render_active())?;
        }

        Ok(())
    })?;

    if outcome.status.success() {
        thread::sleep(Duration::from_millis(350));
        terminal.finish(&animation.render_final())?;

        if let Some(previous_branch) = &outcome.switched_to_trunk_from {
            println!(
                "Switched from '{}' to '{}' before cleanup.",
                previous_branch, plan.trunk_branch
            );
        }
    } else {
        terminal.finish(&animation.render_active())?;

        if let Some(failure_output) = &outcome.failure_output {
            let trimmed = failure_output.trim();
            if !trimmed.is_empty() {
                eprintln!("{trimmed}");
            }
        }
    }

    Ok(outcome)
}

fn execute_without_animation(plan: &CleanPlan) -> io::Result<crate::core::clean::CleanApplyOutcome> {
    let outcome = clean::apply(plan)?;

    if outcome.status.success() {
        if let Some(previous_branch) = outcome.switched_to_trunk_from.as_ref() {
            println!(
                "Switched from '{}' to '{}' before cleanup.",
                previous_branch, plan.trunk_branch
            );
        }

        if !outcome.restacked_branches.is_empty() {
            println!("Restacked:");
            for branch in &outcome.restacked_branches {
                println!("- {} onto {}", branch.branch_name, branch.onto_branch);
            }
        }

        if !outcome.deleted_branches.is_empty() {
            if !outcome.restacked_branches.is_empty() {
                println!();
            }

            println!("Deleted:");
            for branch_name in &outcome.deleted_branches {
                println!("- {branch_name}");
            }
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::{format_blocked_branch, format_clean_plan, CleanArgs};
    use crate::core::clean::{
        BlockedBranch, CleanBlockReason, CleanCandidate, CleanOptions, CleanPlan, CleanReason,
        CleanTreeNode,
    };
    use crate::core::restack::RestackPreview;
    use uuid::Uuid;

    #[test]
    fn converts_cli_args_into_core_clean_options() {
        let args = CleanArgs {
            branch_name: Some("feat/auth".into()),
        };

        let options = CleanOptions::from(args);

        assert_eq!(options.branch_name.as_deref(), Some("feat/auth"));
    }

    #[test]
    fn formats_clean_plan_with_restack_preview() {
        let rendered = format_clean_plan(&CleanPlan {
            trunk_branch: "main".into(),
            current_branch: "feat/auth".into(),
            requested_branch_name: Some("feat/auth".into()),
            candidates: vec![CleanCandidate {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "main".into(),
                reason: CleanReason::IntegratedIntoParent {
                    parent_branch: "main".into(),
                },
                tree: CleanTreeNode {
                    branch_name: "feat/auth".into(),
                    children: vec![CleanTreeNode {
                        branch_name: "feat/auth-api".into(),
                        children: vec![],
                    }],
                },
                restack_plan: vec![RestackPreview {
                    branch_name: "feat/auth-api".into(),
                    onto_branch: "main".into(),
                    parent_changed: true,
                }],
                depth: 0,
            }],
            blocked: Vec::new(),
        });

        assert_eq!(
            rendered,
            concat!(
                "Merged branches ready to clean:\n",
                "- feat/auth merged into main\n",
                "  restack feat/auth-api onto main\n",
                "\n",
                "Will switch from 'feat/auth' to 'main' before cleanup."
            )
        );
    }

    #[test]
    fn formats_blocked_branch_reason() {
        let rendered = format_blocked_branch(&BlockedBranch {
            branch_name: "feat/auth".into(),
            reason: CleanBlockReason::NotIntegrated {
                parent_branch: "main".into(),
            },
        });

        assert_eq!(rendered, "'feat/auth' is not fully integrated into 'main'.");
    }
}
