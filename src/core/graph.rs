use std::collections::HashSet;
use std::io;

use uuid::Uuid;

use crate::core::git;
use crate::core::store::types::DaggerState;
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
    state: &'a DaggerState,
}

impl<'a> BranchGraph<'a> {
    pub fn new(state: &'a DaggerState) -> Self {
        Self { state }
    }

    pub fn lineage(&self, branch_name: &str, trunk_branch: &str) -> Vec<BranchLineageNode> {
        let Some(mut current_node) = self.state.find_branch_by_name(branch_name) else {
            return vec![BranchLineageNode {
                branch_name: branch_name.to_string(),
                pull_request_number: None,
            }];
        };

        let mut visited = HashSet::new();
        visited.insert(current_node.id);
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
                    if !visited.insert(*node_id) {
                        break; // cycle detected
                    }
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
        let mut visited = HashSet::new();
        visited.insert(node_id);
        let mut frontier = self.active_children_ids(node_id);

        while let Some(child_id) = frontier.pop() {
            if !visited.insert(child_id) {
                continue; // cycle detected
            }
            descendants.push(child_id);
            frontier.extend(self.active_children_ids(child_id));
        }

        descendants
    }

    pub fn branch_depth(&self, node_id: Uuid) -> usize {
        let Some(mut current_node) = self.state.find_branch_by_id(node_id) else {
            return 0;
        };

        let mut visited = HashSet::new();
        visited.insert(node_id);
        let mut depth = 0;

        loop {
            match current_node.parent {
                ParentRef::Trunk => return depth,
                ParentRef::Branch { node_id: parent_id } => {
                    if !visited.insert(parent_id) {
                        return depth; // cycle detected
                    }
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
        let mut visited = HashSet::new();
        self.subtree_inner(node_id, &mut visited)
    }

    fn subtree_inner(
        &self,
        node_id: Uuid,
        visited: &mut HashSet<Uuid>,
    ) -> io::Result<BranchTreeNode> {
        if !visited.insert(node_id) {
            let branch_info = self
                .state
                .find_branch_by_id(node_id)
                .map(|n| format!(" (branch: {})", n.branch_name))
                .unwrap_or_default();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cycle detected in branch tree at node {}{}", node_id, branch_info),
            ));
        }

        let node = self.state.find_branch_by_id(node_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found")
        })?;

        let mut children = Vec::new();
        for child_id in self.active_children_ids(node_id) {
            children.push(self.subtree_inner(child_id, visited)?);
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
    use crate::core::store::types::{DAGGER_STATE_VERSION, DaggerState};
    use crate::core::store::{BranchDivergenceState, BranchNode, ParentRef, TrackedPullRequest};
    use uuid::Uuid;

    fn fixture_state() -> (DaggerState, Uuid, Uuid, Uuid) {
        let parent_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let grandchild_id = Uuid::new_v4();

        (
            DaggerState {
                version: DAGGER_STATE_VERSION,
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

    /// Helper to build a cyclic state where A's parent is B and B's parent is A.
    fn fixture_cycle_state() -> (DaggerState, Uuid, Uuid) {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        (
            DaggerState {
                version: DAGGER_STATE_VERSION,
                nodes: vec![
                    BranchNode {
                        id: id_a,
                        branch_name: "branch-a".into(),
                        parent: ParentRef::Branch { node_id: id_b },
                        base_ref: "branch-b".into(),
                        fork_point_oid: "aaa".into(),
                        head_oid_at_creation: "aaa".into(),
                        created_at_unix_secs: 1,
                        divergence_state: BranchDivergenceState::Unknown,
                        pull_request: None,
                        archived: false,
                    },
                    BranchNode {
                        id: id_b,
                        branch_name: "branch-b".into(),
                        parent: ParentRef::Branch { node_id: id_a },
                        base_ref: "branch-a".into(),
                        fork_point_oid: "bbb".into(),
                        head_oid_at_creation: "bbb".into(),
                        created_at_unix_secs: 2,
                        divergence_state: BranchDivergenceState::Unknown,
                        pull_request: None,
                        archived: false,
                    },
                ],
            },
            id_a,
            id_b,
        )
    }

    #[test]
    fn lineage_terminates_on_cycle() {
        let (state, _, _) = fixture_cycle_state();
        let graph = BranchGraph::new(&state);

        let result = graph.lineage("branch-a", "main");

        // Should contain branch-a and branch-b exactly once, cycle broken
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].branch_name, "branch-a");
        assert_eq!(result[1].branch_name, "branch-b");
    }

    #[test]
    fn branch_depth_terminates_on_cycle() {
        let (state, id_a, _) = fixture_cycle_state();
        let graph = BranchGraph::new(&state);

        let depth = graph.branch_depth(id_a);
        // A -> B -> cycle detected, depth should be 1
        assert_eq!(depth, 1);
    }

    #[test]
    fn active_descendant_ids_terminates_on_cycle() {
        // Create a state where a node appears as its own descendant via a
        // circular child reference. We simulate this by making node C point
        // to node A as parent while A points to C as parent, forming a cycle
        // in the child-walk direction.
        let id_a = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        let state = DaggerState {
            version: DAGGER_STATE_VERSION,
            nodes: vec![
                BranchNode {
                    id: id_a,
                    branch_name: "cycle-a".into(),
                    parent: ParentRef::Branch { node_id: id_c },
                    base_ref: "cycle-c".into(),
                    fork_point_oid: "aaa".into(),
                    head_oid_at_creation: "aaa".into(),
                    created_at_unix_secs: 1,
                    divergence_state: BranchDivergenceState::Unknown,
                    pull_request: None,
                    archived: false,
                },
                BranchNode {
                    id: id_c,
                    branch_name: "cycle-c".into(),
                    parent: ParentRef::Branch { node_id: id_a },
                    base_ref: "cycle-a".into(),
                    fork_point_oid: "ccc".into(),
                    head_oid_at_creation: "ccc".into(),
                    created_at_unix_secs: 2,
                    divergence_state: BranchDivergenceState::Unknown,
                    pull_request: None,
                    archived: false,
                },
            ],
        };

        let graph = BranchGraph::new(&state);

        // A is a child of C and C is a child of A — this creates a cycle in
        // descendant traversal. It must terminate.
        let descendants = graph.active_descendant_ids(id_a);
        // Should contain id_c exactly once (it's a "child" of id_a since id_c's parent is id_a)
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0], id_c);
    }

    #[test]
    fn subtree_returns_error_on_cycle() {
        let (state, id_a, _) = fixture_cycle_state();
        let graph = BranchGraph::new(&state);

        let result = graph.subtree(id_a);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("cycle detected"));
    }
}
