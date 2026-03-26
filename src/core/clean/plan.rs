use std::collections::HashSet;
use std::io;

use crate::core::git::{self, CherryMarker, CommitMetadata};
use crate::core::graph::BranchGraph;
use crate::core::restack;
use crate::core::store::types::DigState;
use crate::core::store::{BranchNode, dig_paths, load_config, load_state};
use crate::core::workflow;

use super::types::{
    BlockedBranch, CleanBlockReason, CleanCandidate, CleanOptions, CleanPlan, CleanReason,
};

#[derive(Debug)]
enum BranchEvaluation {
    Cleanable(CleanCandidate),
    Blocked(BlockedBranch),
}

pub(crate) fn plan(options: &CleanOptions) -> io::Result<CleanPlan> {
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
        Some(branch_name) => {
            plan_for_requested_branch(&state, &config.trunk_branch, &current_branch, branch_name)
        }
        None => plan_for_all_branches(&state, &config.trunk_branch, &current_branch),
    }
}

fn plan_for_requested_branch(
    state: &DigState,
    trunk_branch: &str,
    current_branch: &str,
    branch_name: &str,
) -> io::Result<CleanPlan> {
    let evaluation = match state.find_branch_by_name(branch_name) {
        Some(node) => evaluate_branch(state, trunk_branch, node)?,
        None => BranchEvaluation::Blocked(BlockedBranch {
            branch_name: branch_name.to_string(),
            reason: CleanBlockReason::BranchNotTracked,
        }),
    };

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
) -> io::Result<CleanPlan> {
    let mut cleanable = Vec::new();
    let mut blocked = Vec::new();

    for node in state.nodes.iter().filter(|node| !node.archived) {
        match evaluate_branch(state, trunk_branch, node)? {
            BranchEvaluation::Cleanable(candidate) => cleanable.push(candidate),
            BranchEvaluation::Blocked(blocked_branch) => blocked.push(blocked_branch),
        }
    }

    let cleanable_ids = cleanable
        .iter()
        .map(|candidate| candidate.node_id)
        .collect::<HashSet<_>>();

    let mut candidates = cleanable
        .into_iter()
        .filter(|candidate| {
            !BranchGraph::new(state)
                .active_descendant_ids(candidate.node_id)
                .iter()
                .any(|descendant_id| cleanable_ids.contains(descendant_id))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .depth
            .cmp(&left.depth)
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });

    Ok(CleanPlan {
        trunk_branch: trunk_branch.to_string(),
        current_branch: current_branch.to_string(),
        requested_branch_name: None,
        candidates,
        blocked,
    })
}

fn evaluate_branch(
    state: &DigState,
    trunk_branch: &str,
    node: &BranchNode,
) -> io::Result<BranchEvaluation> {
    if !git::branch_exists(&node.branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::BranchMissingLocally,
        }));
    }

    let graph = BranchGraph::new(state);

    let Some(parent_branch_name) = graph.parent_branch_name(node, trunk_branch) else {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::ParentMissingFromDig,
        }));
    };

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

    if !branch_is_integrated(&parent_branch_name, &node.branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::NotIntegrated {
                parent_branch: parent_branch_name,
            },
        }));
    }

    let restack_actions = restack::plan_after_branch_detach(
        state,
        node.id,
        &node.branch_name,
        &parent_branch_name,
        &node.parent,
    )?;

    Ok(BranchEvaluation::Cleanable(CleanCandidate {
        node_id: node.id,
        branch_name: node.branch_name.clone(),
        parent_branch_name: parent_branch_name.clone(),
        reason: CleanReason::IntegratedIntoParent {
            parent_branch: parent_branch_name,
        },
        tree: graph.subtree(node.id)?,
        restack_plan: restack::previews_for_actions(&restack_actions),
        depth: graph.branch_depth(node.id),
    }))
}

pub(crate) fn branch_is_integrated(
    parent_branch_name: &str,
    branch_name: &str,
) -> io::Result<bool> {
    if branch_is_integrated_by_cherry(parent_branch_name, branch_name)? {
        return Ok(true);
    }

    branch_is_integrated_by_squash_message(parent_branch_name, branch_name)
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
