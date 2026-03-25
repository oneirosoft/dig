use std::io;

use clap::Args;

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
        status: outcome.status,
    })
}

impl From<SyncArgs> for SyncOptions {
    fn from(args: SyncArgs) -> Self {
        Self {
            continue_operation: args.continue_operation,
        }
    }
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
