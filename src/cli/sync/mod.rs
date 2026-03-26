use std::io;

use clap::Args;

use crate::core::clean;
use crate::core::merge;
use crate::core::sync::{self, SyncCompletion, SyncOptions};
use crate::core::tree;

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone, Default)]
pub struct SyncArgs {
    /// Continue a paused restack rebase sequence
    #[arg(short = 'c', long = "continue")]
    pub continue_operation: bool,
}

pub fn execute(args: SyncArgs) -> io::Result<CommandOutcome> {
    let outcome = sync::run(&args.clone().into())?;
    let mut final_status = outcome.status;

    if let Some(completion) = &outcome.completion {
        match completion {
            SyncCompletion::Commit(commit_outcome) => {
                let output = super::commit::format_commit_success_output(commit_outcome);
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Adopt(adopt_outcome) if adopt_outcome.status.success() => {
                let view = tree::focused_context_view(&adopt_outcome.branch_name)?;
                let rendered_tree = super::tree::render_stack_tree(&view);
                let output =
                    super::adopt::format_adopt_success_output(adopt_outcome, &rendered_tree);
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Merge(merge_outcome) if merge_outcome.outcome.status.success() => {
                let deleted_branch_name = if super::merge::confirm_delete_merged_branch(
                    &merge_outcome.source_branch_name,
                )? {
                    let delete_outcome = merge::delete_merged_branch_by_id(
                        merge_outcome.source_node_id,
                        &merge_outcome.target_branch_name,
                    )?;
                    if !delete_outcome.status.success() {
                        return Ok(CommandOutcome {
                            status: delete_outcome.status,
                        });
                    }
                    delete_outcome.deleted_branch_name
                } else {
                    None
                };

                let rendered_tree = super::merge::load_relative_tree(
                    &merge_outcome.target_branch_name,
                    &merge_outcome.trunk_branch,
                )?;
                let output = super::merge::format_merge_resume_success_output(
                    &merge_outcome.source_branch_name,
                    &merge_outcome.target_branch_name,
                    &merge_outcome.outcome,
                    deleted_branch_name.as_deref(),
                    &rendered_tree,
                );
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Clean {
                trunk_branch,
                outcome: clean_outcome,
            } if clean_outcome.status.success() => {
                let output = super::clean::format_clean_success_output(trunk_branch, clean_outcome);
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Orphan(orphan_outcome) if orphan_outcome.status.success() => {
                let view = tree::focused_context_view(&orphan_outcome.parent_branch_name)?;
                let rendered_tree = super::tree::render_stack_tree(&view);
                let output =
                    super::orphan::format_orphan_success_output(orphan_outcome, &rendered_tree);
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Full(full_outcome) if outcome.status.success() => {
                let summary = format_full_sync_summary(full_outcome);
                if !summary.is_empty() {
                    println!("{summary}");
                }

                if !full_outcome.cleanup_plan.candidates.is_empty() {
                    if !summary.is_empty() {
                        println!();
                    }

                    println!(
                        "{}",
                        super::clean::format_clean_plan(&full_outcome.cleanup_plan)
                    );

                    if !super::clean::confirm_cleanup(full_outcome.cleanup_plan.candidates.len())? {
                        println!("Skipped cleanup.");
                    } else {
                        println!();

                        let clean_outcome = clean::apply(&full_outcome.cleanup_plan)?;
                        final_status = clean_outcome.status;

                        if clean_outcome.status.success() {
                            let output = super::clean::format_clean_success_output(
                                &full_outcome.cleanup_plan.trunk_branch,
                                &clean_outcome,
                            );
                            if !output.is_empty() {
                                println!("{output}");
                            }
                        } else if clean_outcome.paused {
                            common::print_restack_pause_guidance(
                                clean_outcome.failure_output.as_deref(),
                            );
                        } else {
                            common::print_trimmed_stderr(clean_outcome.failure_output.as_deref());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if !outcome.status.success() {
        if outcome.paused {
            common::print_restack_pause_guidance(outcome.failure_output.as_deref());
        } else {
            common::print_trimmed_stderr(outcome.failure_output.as_deref());
        }
    }

    Ok(CommandOutcome {
        status: final_status,
    })
}

impl From<SyncArgs> for SyncOptions {
    fn from(args: SyncArgs) -> Self {
        Self {
            continue_operation: args.continue_operation,
        }
    }
}

fn format_full_sync_summary(outcome: &sync::FullSyncOutcome) -> String {
    let mut sections = Vec::new();

    if !outcome.deleted_branches.is_empty() {
        let mut lines = vec!["Deleted locally and no longer tracked by dig:".to_string()];
        for branch_name in &outcome.deleted_branches {
            lines.push(format!("- {branch_name}"));
        }
        sections.push(lines.join("\n"));
    }

    if !outcome.restacked_branches.is_empty() {
        sections.push(common::format_restacked_branches(
            &outcome.restacked_branches,
        ));
    }

    if sections.is_empty() && outcome.cleanup_plan.candidates.is_empty() {
        return "Local stacks are already in sync.".to_string();
    }

    common::join_sections(&sections)
}

#[cfg(test)]
mod tests {
    use super::SyncArgs;
    use crate::core::sync::SyncOptions;

    #[test]
    fn converts_cli_args_into_core_sync_options() {
        let options = SyncOptions::from(SyncArgs {
            continue_operation: true,
        });

        assert!(options.continue_operation);
    }
}
