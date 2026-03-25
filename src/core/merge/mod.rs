mod apply;
mod plan;
mod types;

pub(crate) use apply::{
    apply, apply_with_reporter, delete_merged_branch, delete_merged_branch_by_id, resume_after_sync,
};
pub(crate) use plan::plan;
pub(crate) use types::{
    MergeEvent, MergeMode, MergeOptions, MergeOutcome, MergePlan, MergeResumeOutcome, MergeTreeNode,
};

#[cfg(test)]
mod tests;
