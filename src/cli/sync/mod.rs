mod render;

use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use clap::Args;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::time;

use crate::core::clean;
use crate::core::merge;
use crate::core::store::{PendingOperationKind, load_operation, open_initialized};
use crate::core::sync::{
    self, RemotePushActionKind, RemotePushOutcome, SyncCompletion, SyncEvent, SyncOptions,
    SyncStage,
};
use crate::core::tree::{self, TreeOptions, TreeView};

use super::CommandOutcome;
use super::common;

#[derive(Args, Debug, Clone, Default)]
pub struct SyncArgs {
    /// Continue a paused restack rebase sequence
    #[arg(short = 'c', long = "continue")]
    pub continue_operation: bool,
}

pub fn execute(args: SyncArgs) -> io::Result<CommandOutcome> {
    let animate = io::stdout().is_terminal();
    let runtime = if animate {
        Some(build_animation_runtime()?)
    } else {
        None
    };
    let outcome = if let Some(runtime) = runtime.as_ref() {
        let initial_local_view = load_initial_local_sync_view(&args)?;
        execute_sync_with_animation(runtime, args.clone(), initial_local_view)?
    } else {
        sync::run(&args.clone().into())?
    };

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
                let rendered_tree =
                    super::tree::render_focused_context_tree(&adopt_outcome.branch_name, None)?;
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
                let rendered_tree = super::tree::render_focused_context_tree(
                    &orphan_outcome.parent_branch_name,
                    Some((&orphan_outcome.branch_name, "(orphaned)")),
                )?;
                let output =
                    super::orphan::format_orphan_success_output(orphan_outcome, &rendered_tree);
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Reparent(reparent_outcome) if reparent_outcome.status.success() => {
                let rendered_tree =
                    super::tree::render_focused_context_tree(&reparent_outcome.branch_name, None)?;
                let output = super::reparent::format_reparent_success_output(
                    reparent_outcome,
                    &rendered_tree,
                );
                if !output.is_empty() {
                    println!("{output}");
                }
            }
            SyncCompletion::Full(full_outcome) if outcome.status.success() => {
                let summary = format_full_sync_summary(full_outcome);
                let mut printed_output = false;
                let mut restacked_branch_names = full_outcome
                    .restacked_branches
                    .iter()
                    .map(|branch| branch.branch_name.clone())
                    .collect::<Vec<_>>();
                let excluded_branch_names = full_outcome
                    .cleanup_plan
                    .candidates
                    .iter()
                    .map(|candidate| candidate.branch_name.clone())
                    .collect::<Vec<_>>();

                if !summary.is_empty() {
                    println!("{summary}");
                    printed_output = true;
                }

                if !full_outcome.cleanup_plan.candidates.is_empty() {
                    if printed_output {
                        println!();
                    }

                    println!(
                        "{}",
                        super::clean::format_clean_plan(&full_outcome.cleanup_plan)
                    );
                    printed_output = true;

                    if !super::clean::confirm_cleanup(&full_outcome.cleanup_plan)? {
                        println!("Skipped cleanup.");
                    } else {
                        println!();

                        let clean_outcome = if animate {
                            println!("Finished local sync. Moving on to cleanup.");
                            println!();
                            execute_cleanup_with_animation(
                                runtime
                                    .as_ref()
                                    .expect("animation runtime should exist for TTY sync"),
                                &full_outcome.cleanup_plan,
                            )?
                        } else {
                            clean::apply(&full_outcome.cleanup_plan)?
                        };
                        final_status = clean_outcome.status;

                        if clean_outcome.status.success() {
                            restacked_branch_names.extend(
                                clean_outcome
                                    .restacked_branches
                                    .iter()
                                    .map(|branch| branch.branch_name.clone()),
                            );
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

                if final_status.success() {
                    let pull_request_update_plan =
                        sync::plan_pull_request_updates(&restacked_branch_names)?;
                    if !pull_request_update_plan.actions.is_empty() {
                        if printed_output {
                            println!();
                        }

                        let updated_pull_requests =
                            sync::execute_pull_request_update_plan(&pull_request_update_plan)?;
                        let output =
                            format_pull_request_update_success_output(&updated_pull_requests);
                        if !output.is_empty() {
                            println!("{output}");
                            printed_output = true;
                        }
                    }
                }

                if final_status.success() {
                    let push_plan =
                        sync::plan_remote_pushes(&restacked_branch_names, &excluded_branch_names)?;

                    if !push_plan.actions.is_empty() {
                        if printed_output {
                            println!();
                        }

                        println!("{}", format_remote_push_plan(&push_plan));

                        if !confirm_remote_pushes()? {
                            println!("Skipped remote updates.");
                        } else {
                            println!();

                            let push_outcome = sync::execute_remote_push_plan(&push_plan)?;
                            final_status = push_outcome.status;

                            if push_outcome.status.success() {
                                let output = format_remote_push_success_output(&push_outcome);
                                if !output.is_empty() {
                                    println!("{output}");
                                }
                            } else {
                                let output = format_partial_remote_push_output(
                                    "Updated before failure:",
                                    &push_outcome,
                                );
                                if !output.is_empty() {
                                    println!("{output}");
                                    println!();
                                }
                                if let Some(failed_action) = push_outcome.failed_action.as_ref() {
                                    eprintln!(
                                        "Failed to update '{}' on '{}'.",
                                        failed_action.target.branch_name,
                                        failed_action.target.remote_name
                                    );
                                }
                                common::print_trimmed_stderr(
                                    push_outcome.failure_output.as_deref(),
                                );
                            }
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

    if animate && final_status.success() {
        let tree_outcome = tree::run(&TreeOptions::default())?;
        println!();
        println!("{}", render::render_completed_tree(&tree_outcome.view));
    }

    Ok(CommandOutcome {
        status: final_status,
    })
}

fn build_animation_runtime() -> io::Result<Runtime> {
    Builder::new_current_thread().enable_time().build()
}

fn load_initial_local_sync_view(args: &SyncArgs) -> io::Result<Option<TreeView>> {
    if !args.continue_operation {
        return Ok(Some(tree::run(&TreeOptions::default())?.view));
    }

    let session = open_initialized("dagger is not initialized; run 'dgr init' first")?;
    let pending_operation = load_operation(&session.paths)?;

    if matches!(
        pending_operation
            .as_ref()
            .map(|operation| &operation.origin),
        Some(PendingOperationKind::Sync(_))
    ) {
        Ok(Some(tree::run(&TreeOptions::default())?.view))
    } else {
        Ok(None)
    }
}

enum WorkerMessage<Event, Outcome> {
    Event(Event),
    Finished(io::Result<Outcome>),
}

struct SyncAnimationSession {
    terminal: Option<render::AnimationTerminal>,
    stage: Option<ActiveSyncStage>,
    initial_local_view: Option<TreeView>,
}

enum ActiveSyncStage {
    Local(render::SyncAnimation),
    Cleanup(super::clean::render::CleanAnimation),
}

impl SyncAnimationSession {
    fn new(initial_local_view: Option<TreeView>) -> Self {
        Self {
            terminal: None,
            stage: None,
            initial_local_view,
        }
    }

    fn apply(&mut self, event: SyncEvent) -> io::Result<()> {
        match event {
            SyncEvent::StageStarted(SyncStage::LocalSync { .. }) => {
                if let Some(ActiveSyncStage::Local(animation)) = self.stage.as_mut() {
                    if animation.apply_event(&event) {
                        self.render_active()?;
                    }

                    return Ok(());
                }

                let Some(view) = self.initial_local_view.take() else {
                    return Ok(());
                };

                let mut animation = render::SyncAnimation::new(&view);
                animation.apply_event(&event);
                let mut terminal = render::AnimationTerminal::start()?;
                terminal.render(&animation.render_active())?;
                self.terminal = Some(terminal);
                self.stage = Some(ActiveSyncStage::Local(animation));
                Ok(())
            }
            SyncEvent::StageStarted(SyncStage::CleanupResume {
                plan,
                active_branch_name,
                untracked_branches,
                deleted_branches,
                restacked_branches,
            }) => {
                let mut animation = super::clean::render::CleanAnimation::new(&plan);
                animation.prime_resume(
                    &restacked_branches,
                    &deleted_branches,
                    &untracked_branches,
                    &active_branch_name,
                );
                let mut terminal = render::AnimationTerminal::start()?;
                terminal.render(&animation.render_active())?;
                self.terminal = Some(terminal);
                self.stage = Some(ActiveSyncStage::Cleanup(animation));
                Ok(())
            }
            SyncEvent::Cleanup(clean_event) => {
                let Some(ActiveSyncStage::Cleanup(animation)) = self.stage.as_mut() else {
                    return Ok(());
                };

                if animation.apply_event(&clean_event) {
                    self.render_active()?;
                }

                Ok(())
            }
            _ => {
                let Some(ActiveSyncStage::Local(animation)) = self.stage.as_mut() else {
                    return Ok(());
                };

                if animation.apply_event(&event) {
                    self.render_active()?;
                }

                Ok(())
            }
        }
    }

    fn tick(&mut self) -> io::Result<()> {
        let Some(stage) = self.stage.as_mut() else {
            return Ok(());
        };

        let changed = match stage {
            ActiveSyncStage::Local(animation) => animation.tick(),
            ActiveSyncStage::Cleanup(animation) => animation.tick(),
        };

        if changed {
            self.render_active()?;
        }

        Ok(())
    }

    fn finish_success(&mut self) -> io::Result<()> {
        let Some(mut terminal) = self.terminal.take() else {
            return Ok(());
        };
        let Some(stage) = self.stage.take() else {
            return Ok(());
        };

        let frame = match stage {
            ActiveSyncStage::Local(animation) => animation.render_final(),
            ActiveSyncStage::Cleanup(animation) => animation.render_final(),
        };
        terminal.finish(&frame)
    }

    fn finish_failure(&mut self) -> io::Result<()> {
        let Some(mut terminal) = self.terminal.take() else {
            return Ok(());
        };
        let Some(stage) = self.stage.take() else {
            return Ok(());
        };

        let frame = match stage {
            ActiveSyncStage::Local(animation) => animation.render_active(),
            ActiveSyncStage::Cleanup(animation) => animation.render_active(),
        };
        terminal.finish(&frame)
    }

    fn render_active(&mut self) -> io::Result<()> {
        let Some(terminal) = self.terminal.as_mut() else {
            return Ok(());
        };
        let Some(stage) = self.stage.as_ref() else {
            return Ok(());
        };

        let frame = match stage {
            ActiveSyncStage::Local(animation) => animation.render_active(),
            ActiveSyncStage::Cleanup(animation) => animation.render_active(),
        };
        terminal.render(&frame)
    }
}

fn execute_sync_with_animation(
    runtime: &Runtime,
    args: SyncArgs,
    initial_local_view: Option<TreeView>,
) -> io::Result<sync::SyncOutcome> {
    runtime.block_on(execute_sync_with_animation_async(
        args.into(),
        initial_local_view,
    ))
}

async fn execute_sync_with_animation_async(
    options: SyncOptions,
    initial_local_view: Option<TreeView>,
) -> io::Result<sync::SyncOutcome> {
    let (sender, mut receiver) = mpsc::channel::<WorkerMessage<SyncEvent, sync::SyncOutcome>>(64);
    let worker = tokio::task::spawn_blocking(move || {
        let outcome = sync::run_with_reporter(&options, &mut |event| {
            let _ = sender.blocking_send(WorkerMessage::Event(event.clone()));
            Ok(())
        });
        let _ = sender.blocking_send(WorkerMessage::Finished(outcome));
    });

    let mut animation = SyncAnimationSession::new(initial_local_view);
    let outcome = drive_sync_animation(&mut animation, &mut receiver).await;
    let worker_result = worker.await;

    if let Err(err) = worker_result {
        return Err(io::Error::other(err.to_string()));
    }

    let outcome = outcome?;
    if outcome.status.success() {
        animation.finish_success()?;
    } else {
        animation.finish_failure()?;
    }

    Ok(outcome)
}

async fn drive_sync_animation(
    animation: &mut SyncAnimationSession,
    receiver: &mut mpsc::Receiver<WorkerMessage<SyncEvent, sync::SyncOutcome>>,
) -> io::Result<sync::SyncOutcome> {
    loop {
        match time::timeout(Duration::from_millis(80), receiver.recv()).await {
            Ok(Some(WorkerMessage::Event(event))) => animation.apply(event)?,
            Ok(Some(WorkerMessage::Finished(outcome))) => {
                return drain_worker_messages(receiver, |event| animation.apply(event), outcome);
            }
            Ok(None) => return Err(io::Error::other("sync animation worker ended unexpectedly")),
            Err(_) => animation.tick()?,
        }
    }
}

fn execute_cleanup_with_animation(
    runtime: &Runtime,
    plan: &clean::CleanPlan,
) -> io::Result<clean::CleanApplyOutcome> {
    runtime.block_on(execute_cleanup_with_animation_async(plan.clone()))
}

async fn execute_cleanup_with_animation_async(
    plan: clean::CleanPlan,
) -> io::Result<clean::CleanApplyOutcome> {
    let mut animation = super::clean::render::CleanAnimation::new(&plan);
    let mut terminal = render::AnimationTerminal::start()?;
    terminal.render(&animation.render_active())?;

    let (sender, mut receiver) =
        mpsc::channel::<WorkerMessage<clean::CleanEvent, clean::CleanApplyOutcome>>(64);
    let worker = tokio::task::spawn_blocking(move || {
        let outcome = clean::apply_with_reporter(&plan, &mut |event| {
            let _ = sender.blocking_send(WorkerMessage::Event(event.clone()));
            Ok(())
        });
        let _ = sender.blocking_send(WorkerMessage::Finished(outcome));
    });

    let outcome = drive_cleanup_animation(&mut terminal, &mut animation, &mut receiver).await;
    let worker_result = worker.await;

    if let Err(err) = worker_result {
        return Err(io::Error::other(err.to_string()));
    }

    let outcome = outcome?;

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

async fn drive_cleanup_animation(
    terminal: &mut render::AnimationTerminal,
    animation: &mut super::clean::render::CleanAnimation,
    receiver: &mut mpsc::Receiver<WorkerMessage<clean::CleanEvent, clean::CleanApplyOutcome>>,
) -> io::Result<clean::CleanApplyOutcome> {
    loop {
        match time::timeout(Duration::from_millis(80), receiver.recv()).await {
            Ok(Some(WorkerMessage::Event(event))) => {
                if animation.apply_event(&event) {
                    terminal.render(&animation.render_active())?;
                }
            }
            Ok(Some(WorkerMessage::Finished(outcome))) => {
                return drain_worker_messages(
                    receiver,
                    |event| {
                        if animation.apply_event(&event) {
                            terminal.render(&animation.render_active())?;
                        }

                        Ok(())
                    },
                    outcome,
                );
            }
            Ok(None) => {
                return Err(io::Error::other(
                    "cleanup animation worker ended unexpectedly",
                ));
            }
            Err(_) => {
                if animation.tick() {
                    terminal.render(&animation.render_active())?;
                }
            }
        }
    }
}

fn drain_worker_messages<Event, Outcome>(
    receiver: &mut mpsc::Receiver<WorkerMessage<Event, Outcome>>,
    mut apply_event: impl FnMut(Event) -> io::Result<()>,
    mut outcome: io::Result<Outcome>,
) -> io::Result<Outcome> {
    loop {
        match receiver.try_recv() {
            Ok(WorkerMessage::Event(event)) => apply_event(event)?,
            Ok(WorkerMessage::Finished(next_outcome)) => outcome = next_outcome,
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return outcome,
        }
    }
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

    if !outcome.repaired_pull_requests.is_empty() {
        let mut lines = vec!["Recovered pull requests:".to_string()];
        for repair in &outcome.repaired_pull_requests {
            lines.push(format!(
                "- {} (#{}): reopened as draft and retargeted from {} to {}",
                repair.branch_name,
                repair.pull_request_number,
                repair.old_base_branch_name,
                repair.new_base_branch_name
            ));
        }
        sections.push(lines.join("\n"));
    }

    if !outcome.deleted_branches.is_empty() {
        let mut lines = vec!["Deleted locally and no longer tracked by dagger:".to_string()];
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

fn format_remote_push_plan(plan: &sync::RemotePushPlan) -> String {
    let mut lines = vec!["Remote branches to update:".to_string()];

    for action in &plan.actions {
        let action_label = match action.kind {
            RemotePushActionKind::CreateRemoteBranch => "create",
            RemotePushActionKind::UpdateRemoteBranch => "push",
            RemotePushActionKind::ForceUpdateRemoteBranch => "force-push",
        };
        lines.push(format!(
            "- {action_label} {} on {}",
            action.target.branch_name, action.target.remote_name
        ));
    }

    lines.join("\n")
}

fn format_pull_request_update_success_output(
    updated_pull_requests: &[sync::PullRequestUpdateAction],
) -> String {
    if updated_pull_requests.is_empty() {
        return String::new();
    }

    let mut lines = vec!["Updated pull requests:".to_string()];
    for action in updated_pull_requests {
        lines.push(format!(
            "- retargeted #{} for {} to {}",
            action.pull_request_number, action.branch_name, action.new_base_branch_name
        ));
    }

    lines.join("\n")
}

fn confirm_remote_pushes() -> io::Result<bool> {
    common::confirm_yes_no("Push these remote updates? [y/N] ")
}

fn format_remote_push_success_output(outcome: &RemotePushOutcome) -> String {
    format_partial_remote_push_output("Updated remote branches:", outcome)
}

fn format_partial_remote_push_output(header: &str, outcome: &RemotePushOutcome) -> String {
    if outcome.pushed_actions.is_empty() {
        return String::new();
    }

    let mut lines = vec![header.to_string()];
    for action in &outcome.pushed_actions {
        let action_label = match action.kind {
            RemotePushActionKind::CreateRemoteBranch => "created",
            RemotePushActionKind::UpdateRemoteBranch => "pushed",
            RemotePushActionKind::ForceUpdateRemoteBranch => "force-pushed",
        };
        lines.push(format!(
            "- {action_label} {} on {}",
            action.target.branch_name, action.target.remote_name
        ));
    }

    lines.join("\n")
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
