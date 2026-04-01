mod render;

use std::io;
use std::io::IsTerminal;

use clap::{ArgAction, Args};

use crate::core::merge::{self, MergeMode, MergeOptions, MergeOutcome, MergePlan};
use crate::core::tree::{self, TreeOptions};

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone)]
pub struct MergeArgs {
    /// Perform a squash merge and create the squash commit
    #[arg(long = "squash")]
    pub squash: bool,

    /// Use the provided message paragraph(s) for the squash commit
    #[arg(short = 'm', long = "message", value_name = "MESSAGE", action = ArgAction::Append)]
    pub messages: Vec<String>,

    /// The tracked branch to merge into its tracked base
    pub branch_name: String,
}

pub fn execute(args: MergeArgs) -> io::Result<CommandOutcome> {
    let plan = merge::plan(&args.clone().into())?;
    let animate = io::stdout().is_terminal();

    println!("{}", format_merge_plan(&plan));
    println!();

    let outcome = if animate {
        execute_with_animation(&plan)?
    } else {
        execute_without_animation(&plan)?
    };

    if !outcome.status.success() {
        return Ok(CommandOutcome {
            status: outcome.status,
        });
    }

    let deleted_branch = if confirm_delete_merged_branch(&plan.source_branch_name)? {
        let delete_outcome = merge::delete_merged_branch(&plan)?;
        if !delete_outcome.status.success() {
            return Ok(CommandOutcome {
                status: delete_outcome.status,
            });
        }

        delete_outcome.deleted_branch_name
    } else {
        None
    };

    let tree = load_relative_tree(&plan.target_branch_name, &plan.trunk_branch)?;
    let output = format_merge_success_output(&plan, &outcome, deleted_branch.as_deref(), &tree);
    if !output.is_empty() {
        println!("{output}");
    }

    Ok(CommandOutcome {
        status: outcome.status,
    })
}

impl From<MergeArgs> for MergeOptions {
    fn from(args: MergeArgs) -> Self {
        Self {
            branch_name: args.branch_name,
            mode: if args.squash {
                MergeMode::Squash
            } else {
                MergeMode::Normal
            },
            messages: args.messages,
        }
    }
}

fn format_merge_plan(plan: &MergePlan) -> String {
    let mut lines = vec![
        "Merge plan:".to_string(),
        format!(
            "- {} into {}",
            plan.source_branch_name, plan.target_branch_name
        ),
    ];

    for restack in &plan.restack_plan {
        lines.push(format!(
            "  restack {} onto {}",
            restack.branch_name, restack.onto_branch
        ));
    }

    if plan.requires_target_checkout() {
        lines.push(String::new());
        lines.push(format!(
            "Will switch from '{}' to '{}' before merge.",
            plan.current_branch, plan.target_branch_name
        ));
    }

    lines.join("\n")
}

pub(crate) fn confirm_delete_merged_branch(branch_name: &str) -> io::Result<bool> {
    common::confirm_yes_no(&format!("Delete merged branch '{branch_name}'? [y/N] "))
}

fn execute_with_animation(plan: &MergePlan) -> io::Result<MergeOutcome> {
    let mut animation = render::MergeAnimation::new(plan);
    let mut terminal = render::AnimationTerminal::start()?;
    terminal.render(&animation.render_active())?;

    let outcome = merge::apply_with_reporter(plan, &mut |event| {
        if animation.apply_event(&event) {
            terminal.render(&animation.render_active())?;
        }

        Ok(())
    })?;

    if outcome.status.success() {
        terminal.finish(&animation.render_final())?;
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

fn execute_without_animation(plan: &MergePlan) -> io::Result<MergeOutcome> {
    let outcome = merge::apply(plan)?;

    if !outcome.status.success() {
        if outcome.paused {
            common::print_restack_pause_guidance(outcome.failure_output.as_deref());
        } else {
            common::print_trimmed_stderr(outcome.failure_output.as_deref());
        }
    }

    Ok(outcome)
}

pub(crate) fn load_relative_tree(current_branch: &str, trunk_branch: &str) -> io::Result<String> {
    let options = if current_branch == trunk_branch {
        TreeOptions::default()
    } else {
        TreeOptions {
            branch_name: Some(current_branch.to_string()),
        }
    };
    let outcome = tree::run(&options)?;

    Ok(super::tree::render_stack_tree(&outcome.view))
}

fn format_merge_success_output(
    plan: &MergePlan,
    outcome: &MergeOutcome,
    deleted_branch_name: Option<&str>,
    rendered_tree: &str,
) -> String {
    format_merge_resume_success_output(
        &plan.source_branch_name,
        &plan.target_branch_name,
        outcome,
        deleted_branch_name,
        rendered_tree,
    )
}

pub(crate) fn format_merge_resume_success_output(
    source_branch_name: &str,
    target_branch_name: &str,
    outcome: &MergeOutcome,
    deleted_branch_name: Option<&str>,
    rendered_tree: &str,
) -> String {
    let mut sections = Vec::new();
    let mut summary_lines = Vec::new();

    if let Some(previous_branch) = &outcome.switched_to_target_from {
        summary_lines.push(format!(
            "Switched from '{}' to '{}' before merge.",
            previous_branch, target_branch_name
        ));
    }

    summary_lines.push(format!(
        "Merged '{}' into '{}'.",
        source_branch_name, target_branch_name
    ));

    if !outcome.restacked_branches.is_empty() {
        summary_lines.push(String::new());
        summary_lines.push(common::format_restacked_branches(
            &outcome.restacked_branches,
        ));
    }

    match deleted_branch_name {
        Some(branch_name) => {
            summary_lines.push(String::new());
            summary_lines.push("Deleted:".to_string());
            summary_lines.push(format!("- {branch_name}"));
        }
        None => {
            summary_lines.push(String::new());
            summary_lines.push(format!("Kept merged branch '{}'.", source_branch_name));
        }
    }

    sections.push(summary_lines.join("\n"));
    if !rendered_tree.trim().is_empty() {
        sections.push(rendered_tree.to_string());
    }

    common::join_sections(&sections)
}

#[cfg(test)]
mod tests {
    use super::{MergeArgs, format_merge_plan, format_merge_success_output};
    use crate::core::merge::{MergeMode, MergeOptions, MergePlan, MergeTreeNode};
    use crate::core::restack::RestackPreview;
    use std::process::ExitStatus;
    use uuid::Uuid;

    fn exit_status_success() -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
    }

    #[test]
    fn converts_cli_args_into_core_merge_options() {
        let options = MergeOptions::from(MergeArgs {
            squash: true,
            messages: vec!["subject".into()],
            branch_name: "feat/auth".into(),
        });

        assert_eq!(
            options,
            MergeOptions {
                branch_name: "feat/auth".into(),
                mode: MergeMode::Squash,
                messages: vec!["subject".into()],
            }
        );
    }

    #[test]
    fn formats_merge_plan_with_restack_preview() {
        let rendered = format_merge_plan(&MergePlan {
            trunk_branch: "main".into(),
            current_branch: "feat/auth-api".into(),
            source_branch_name: "feat/auth-api".into(),
            target_branch_name: "feat/auth".into(),
            source_node_id: Uuid::new_v4(),
            mode: MergeMode::Normal,
            messages: vec![],
            tree: MergeTreeNode {
                branch_name: "feat/auth-api".into(),
                children: vec![],
            },
            restack_plan: vec![RestackPreview {
                branch_name: "feat/auth-api-tests".into(),
                onto_branch: "feat/auth".into(),
                parent_changed: true,
            }],
        });

        assert_eq!(
            rendered,
            concat!(
                "Merge plan:\n",
                "- feat/auth-api into feat/auth\n",
                "  restack feat/auth-api-tests onto feat/auth\n",
                "\n",
                "Will switch from 'feat/auth-api' to 'feat/auth' before merge."
            )
        );
    }

    #[test]
    fn formats_merge_success_summary_and_tree() {
        let rendered = format_merge_success_output(
            &MergePlan {
                trunk_branch: "main".into(),
                current_branch: "feat/auth".into(),
                source_branch_name: "feat/auth-api".into(),
                target_branch_name: "feat/auth".into(),
                source_node_id: Uuid::new_v4(),
                mode: MergeMode::Normal,
                messages: vec![],
                tree: MergeTreeNode {
                    branch_name: "feat/auth-api".into(),
                    children: vec![],
                },
                restack_plan: vec![],
            },
            &crate::core::merge::MergeOutcome {
                status: exit_status_success(),
                switched_to_target_from: Some("feat/auth-api".into()),
                restacked_branches: vec![RestackPreview {
                    branch_name: "feat/auth-api-tests".into(),
                    onto_branch: "feat/auth".into(),
                    parent_changed: true,
                }],
                failure_output: None,
                paused: false,
            },
            Some("feat/auth-api"),
            "feat/auth\n└── feat/auth-api-tests",
        );

        assert_eq!(
            rendered,
            concat!(
                "Switched from 'feat/auth-api' to 'feat/auth' before merge.\n",
                "Merged 'feat/auth-api' into 'feat/auth'.\n",
                "\n",
                "Restacked:\n",
                "- feat/auth-api-tests onto feat/auth\n",
                "\n",
                "Deleted:\n",
                "- feat/auth-api\n",
                "\n",
                "feat/auth\n",
                "└── feat/auth-api-tests"
            )
        );
    }
}
