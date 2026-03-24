use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DIG_STATE_VERSION: u32 = 1;
pub const DIG_CONFIG_VERSION: u32 = 1;

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

    pub fn branch_lineage(&self, branch_name: &str, trunk_branch: &str) -> Vec<String> {
        let Some(mut current_node) = self.find_branch_by_name(branch_name) else {
            return vec![branch_name.to_string()];
        };

        let mut lineage = vec![current_node.branch_name.clone()];

        loop {
            match &current_node.parent {
                ParentRef::Trunk => {
                    if current_node.branch_name != trunk_branch {
                        lineage.push(trunk_branch.to_string());
                    }
                    break;
                }
                ParentRef::Branch { node_id } => {
                    let Some(parent_node) = self.nodes.iter().find(|node| node.id == *node_id) else {
                        break;
                    };

                    lineage.push(parent_node.branch_name.clone());
                    current_node = parent_node;
                }
            }
        }

        lineage
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCreatedEvent {
    pub occurred_at_unix_secs: u64,
    pub node: BranchNode,
}

pub fn now_unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{BranchNode, DigConfig, DigState, ParentRef, DIG_STATE_VERSION};
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
        assert_eq!(
            DigConfig::new("main".into()).trunk_branch,
            "main"
        );
    }

    #[test]
    fn builds_branch_lineage_from_child_to_trunk() {
        let parent_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let state = DigState {
            version: DIG_STATE_VERSION,
            nodes: vec![
                BranchNode {
                    id: parent_id,
                    branch_name: "feature/api".into(),
                    parent: ParentRef::Trunk,
                    base_ref: "main".into(),
                    fork_point_oid: "abc123".into(),
                    head_oid_at_creation: "abc123".into(),
                    created_at_unix_secs: 1,
                    archived: false,
                },
                BranchNode {
                    id: child_id,
                    branch_name: "feature/api-followup".into(),
                    parent: ParentRef::Branch { node_id: parent_id },
                    base_ref: "feature/api".into(),
                    fork_point_oid: "def456".into(),
                    head_oid_at_creation: "def456".into(),
                    created_at_unix_secs: 2,
                    archived: false,
                },
            ],
        };

        assert_eq!(
            state.branch_lineage("feature/api-followup", "main"),
            vec![
                "feature/api-followup".to_string(),
                "feature/api".to_string(),
                "main".to_string()
            ]
        );
    }
}
