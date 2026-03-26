mod render;

use std::io;
use std::io::IsTerminal;
use std::thread;
use std::time::Duration;

use clap::Args;

use crate::core::clean::{self, CleanBlockReason, CleanOptions, CleanPlan, CleanReason};
use crate::core::git;

use super::CommandOutcome;
use super::common;

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

    if !confirm_cleanup(&plan)? {
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

pub(crate) fn format_clean_plan(plan: &CleanPlan) -> String {
    let deleted_candidates = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.is_deleted_locally())
        .collect::<Vec<_>>();
    let merged_candidates = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.is_integrated())
        .collect::<Vec<_>>();
    let mut sections = Vec::new();

    if !deleted_candidates.is_empty() {
        let mut lines = vec!["Tracked branches missing locally and ready to stop tracking:".to_string()];

        for candidate in deleted_candidates {
            lines.push(format!("- {} no longer exists locally", candidate.branch_name));

            for restack in &candidate.restack_plan {
                lines.push(format!(
                    "  restack {} onto {}",
                    restack.branch_name, restack.onto_branch
                ));
            }
        }

        sections.push(lines.join("\n"));
    }

    if !merged_candidates.is_empty() {
        let mut lines = vec!["Merged branches ready to clean:".to_string()];

        for candidate in merged_candidates {
            let parent_branch = match &candidate.reason {
                CleanReason::DeletedLocally => continue,
                CleanReason::IntegratedIntoParent { parent_branch } => parent_branch,
            };

            lines.push(format!(
                "- {} merged into {}",
                candidate.branch_name, parent_branch
            ));

            for restack in &candidate.restack_plan {
                lines.push(format!(
                    "  restack {} onto {}",
                    restack.branch_name, restack.onto_branch
                ));
            }
        }

        sections.push(lines.join("\n"));
    }

    let mut rendered = common::join_sections(&sections);
    if plan.targets_current_branch() && plan.current_branch != plan.trunk_branch {
        if !rendered.is_empty() {
            rendered.push_str("\n\n");
        }
        rendered.push_str(&format!(
            "Will switch from '{}' to '{}' before cleanup.",
            plan.current_branch, plan.trunk_branch
        ));
    }

    rendered
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

pub(crate) fn confirm_cleanup(plan: &CleanPlan) -> io::Result<bool> {
    let missing_count = plan.deleted_local_count();
    let merged_count = plan.merged_count();
    let merged_label = if merged_count == 1 { "branch" } else { "branches" };
    let missing_label = if missing_count == 1 { "branch" } else { "branches" };

    let prompt = match (missing_count, merged_count) {
        (0, merged) => format!("Delete {merged} merged {merged_label}? [y/N] "),
        (missing, 0) => format!("Stop tracking {missing} missing {missing_label}? [y/N] "),
        (missing, merged) => format!(
            "Delete {merged} merged {merged_label} and stop tracking {missing} missing {missing_label}? [y/N] "
        ),
    };

    common::confirm_yes_no(&prompt)
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

        if let Some(restored_branch) = &outcome.restored_original_branch {
            println!("Returned to '{}' after cleanup.", restored_branch);
        }
    } else {
        terminal.finish(&animation.render_active())?;
        if outcome.paused {
            common::print_restack_pause_guidance(outcome.failure_output.as_deref());
        } else {
            common::print_trimmed_stderr(outcome.failure_output.as_deref());
        }
    }

    Ok(outcome)
}

fn execute_without_animation(
    plan: &CleanPlan,
) -> io::Result<crate::core::clean::CleanApplyOutcome> {
    let outcome = clean::apply(plan)?;

    if outcome.status.success() {
        let output = format_clean_success_output(&plan.trunk_branch, &outcome);
        if !output.is_empty() {
            println!("{output}");
        }
    } else if outcome.paused {
        common::print_restack_pause_guidance(outcome.failure_output.as_deref());
    } else {
        common::print_trimmed_stderr(outcome.failure_output.as_deref());
    }

    Ok(outcome)
}

pub(crate) fn format_clean_success_output(
    trunk_branch: &str,
    outcome: &crate::core::clean::CleanApplyOutcome,
) -> String {
    let mut lines = Vec::new();

    if let Some(previous_branch) = outcome.switched_to_trunk_from.as_ref() {
        lines.push(format!(
            "Switched from '{}' to '{}' before cleanup.",
            previous_branch, trunk_branch
        ));
    }

    if let Some(restored_branch) = outcome.restored_original_branch.as_ref() {
        lines.push(format!("Returned to '{}' after cleanup.", restored_branch));
    }

    if !outcome.restacked_branches.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(common::format_restacked_branches(
            &outcome.restacked_branches,
        ));
    }

    if !outcome.untracked_branches.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("No longer tracked by dig:".to_string());
        for branch_name in &outcome.untracked_branches {
            lines.push(format!("- {branch_name}"));
        }
    }

    if !outcome.deleted_branches.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("Deleted:".to_string());
        for branch_name in &outcome.deleted_branches {
            lines.push(format!("- {branch_name}"));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{CleanArgs, format_blocked_branch, format_clean_plan};
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
    fn formats_clean_plan_with_deleted_local_section() {
        let rendered = format_clean_plan(&CleanPlan {
            trunk_branch: "main".into(),
            current_branch: "main".into(),
            requested_branch_name: None,
            candidates: vec![CleanCandidate {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "main".into(),
                reason: CleanReason::DeletedLocally,
                tree: CleanTreeNode {
                    branch_name: "feat/auth".into(),
                    children: vec![],
                },
                restack_plan: vec![RestackPreview {
                    branch_name: "feat/users".into(),
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
                "Tracked branches missing locally and ready to stop tracking:\n",
                "- feat/auth no longer exists locally\n",
                "  restack feat/users onto main"
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
