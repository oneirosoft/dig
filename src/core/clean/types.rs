use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::graph::BranchTreeNode;
use crate::core::restack::RestackPreview;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CleanOptions {
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanPlan {
    pub trunk_branch: String,
    pub current_branch: String,
    pub requested_branch_name: Option<String>,
    pub candidates: Vec<CleanCandidate>,
    pub blocked: Vec<BlockedBranch>,
}

impl CleanPlan {
    pub(crate) fn targets_current_branch(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| candidate.branch_name == self.current_branch)
    }

    pub(crate) fn deleted_local_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.is_deleted_locally())
            .count()
    }

    pub(crate) fn merged_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.is_integrated())
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanCandidate {
    pub node_id: Uuid,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub reason: CleanReason,
    pub tree: CleanTreeNode,
    pub restack_plan: Vec<RestackPreview>,
    pub(crate) depth: usize,
}

impl CleanCandidate {
    pub(crate) fn is_deleted_locally(&self) -> bool {
        matches!(self.reason, CleanReason::DeletedLocally)
    }

    pub(crate) fn is_integrated(&self) -> bool {
        matches!(self.reason, CleanReason::IntegratedIntoParent { .. })
    }
}

pub(crate) type CleanTreeNode = BranchTreeNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanReason {
    DeletedLocally,
    IntegratedIntoParent { parent_branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockedBranch {
    pub branch_name: String,
    pub reason: CleanBlockReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanBlockReason {
    BranchNotTracked,
    BranchMissingLocally,
    ParentMissingLocally { parent_branch: String },
    ParentMissingFromDig,
    NotIntegrated { parent_branch: String },
    DescendantsMissingLocally { branch_names: Vec<String> },
}

#[derive(Debug)]
pub(crate) struct CleanApplyOutcome {
    pub status: ExitStatus,
    pub switched_to_trunk_from: Option<String>,
    pub restored_original_branch: Option<String>,
    pub untracked_branches: Vec<String>,
    pub deleted_branches: Vec<String>,
    pub restacked_branches: Vec<RestackPreview>,
    pub failure_output: Option<String>,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanEvent {
    SwitchingToTrunk {
        from_branch: String,
        to_branch: String,
    },
    SwitchedToTrunk {
        from_branch: String,
        to_branch: String,
    },
    RebaseStarted {
        branch_name: String,
        onto_branch: String,
    },
    RebaseProgress {
        branch_name: String,
        onto_branch: String,
        current_commit: usize,
        total_commits: usize,
    },
    RebaseCompleted {
        branch_name: String,
        onto_branch: String,
    },
    DeleteStarted {
        branch_name: String,
    },
    DeleteCompleted {
        branch_name: String,
    },
    ArchiveStarted {
        branch_name: String,
    },
    ArchiveCompleted {
        branch_name: String,
    },
}
