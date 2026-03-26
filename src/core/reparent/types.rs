use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::restack::RestackPreview;
use crate::core::store::ParentRef;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReparentOptions {
    pub branch_name: Option<String>,
    pub parent_branch_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReparentPlan {
    pub original_branch: String,
    pub branch_name: String,
    pub current_parent_branch_name: String,
    pub parent_branch_name: String,
    pub node_id: Uuid,
    pub current_parent: ParentRef,
    pub new_parent: ParentRef,
    pub restack_plan: Vec<RestackPreview>,
}

#[derive(Debug)]
pub(crate) struct ReparentOutcome {
    pub status: ExitStatus,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub restacked_branches: Vec<RestackPreview>,
    pub restored_original_branch: Option<String>,
    pub failure_output: Option<String>,
    pub paused: bool,
}
