mod apply;
mod plan;
mod types;

pub(crate) use apply::{apply, apply_with_reporter, resume_after_sync};
pub(crate) use plan::{branch_is_integrated, plan};
pub(crate) use types::{
    BlockedBranch, CleanApplyOutcome, CleanBlockReason, CleanCandidate, CleanEvent, CleanOptions,
    CleanPlan, CleanReason, CleanTreeNode,
};

#[cfg(test)]
mod tests;
