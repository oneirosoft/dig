use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::restack::{RestackAction, RestackPreview};

pub const DIG_STATE_VERSION: u32 = 1;
pub const DIG_CONFIG_VERSION: u32 = 1;
pub const DIG_OPERATION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigConfig {
    pub version: u32,
    pub trunk_branch: String,
}

impl DigConfig {
    pub fn new(trunk_branch: String) -> Self {
        Self {
            version: DIG_CONFIG_VERSION,
            trunk_branch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigState {
    pub version: u32,
    pub nodes: Vec<BranchNode>,
}

impl Default for DigState {
    fn default() -> Self {
        Self {
            version: DIG_STATE_VERSION,
            nodes: Vec::new(),
        }
    }
}

impl DigState {
    pub fn find_branch_by_name(&self, branch_name: &str) -> Option<&BranchNode> {
        self.nodes
            .iter()
            .find(|node| !node.archived && node.branch_name == branch_name)
    }

    pub fn find_branch_by_id(&self, node_id: Uuid) -> Option<&BranchNode> {
        self.nodes
            .iter()
            .find(|node| !node.archived && node.id == node_id)
    }

    pub fn find_branch_by_id_mut(&mut self, node_id: Uuid) -> Option<&mut BranchNode> {
        self.nodes
            .iter_mut()
            .find(|node| !node.archived && node.id == node_id)
    }

    pub fn insert_branch(&mut self, node: BranchNode) -> io::Result<()> {
        if self.find_branch_by_name(&node.branch_name).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("branch '{}' is already tracked by dig", node.branch_name),
            ));
        }

        self.nodes.push(node);

        Ok(())
    }

    pub fn archive_branch(&mut self, node_id: Uuid) -> io::Result<()> {
        let node = self.find_branch_by_id_mut(node_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found")
        })?;

        node.archived = true;

        Ok(())
    }

    pub fn reparent_branch(
        &mut self,
        node_id: Uuid,
        new_parent: ParentRef,
        new_base_ref: String,
    ) -> io::Result<(ParentRef, String)> {
        let node = self.find_branch_by_id_mut(node_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found")
        })?;

        let old_parent = node.parent.clone();
        let old_base_ref = node.base_ref.clone();
        node.parent = new_parent;
        node.base_ref = new_base_ref;

        Ok((old_parent, old_base_ref))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOperationState {
    pub version: u32,
    pub origin: PendingOperationKind,
    pub restack: PendingRestackState,
}

impl PendingOperationState {
    pub fn start(origin: PendingOperationKind, actions: Vec<RestackAction>) -> Option<Self> {
        let mut actions = actions.into_iter();
        let active_action = actions.next()?;

        Some(Self {
            version: DIG_OPERATION_VERSION,
            origin,
            restack: PendingRestackState {
                active_action,
                remaining_actions: actions.collect(),
                completed_branches: Vec::new(),
            },
        })
    }

    pub fn active_action(&self) -> &RestackAction {
        &self.restack.active_action
    }

    pub fn completed_branches(&self) -> &[RestackPreview] {
        &self.restack.completed_branches
    }

    pub fn advance_after_success(mut self) -> (RestackPreview, Option<Self>) {
        let preview = RestackPreview {
            branch_name: self.restack.active_action.branch_name.clone(),
            onto_branch: self.restack.active_action.new_base_branch_name.clone(),
            parent_changed: self.restack.active_action.new_parent.is_some(),
        };
        self.restack.completed_branches.push(preview.clone());

        if self.restack.remaining_actions.is_empty() {
            return (preview, None);
        }

        self.restack.active_action = self.restack.remaining_actions.remove(0);

        (preview, Some(self))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRestackState {
    pub active_action: RestackAction,
    pub remaining_actions: Vec<RestackAction>,
    pub completed_branches: Vec<RestackPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PendingOperationKind {
    Commit(PendingCommitOperation),
    Adopt(PendingAdoptOperation),
    Merge(PendingMergeOperation),
    Clean(PendingCleanOperation),
    Orphan(PendingOrphanOperation),
}

impl PendingOperationKind {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Commit(_) => "commit",
            Self::Adopt(_) => "adopt",
            Self::Merge(_) => "merge",
            Self::Clean(_) => "clean",
            Self::Orphan(_) => "orphan",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCommitOperation {
    pub current_branch: String,
    pub summary_line: Option<String>,
    pub recent_commits: Vec<PendingCommitEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAdoptOperation {
    pub original_branch: String,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub parent: ParentRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMergeOperation {
    pub trunk_branch: String,
    pub source_branch_name: String,
    pub target_branch_name: String,
    pub source_node_id: Uuid,
    pub switched_to_target_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCleanOperation {
    pub trunk_branch: String,
    pub original_branch: String,
    pub switched_to_trunk_from: Option<String>,
    pub current_candidate_branch_name: String,
    pub remaining_branch_names: Vec<String>,
    pub deleted_branches: Vec<String>,
    pub restacked_branches: Vec<RestackPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOrphanOperation {
    pub original_branch: String,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub node_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCommitEntry {
    pub hash: String,
    pub refs: Vec<String>,
    pub is_head: bool,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchNode {
    pub id: Uuid,
    pub branch_name: String,
    pub parent: ParentRef,
    pub base_ref: String,
    pub fork_point_oid: String,
    pub head_oid_at_creation: String,
    pub created_at_unix_secs: u64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParentRef {
    Trunk,
    Branch { node_id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DigEvent {
    BranchCreated(BranchCreatedEvent),
    BranchAdopted(BranchAdoptedEvent),
    BranchArchived(BranchArchivedEvent),
    BranchReparented(BranchReparentedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCreatedEvent {
    pub occurred_at_unix_secs: u64,
    pub node: BranchNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchAdoptedEvent {
    pub occurred_at_unix_secs: u64,
    pub node: BranchNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BranchArchiveReason {
    IntegratedIntoParent { parent_branch: String },
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchArchivedEvent {
    pub occurred_at_unix_secs: u64,
    pub branch_id: Uuid,
    pub branch_name: String,
    pub reason: BranchArchiveReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchReparentedEvent {
    pub occurred_at_unix_secs: u64,
    pub branch_id: Uuid,
    pub branch_name: String,
    pub old_parent: ParentRef,
    pub new_parent: ParentRef,
    pub old_base_ref: String,
    pub new_base_ref: String,
}

pub fn now_unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        BranchAdoptedEvent, BranchArchiveReason, BranchArchivedEvent, BranchNode, DigConfig,
        DigEvent, DigState, ParentRef, PendingCommitOperation, PendingOperationKind,
        PendingOperationState, PendingOrphanOperation,
    };
    use crate::core::restack::RestackAction;
    use uuid::Uuid;

    #[test]
    fn tracks_roots_with_union_parent_ref() {
        let node = BranchNode {
            id: Uuid::nil(),
            branch_name: "feature/api".into(),
            parent: ParentRef::Trunk,
            base_ref: "main".into(),
            fork_point_oid: "abc123".into(),
            head_oid_at_creation: "abc123".into(),
            created_at_unix_secs: 1,
            archived: false,
        };

        let mut state = DigState::default();
        state.insert_branch(node.clone()).unwrap();

        assert_eq!(state.find_branch_by_name("feature/api"), Some(&node));
    }

    #[test]
    fn builds_config_with_trunk_branch() {
        assert_eq!(DigConfig::new("main".into()).trunk_branch, "main");
    }

    #[test]
    fn serializes_branch_archive_event_with_union_reason() {
        let event = DigEvent::BranchArchived(BranchArchivedEvent {
            occurred_at_unix_secs: 1,
            branch_id: Uuid::nil(),
            branch_name: "feature/api".into(),
            reason: BranchArchiveReason::IntegratedIntoParent {
                parent_branch: "main".into(),
            },
        });

        let serialized = serde_json::to_string(&event).unwrap();

        assert!(serialized.contains("\"type\":\"branch_archived\""));
        assert!(serialized.contains("\"kind\":\"integrated_into_parent\""));
    }

    #[test]
    fn serializes_orphaned_branch_archive_event() {
        let event = DigEvent::BranchArchived(BranchArchivedEvent {
            occurred_at_unix_secs: 1,
            branch_id: Uuid::nil(),
            branch_name: "feature/api".into(),
            reason: BranchArchiveReason::Orphaned,
        });

        let serialized = serde_json::to_string(&event).unwrap();

        assert!(serialized.contains("\"type\":\"branch_archived\""));
        assert!(serialized.contains("\"kind\":\"orphaned\""));
    }

    #[test]
    fn serializes_branch_adopted_event() {
        let event = DigEvent::BranchAdopted(BranchAdoptedEvent {
            occurred_at_unix_secs: 1,
            node: BranchNode {
                id: Uuid::nil(),
                branch_name: "feature/api".into(),
                parent: ParentRef::Trunk,
                base_ref: "main".into(),
                fork_point_oid: "abc123".into(),
                head_oid_at_creation: "def456".into(),
                created_at_unix_secs: 1,
                archived: false,
            },
        });

        let serialized = serde_json::to_string(&event).unwrap();

        assert!(serialized.contains("\"type\":\"branch_adopted\""));
        assert!(serialized.contains("\"branch_name\":\"feature/api\""));
    }

    #[test]
    fn advances_pending_operation_queue_after_success() {
        let first_action = RestackAction {
            node_id: Uuid::new_v4(),
            branch_name: "feature/api-followup".into(),
            old_upstream_branch_name: "feature/api".into(),
            old_upstream_oid: "abc123".into(),
            new_base_branch_name: "feature/api".into(),
            new_parent: None,
        };
        let second_action = RestackAction {
            node_id: Uuid::new_v4(),
            branch_name: "feature/api-tests".into(),
            old_upstream_branch_name: "feature/api-followup".into(),
            old_upstream_oid: "def456".into(),
            new_base_branch_name: "feature/api-followup".into(),
            new_parent: Some(ParentRef::Trunk),
        };
        let operation = PendingOperationState::start(
            PendingOperationKind::Commit(PendingCommitOperation {
                current_branch: "feature/api".into(),
                summary_line: Some("1 file changed".into()),
                recent_commits: Vec::new(),
            }),
            vec![first_action.clone(), second_action.clone()],
        )
        .unwrap();

        assert_eq!(operation.active_action(), &first_action);

        let (first_preview, operation) = operation.advance_after_success();
        let operation = operation.unwrap();

        assert_eq!(first_preview.branch_name, "feature/api-followup");
        assert_eq!(operation.active_action(), &second_action);
        assert_eq!(operation.completed_branches(), &[first_preview]);

        let (_, operation) = operation.advance_after_success();

        assert!(operation.is_none());
    }

    #[test]
    fn reports_orphan_operation_command_name() {
        let operation = PendingOperationKind::Orphan(PendingOrphanOperation {
            original_branch: "feature/api".into(),
            branch_name: "feature/api".into(),
            parent_branch_name: "main".into(),
            node_id: Uuid::nil(),
        });

        assert_eq!(operation.command_name(), "orphan");
    }
}
