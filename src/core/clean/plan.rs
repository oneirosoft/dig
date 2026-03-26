use std::collections::HashSet;
use std::io;

use crate::core::deleted_local;
use crate::core::git::{self, CherryMarker, CommitMetadata};
use crate::core::graph::BranchGraph;
use crate::core::restack::{self, RestackBaseTarget};
use crate::core::store::types::DigState;
use crate::core::store::{
    BranchNode, PendingCleanCandidate, PendingCleanCandidateKind, PendingCleanOperation, dig_paths,
    load_config, load_state,
};
use crate::core::workflow;

use super::types::{
    BlockedBranch, CleanBlockReason, CleanCandidate, CleanOptions, CleanPlan, CleanReason,
};

#[derive(Debug)]
enum BranchEvaluation {
    Cleanable(CleanCandidate),
    Blocked(BlockedBranch),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanPlanMode {
    LocalOnly,
    RemoteAwareSync,
}

pub(crate) fn plan(options: &CleanOptions) -> io::Result<CleanPlan> {
    plan_with_mode(options, CleanPlanMode::LocalOnly)
}

pub(crate) fn plan_for_sync() -> io::Result<CleanPlan> {
    plan_with_mode(&CleanOptions::default(), CleanPlanMode::RemoteAwareSync)
}

pub(crate) fn plan_for_resume(payload: &PendingCleanOperation) -> io::Result<CleanPlan> {
    let repo = git::resolve_repo_context()?;
    let store_paths = dig_paths(&repo.git_dir);
    let state = load_state(&store_paths)?;
    let current_branch = git::current_branch_name_or(&payload.original_branch)?;
    let mut candidates = Vec::with_capacity(1 + payload.remaining_candidates.len());

    candidates.push(clean_candidate_from_pending_candidate(
        &state,
        &payload.trunk_branch,
        &payload.current_candidate,
    )?);

    for candidate in &payload.remaining_candidates {
        candidates.push(clean_candidate_from_pending_candidate(
            &state,
            &payload.trunk_branch,
            candidate,
        )?);
    }

    Ok(CleanPlan {
        trunk_branch: payload.trunk_branch.clone(),
        current_branch,
        requested_branch_name: None,
        candidates,
        blocked: Vec::new(),
    })
}

pub(crate) fn mode_for_sync(remote_sync_enabled: bool) -> CleanPlanMode {
    if remote_sync_enabled {
        CleanPlanMode::RemoteAwareSync
    } else {
        CleanPlanMode::LocalOnly
    }
}

fn plan_with_mode(options: &CleanOptions, mode: CleanPlanMode) -> io::Result<CleanPlan> {
    workflow::ensure_no_pending_operation_for_command("clean")?;
    let repo = git::resolve_repo_context()?;
    let store_paths = dig_paths(&repo.git_dir);
    let config = load_config(&store_paths)?
        .ok_or_else(|| io::Error::other("dig is not initialized; run 'dig init' first"))?;
    let state = load_state(&store_paths)?;
    let current_branch = git::current_branch_name()?;
    let requested_branch_name = options
        .branch_name
        .as_deref()
        .map(str::trim)
        .filter(|branch_name| !branch_name.is_empty())
        .map(str::to_string);

    match &requested_branch_name {
        Some(branch_name) => plan_for_requested_branch(
            &state,
            &config.trunk_branch,
            &current_branch,
            branch_name,
            mode,
        ),
        None => plan_for_all_branches(&state, &config.trunk_branch, &current_branch, mode),
    }
}

fn plan_for_requested_branch(
    state: &DigState,
    trunk_branch: &str,
    current_branch: &str,
    branch_name: &str,
    mode: CleanPlanMode,
) -> io::Result<CleanPlan> {
    let Some(node) = state.find_branch_by_name(branch_name) else {
        return Ok(CleanPlan {
            trunk_branch: trunk_branch.to_string(),
            current_branch: current_branch.to_string(),
            requested_branch_name: Some(branch_name.to_string()),
            candidates: Vec::new(),
            blocked: vec![BlockedBranch {
                branch_name: branch_name.to_string(),
                reason: CleanBlockReason::BranchNotTracked,
            }],
        });
    };

    if !git::branch_exists(&node.branch_name)? {
        let candidates =
            deleted_local::collect_deleted_local_subtree_steps(state, trunk_branch, node.id)?
                .into_iter()
                .map(clean_candidate_from_deleted_step)
                .collect();

        return Ok(CleanPlan {
            trunk_branch: trunk_branch.to_string(),
            current_branch: current_branch.to_string(),
            requested_branch_name: Some(branch_name.to_string()),
            candidates,
            blocked: Vec::new(),
        });
    }

    let evaluation = evaluate_integrated_branch(state, trunk_branch, node, mode)?;

    let (candidates, blocked) = match evaluation {
        BranchEvaluation::Cleanable(candidate) => (vec![candidate], Vec::new()),
        BranchEvaluation::Blocked(blocked) => (Vec::new(), vec![blocked]),
    };

    Ok(CleanPlan {
        trunk_branch: trunk_branch.to_string(),
        current_branch: current_branch.to_string(),
        requested_branch_name: Some(branch_name.to_string()),
        candidates,
        blocked,
    })
}

fn plan_for_all_branches(
    state: &DigState,
    trunk_branch: &str,
    current_branch: &str,
    mode: CleanPlanMode,
) -> io::Result<CleanPlan> {
    let deleted_steps = deleted_local::collect_deleted_local_steps(state, trunk_branch)?;
    let deleted_candidates = deleted_steps
        .iter()
        .cloned()
        .map(clean_candidate_from_deleted_step)
        .collect::<Vec<_>>();
    let projected_state = apply_deleted_step_projection(state, &deleted_steps)?;

    let mut cleanable = Vec::new();
    let mut blocked = Vec::new();

    for node in projected_state.nodes.iter().filter(|node| !node.archived) {
        match evaluate_integrated_branch(&projected_state, trunk_branch, node, mode)? {
            BranchEvaluation::Cleanable(candidate) => cleanable.push(candidate),
            BranchEvaluation::Blocked(blocked_branch) => blocked.push(blocked_branch),
        }
    }

    let cleanable_ids = cleanable
        .iter()
        .map(|candidate| candidate.node_id)
        .collect::<HashSet<_>>();

    let mut merged_candidates = cleanable
        .into_iter()
        .filter(|candidate| {
            !BranchGraph::new(&projected_state)
                .active_descendant_ids(candidate.node_id)
                .iter()
                .any(|descendant_id| cleanable_ids.contains(descendant_id))
        })
        .collect::<Vec<_>>();

    merged_candidates.sort_by(|left, right| {
        right
            .depth
            .cmp(&left.depth)
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });

    let mut candidates = deleted_candidates;
    candidates.extend(merged_candidates);

    Ok(CleanPlan {
        trunk_branch: trunk_branch.to_string(),
        current_branch: current_branch.to_string(),
        requested_branch_name: None,
        candidates,
        blocked,
    })
}

fn apply_deleted_step_projection(
    state: &DigState,
    deleted_steps: &[deleted_local::DeletedLocalBranchStep],
) -> io::Result<DigState> {
    let mut projected_state = state.clone();

    for step in deleted_steps {
        deleted_local::simulate_deleted_local_step(&mut projected_state, &step)?;
    }

    Ok(projected_state)
}

fn evaluate_integrated_branch(
    state: &DigState,
    trunk_branch: &str,
    node: &BranchNode,
    mode: CleanPlanMode,
) -> io::Result<BranchEvaluation> {
    if !git::branch_exists(&node.branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::BranchMissingLocally,
        }));
    }

    let graph = BranchGraph::new(state);

    let (local_parent_base, resolved_parent) =
        match deleted_local::resolve_replacement_parent(state, trunk_branch, &node.parent) {
            Ok(resolved) => resolved,
            Err(_) => {
                return Ok(BranchEvaluation::Blocked(BlockedBranch {
                    branch_name: node.branch_name.clone(),
                    reason: CleanBlockReason::ParentMissingFromDig,
                }));
            }
        };
    let parent_branch_name = local_parent_base.branch_name.clone();

    if !git::branch_exists(&parent_branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::ParentMissingLocally {
                parent_branch: parent_branch_name,
            },
        }));
    }

    let missing_descendants = graph.missing_local_descendants(node.id)?;
    if !missing_descendants.is_empty() {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::DescendantsMissingLocally {
                branch_names: missing_descendants,
            },
        }));
    }

    let tracked_pull_request_number = node.pull_request.as_ref().map(|pr| pr.number);
    let parent_base = if branch_is_integrated_for_pull_request(
        local_parent_base.rebase_ref(),
        &node.branch_name,
        tracked_pull_request_number,
    )? {
        local_parent_base
    } else if mode == CleanPlanMode::RemoteAwareSync {
        match resolve_remote_parent_base(&parent_branch_name)? {
            Some(remote_parent_base)
                if branch_is_integrated_for_pull_request(
                    remote_parent_base.rebase_ref(),
                    &node.branch_name,
                    tracked_pull_request_number,
                )? =>
            {
                remote_parent_base
            }
            _ => {
                return Ok(BranchEvaluation::Blocked(BlockedBranch {
                    branch_name: node.branch_name.clone(),
                    reason: CleanBlockReason::NotIntegrated {
                        parent_branch: parent_branch_name,
                    },
                }));
            }
        }
    } else {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::NotIntegrated {
                parent_branch: parent_branch_name,
            },
        }));
    };

    let restack_actions = restack::plan_after_branch_detach(
        state,
        node.id,
        &node.branch_name,
        &parent_base,
        &resolved_parent,
    )?;

    Ok(BranchEvaluation::Cleanable(CleanCandidate {
        node_id: node.id,
        branch_name: node.branch_name.clone(),
        parent_branch_name: parent_base.branch_name.clone(),
        reason: CleanReason::IntegratedIntoParent { parent_base },
        tree: graph.subtree(node.id)?,
        restack_plan: restack::previews_for_actions(&restack_actions),
        depth: graph.branch_depth(node.id),
    }))
}

pub(crate) fn cleanup_candidate_for_branch(
    state: &DigState,
    trunk_branch: &str,
    node: &BranchNode,
    mode: CleanPlanMode,
) -> io::Result<Option<CleanCandidate>> {
    match evaluate_integrated_branch(state, trunk_branch, node, mode)? {
        BranchEvaluation::Cleanable(candidate) => Ok(Some(candidate)),
        BranchEvaluation::Blocked(_) => Ok(None),
    }
}

fn clean_candidate_from_deleted_step(
    step: deleted_local::DeletedLocalBranchStep,
) -> CleanCandidate {
    CleanCandidate {
        node_id: step.node_id,
        branch_name: step.branch_name,
        parent_branch_name: step.new_parent_base.branch_name,
        reason: CleanReason::DeletedLocally,
        tree: step.tree,
        restack_plan: step.restack_plan,
        depth: step.depth,
    }
}

fn clean_candidate_from_pending_candidate(
    state: &DigState,
    trunk_branch: &str,
    candidate: &PendingCleanCandidate,
) -> io::Result<CleanCandidate> {
    match &candidate.kind {
        PendingCleanCandidateKind::DeletedLocally => {
            let step = deleted_local::plan_deleted_local_step_for_branch(
                state,
                trunk_branch,
                &candidate.branch_name,
            )?;

            Ok(clean_candidate_from_deleted_step(step))
        }
        PendingCleanCandidateKind::IntegratedIntoParent { parent_base } => {
            let node = state
                .find_branch_by_name(&candidate.branch_name)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("tracked branch '{}' was not found", candidate.branch_name),
                    )
                })?;
            let graph = BranchGraph::new(state);
            let restack_actions = restack::plan_after_branch_detach(
                state,
                node.id,
                &node.branch_name,
                parent_base,
                &node.parent,
            )?;

            Ok(CleanCandidate {
                node_id: node.id,
                branch_name: node.branch_name.clone(),
                parent_branch_name: parent_base.branch_name.clone(),
                reason: CleanReason::IntegratedIntoParent {
                    parent_base: parent_base.clone(),
                },
                tree: graph.subtree(node.id)?,
                restack_plan: restack::previews_for_actions(&restack_actions),
                depth: graph.branch_depth(node.id),
            })
        }
    }
}

pub(crate) fn branch_is_integrated(
    parent_branch_name: &str,
    branch_name: &str,
) -> io::Result<bool> {
    branch_is_integrated_for_pull_request(parent_branch_name, branch_name, None)
}

fn branch_is_integrated_for_pull_request(
    parent_branch_name: &str,
    branch_name: &str,
    tracked_pull_request_number: Option<u64>,
) -> io::Result<bool> {
    if branch_is_integrated_by_cherry(parent_branch_name, branch_name)? {
        return Ok(true);
    }

    branch_is_integrated_by_squash_message(
        parent_branch_name,
        branch_name,
        tracked_pull_request_number,
    )
}

fn resolve_remote_parent_base(parent_branch_name: &str) -> io::Result<Option<RestackBaseTarget>> {
    let Some(target) = git::branch_push_target(parent_branch_name)? else {
        return Ok(None);
    };

    if !git::remote_tracking_branch_exists(&target.remote_name, &target.branch_name)? {
        return Ok(None);
    }

    Ok(Some(RestackBaseTarget::with_rebase_ref(
        parent_branch_name,
        git::remote_tracking_branch_ref(&target.remote_name, &target.branch_name),
    )))
}

fn branch_is_integrated_by_cherry(parent_branch_name: &str, branch_name: &str) -> io::Result<bool> {
    let markers = git::cherry_markers(parent_branch_name, branch_name)?;

    Ok(markers
        .into_iter()
        .all(|marker| matches!(marker, CherryMarker::Equivalent)))
}

fn branch_is_integrated_by_squash_message(
    parent_branch_name: &str,
    branch_name: &str,
    tracked_pull_request_number: Option<u64>,
) -> io::Result<bool> {
    let merge_base = git::merge_base(parent_branch_name, branch_name)?;
    let branch_commits = git::commit_metadata_in_range(&format!("{merge_base}..{branch_name}"))?;

    if branch_commits.is_empty() {
        return Ok(true);
    }

    let parent_commits =
        git::commit_metadata_in_range(&format!("{merge_base}..{parent_branch_name}"))?;

    Ok(parent_commits.iter().any(|parent_commit| {
        parent_commit_mentions_all_branch_commits(parent_commit, &branch_commits)
            || tracked_pull_request_number.is_some_and(|number| {
                parent_commit_mentions_tracked_pull_request(parent_commit, number)
            })
    }))
}

pub(super) fn parent_commit_mentions_all_branch_commits(
    parent_commit: &CommitMetadata,
    branch_commits: &[CommitMetadata],
) -> bool {
    if branch_commits
        .iter()
        .all(|branch_commit| parent_commit.body.contains(&branch_commit.sha))
    {
        return true;
    }

    let message_lines = parent_commit
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<HashSet<_>>();

    branch_commits
        .iter()
        .all(|branch_commit| message_lines.contains(branch_commit.subject.as_str()))
}

pub(super) fn parent_commit_mentions_tracked_pull_request(
    parent_commit: &CommitMetadata,
    pull_request_number: u64,
) -> bool {
    let tracked_pull_request_suffix = format!("(#{pull_request_number})");
    let tracked_pull_request_ref = format!("pull request #{pull_request_number}");
    let tracked_pull_request_url = format!("/pull/{pull_request_number}");
    let subject = parent_commit.subject.to_ascii_lowercase();
    let body = parent_commit.body.to_ascii_lowercase();

    [subject.as_str(), body.as_str()].iter().any(|text| {
        text.contains(&tracked_pull_request_suffix)
            || text.contains(&tracked_pull_request_ref)
            || text.contains(&tracked_pull_request_url)
    })
}
