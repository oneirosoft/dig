use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::restack::RestackPreview;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrphanOptions {
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanPlan {
    pub trunk_branch: String,
    pub original_branch: String,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub node_id: Uuid,
    pub restack_plan: Vec<RestackPreview>,
}

#[derive(Debug)]
pub struct OrphanOutcome {
    pub status: ExitStatus,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub restacked_branches: Vec<RestackPreview>,
    pub restored_original_branch: Option<String>,
    pub failure_output: Option<String>,
    pub paused: bool,
}
