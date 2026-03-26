use std::io;

use uuid::Uuid;

use crate::core::git;
use crate::core::store::types::DigState;
use crate::core::store::{BranchNode, ParentRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchTreeNode {
    pub branch_name: String,
    pub children: Vec<BranchTreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchLineageNode {
    pub branch_name: String,
    pub pull_request_number: Option<u64>,
}

pub struct BranchGraph<'a> {
    state: &'a DigState,
}

impl<'a> BranchGraph<'a> {
    pub fn new(state: &'a DigState) -> Self {
        Self { state }
    }

    pub fn lineage(&self, branch_name: &str, trunk_branch: &str) -> Vec<BranchLineageNode> {
        let Some(mut current_node) = self.state.find_branch_by_name(branch_name) else {
            return vec![BranchLineageNode {
                branch_name: branch_name.to_string(),
                pull_request_number: None,
            }];
        };

        let mut lineage = vec![BranchLineageNode {
            branch_name: current_node.branch_name.clone(),
            pull_request_number: current_node.pull_request.as_ref().map(|pr| pr.number),
        }];

        loop {
            match &current_node.parent {
                ParentRef::Trunk => {
                    if current_node.branch_name != trunk_branch {
                        lineage.push(BranchLineageNode {
                            branch_name: trunk_branch.to_string(),
                            pull_request_number: None,
                        });
                    }
                    break;
                }
                ParentRef::Branch { node_id } => {
                    let Some(parent_node) = self.state.find_branch_by_id(*node_id) else {
                        break;
                    };

                    lineage.push(BranchLineageNode {
                        branch_name: parent_node.branch_name.clone(),
                        pull_request_number: parent_node.pull_request.as_ref().map(|pr| pr.number),
                    });
                    current_node = parent_node;
                }
            }
        }

        lineage
    }

    pub fn active_children_ids(&self, node_id: Uuid) -> Vec<Uuid> {
        self.state
            .nodes
            .iter()
            .filter(|node| !node.archived)
            .filter_map(|node| match node.parent {
                ParentRef::Branch { node_id: parent_id } if parent_id == node_id => Some(node.id),
                _ => None,
            })
            .collect()
    }

    pub fn active_descendant_ids(&self, node_id: Uuid) -> Vec<Uuid> {
        let mut descendants = Vec::new();
        let mut frontier = self.active_children_ids(node_id);

        while let Some(child_id) = frontier.pop() {
            descendants.push(child_id);
            frontier.extend(self.active_children_ids(child_id));
        }

        descendants
    }

    pub fn branch_depth(&self, node_id: Uuid) -> usize {
        let Some(mut current_node) = self.state.find_branch_by_id(node_id) else {
            return 0;
        };

        let mut depth = 0;

        loop {
            match current_node.parent {
                ParentRef::Trunk => return depth,
                ParentRef::Branch { node_id: parent_id } => {
                    let Some(parent_node) = self.state.find_branch_by_id(parent_id) else {
                        return depth;
                    };

                    depth += 1;
                    current_node = parent_node;
                }
            }
        }
    }

    pub fn parent_branch_name(&self, node: &BranchNode, trunk_branch: &str) -> Option<String> {
        match node.parent {
            ParentRef::Trunk => Some(trunk_branch.to_string()),
            ParentRef::Branch { node_id } => self
                .state
                .find_branch_by_id(node_id)
                .map(|parent_node| parent_node.branch_name.clone()),
        }
    }

    pub fn subtree(&self, node_id: Uuid) -> io::Result<BranchTreeNode> {
        let node = self.state.find_branch_by_id(node_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found")
        })?;

        let mut children = Vec::new();
        for child_id in self.active_children_ids(node_id) {
            children.push(self.subtree(child_id)?);
        }

        Ok(BranchTreeNode {
            branch_name: node.branch_name.clone(),
            children,
        })
    }

    pub fn missing_local_descendants(&self, node_id: Uuid) -> io::Result<Vec<String>> {
        let mut missing = Vec::new();

        for descendant_id in self.active_descendant_ids(node_id) {
            let Some(descendant) = self.state.find_branch_by_id(descendant_id) else {
                continue;
            };

            if !git::branch_exists(&descendant.branch_name)? {
                missing.push(descendant.branch_name.clone());
            }
        }

        Ok(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchGraph, BranchLineageNode, BranchTreeNode};
    use crate::core::store::types::{DIG_STATE_VERSION, DigState};
    use crate::core::store::{BranchDivergenceState, BranchNode, ParentRef, TrackedPullRequest};
    use uuid::Uuid;

    fn fixture_state() -> (DigState, Uuid, Uuid, Uuid) {
        let parent_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let grandchild_id = Uuid::new_v4();

        (
            DigState {
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
                        divergence_state: BranchDivergenceState::Unknown,
                        pull_request: Some(TrackedPullRequest { number: 42 }),
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
                        divergence_state: BranchDivergenceState::Unknown,
                        pull_request: None,
                        archived: false,
                    },
                    BranchNode {
                        id: grandchild_id,
                        branch_name: "feature/api-tests".into(),
                        parent: ParentRef::Branch { node_id: child_id },
                        base_ref: "feature/api-followup".into(),
                        fork_point_oid: "fedcba".into(),
                        head_oid_at_creation: "fedcba".into(),
                        created_at_unix_secs: 3,
                        divergence_state: BranchDivergenceState::Unknown,
                        pull_request: None,
                        archived: false,
                    },
                ],
            },
            parent_id,
            child_id,
            grandchild_id,
        )
    }

    #[test]
    fn builds_branch_lineage_from_child_to_trunk() {
        let (state, _, _, _) = fixture_state();
        let graph = BranchGraph::new(&state);

        assert_eq!(
            graph.lineage("feature/api-followup", "main"),
            vec![
                BranchLineageNode {
                    branch_name: "feature/api-followup".to_string(),
                    pull_request_number: None,
                },
                BranchLineageNode {
                    branch_name: "feature/api".to_string(),
                    pull_request_number: Some(42),
                },
                BranchLineageNode {
                    branch_name: "main".to_string(),
                    pull_request_number: None,
                }
            ]
        );
    }

    #[test]
    fn tracks_active_descendants_and_depth() {
        let (state, parent_id, child_id, grandchild_id) = fixture_state();
        let graph = BranchGraph::new(&state);

        assert_eq!(graph.active_children_ids(parent_id), vec![child_id]);
        assert_eq!(
            graph.active_descendant_ids(parent_id),
            vec![child_id, grandchild_id]
        );
        assert_eq!(graph.branch_depth(grandchild_id), 2);
    }

    #[test]
    fn builds_branch_subtree() {
        let (state, parent_id, _, _) = fixture_state();
        let graph = BranchGraph::new(&state);

        assert_eq!(
            graph.subtree(parent_id).unwrap(),
            BranchTreeNode {
                branch_name: "feature/api".into(),
                children: vec![BranchTreeNode {
                    branch_name: "feature/api-followup".into(),
                    children: vec![BranchTreeNode {
                        branch_name: "feature/api-tests".into(),
                        children: vec![],
                    }],
                }],
            }
        );
    }
}
