mod apply;
mod plan;
mod types;

pub(crate) use apply::{apply, apply_with_reporter, resume_after_sync};
pub(crate) use plan::{
    CleanPlanMode, branch_is_integrated, cleanup_candidate_for_branch, mode_for_sync, plan,
    plan_for_sync,
};
pub(crate) use types::{
    BlockedBranch, CleanApplyOutcome, CleanBlockReason, CleanCandidate, CleanEvent, CleanOptions,
    CleanPlan, CleanReason, CleanTreeNode,
};

#[cfg(test)]
mod tests;
