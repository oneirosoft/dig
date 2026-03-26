use std::collections::{HashMap, HashSet};
use std::io;
use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::git;
use crate::core::store::types::DigState;
use crate::core::store::{BranchNode, ParentRef, open_initialized};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeOptions {
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeLabel {
    pub branch_name: String,
    pub is_current: bool,
    pub pull_request_number: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub branch_name: String,
    pub is_current: bool,
    pub pull_request_number: Option<u64>,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeView {
    pub root_label: Option<TreeLabel>,
    pub roots: Vec<TreeNode>,
}

#[derive(Debug)]
pub struct TreeOutcome {
    pub status: ExitStatus,
    pub view: TreeView,
}

pub fn run(options: &TreeOptions) -> io::Result<TreeOutcome> {
    let status = git::probe_repo_status()?;
    let session = open_initialized("dig is not initialized")?;
    let current_branch = git::current_branch_name_if_any()?;
    let full_view = build_tree_view(
        &session.state,
        &session.config.trunk_branch,
        current_branch.as_deref(),
    );
    let view = filter_tree_view(full_view, options.branch_name.as_deref())?;

    Ok(TreeOutcome { status, view })
}

pub(crate) fn focused_context_view(branch_name: &str) -> io::Result<TreeView> {
    let session = open_initialized("dig is not initialized")?;
    let full_view = build_tree_view(&session.state, &session.config.trunk_branch, None);

    focus_tree_view(full_view, branch_name)
}

fn build_tree_view(state: &DigState, trunk_branch: &str, current_branch: Option<&str>) -> TreeView {
    let active_nodes = state
        .nodes
        .iter()
        .filter(|node| !node.archived)
        .collect::<Vec<_>>();
    let order_lookup = active_nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect::<HashMap<_, _>>();

    let known_ids = active_nodes
        .iter()
        .map(|node| node.id)
        .collect::<HashSet<_>>();
    let mut child_lookup = HashMap::<Uuid, Vec<&BranchNode>>::new();
    let mut root_nodes = Vec::<&BranchNode>::new();

    for node in &active_nodes {
        match node.parent {
            ParentRef::Trunk => root_nodes.push(node),
            ParentRef::Branch { node_id } if known_ids.contains(&node_id) => {
                child_lookup.entry(node_id).or_default().push(node);
            }
            ParentRef::Branch { .. } => root_nodes.push(node),
        }
    }

    sort_branch_nodes(&mut root_nodes, &order_lookup);
    for children in child_lookup.values_mut() {
        sort_branch_nodes(children, &order_lookup);
    }

    TreeView {
        root_label: Some(TreeLabel {
            branch_name: trunk_branch.to_string(),
            is_current: current_branch == Some(trunk_branch),
            pull_request_number: None,
        }),
        roots: root_nodes
            .into_iter()
            .map(|node| build_tree_node(node, current_branch, &child_lookup))
            .collect(),
    }
}

fn filter_tree_view(view: TreeView, requested_branch: Option<&str>) -> io::Result<TreeView> {
    let Some(requested_branch) = requested_branch
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    else {
        return Ok(view);
    };

    let Some(root_label) = &view.root_label else {
        return Ok(view);
    };

    if requested_branch == root_label.branch_name {
        return Ok(TreeView {
            root_label: None,
            roots: view.roots,
        });
    }

    let selected_node = view
        .roots
        .iter()
        .find_map(|root| find_tree_node(root, requested_branch))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "tracked branch '{}' was not found in dig tree",
                    requested_branch
                ),
            )
        })?;

    Ok(TreeView {
        root_label: Some(TreeLabel {
            branch_name: selected_node.branch_name.clone(),
            is_current: selected_node.is_current,
            pull_request_number: selected_node.pull_request_number,
        }),
        roots: selected_node.children.clone(),
    })
}

fn focus_tree_view(view: TreeView, requested_branch: &str) -> io::Result<TreeView> {
    let requested_branch = requested_branch.trim();
    if requested_branch.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "branch name cannot be empty",
        ));
    }

    let Some(root_label) = &view.root_label else {
        return Ok(view);
    };

    if requested_branch == root_label.branch_name {
        return Ok(view);
    }

    let mut focused_root = view
        .roots
        .iter()
        .find_map(|root| prune_to_branch_path(root, requested_branch))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "tracked branch '{}' was not found in dig tree",
                    requested_branch
                ),
            )
        })?;

    clear_current_flags(&mut focused_root);
    mark_current_branch(&mut focused_root, requested_branch);

    Ok(TreeView {
        root_label: view.root_label,
        roots: vec![focused_root],
    })
}

fn build_tree_node(
    node: &BranchNode,
    current_branch: Option<&str>,
    child_lookup: &HashMap<Uuid, Vec<&BranchNode>>,
) -> TreeNode {
    let children = child_lookup
        .get(&node.id)
        .map(|children| {
            children
                .iter()
                .map(|child| build_tree_node(child, current_branch, child_lookup))
                .collect()
        })
        .unwrap_or_default();

    TreeNode {
        branch_name: node.branch_name.clone(),
        is_current: current_branch == Some(node.branch_name.as_str()),
        pull_request_number: node.pull_request.as_ref().map(|pr| pr.number),
        children,
    }
}

fn find_tree_node<'a>(node: &'a TreeNode, branch_name: &str) -> Option<&'a TreeNode> {
    if node.branch_name == branch_name {
        return Some(node);
    }

    node.children
        .iter()
        .find_map(|child| find_tree_node(child, branch_name))
}

fn prune_to_branch_path(node: &TreeNode, branch_name: &str) -> Option<TreeNode> {
    if node.branch_name == branch_name {
        return Some(node.clone());
    }

    node.children.iter().find_map(|child| {
        prune_to_branch_path(child, branch_name).map(|pruned_child| TreeNode {
            branch_name: node.branch_name.clone(),
            is_current: node.is_current,
            pull_request_number: node.pull_request_number,
            children: vec![pruned_child],
        })
    })
}

fn clear_current_flags(node: &mut TreeNode) {
    node.is_current = false;
    for child in &mut node.children {
        clear_current_flags(child);
    }
}

fn mark_current_branch(node: &mut TreeNode, branch_name: &str) -> bool {
    if node.branch_name == branch_name {
        node.is_current = true;
        return true;
    }

    for child in &mut node.children {
        if mark_current_branch(child, branch_name) {
            return true;
        }
    }

    false
}

fn sort_branch_nodes(nodes: &mut Vec<&BranchNode>, order_lookup: &HashMap<Uuid, usize>) {
    nodes.sort_by(|left, right| {
        left.created_at_unix_secs
            .cmp(&right.created_at_unix_secs)
            .then_with(|| order_lookup.get(&left.id).cmp(&order_lookup.get(&right.id)))
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });
}

#[cfg(test)]
mod tests {
    use super::{
        TreeLabel, TreeNode, TreeView, build_tree_view, filter_tree_view, focus_tree_view,
    };
    use crate::core::store::types::DIG_STATE_VERSION;
    use crate::core::store::types::DigState;
    use crate::core::store::{BranchNode, ParentRef};
    use uuid::Uuid;

    #[test]
    fn builds_tree_view_from_shared_root_graph() {
        let auth_id = Uuid::new_v4();
        let auth_api_id = Uuid::new_v4();
        let billing_id = Uuid::new_v4();
        let state = DigState {
            version: DIG_STATE_VERSION,
            nodes: vec![
                BranchNode {
                    id: auth_id,
                    branch_name: "feat/auth".into(),
                    parent: ParentRef::Trunk,
                    base_ref: "main".into(),
                    fork_point_oid: "1".into(),
                    head_oid_at_creation: "1".into(),
                    created_at_unix_secs: 1,
                    pull_request: Some(crate::core::store::TrackedPullRequest { number: 101 }),
                    archived: false,
                },
                BranchNode {
                    id: auth_api_id,
                    branch_name: "feat/auth-api".into(),
                    parent: ParentRef::Branch { node_id: auth_id },
                    base_ref: "feat/auth".into(),
                    fork_point_oid: "2".into(),
                    head_oid_at_creation: "2".into(),
                    created_at_unix_secs: 2,
                    pull_request: Some(crate::core::store::TrackedPullRequest { number: 102 }),
                    archived: false,
                },
                BranchNode {
                    id: billing_id,
                    branch_name: "feat/billing".into(),
                    parent: ParentRef::Trunk,
                    base_ref: "main".into(),
                    fork_point_oid: "3".into(),
                    head_oid_at_creation: "3".into(),
                    created_at_unix_secs: 3,
                    pull_request: None,
                    archived: false,
                },
            ],
        };

        assert_eq!(
            build_tree_view(&state, "main", Some("feat/auth-api")),
            TreeView {
                root_label: Some(TreeLabel {
                    branch_name: "main".into(),
                    is_current: false,
                    pull_request_number: None,
                }),
                roots: vec![
                    TreeNode {
                        branch_name: "feat/auth".into(),
                        is_current: false,
                        pull_request_number: Some(101),
                        children: vec![TreeNode {
                            branch_name: "feat/auth-api".into(),
                            is_current: true,
                            pull_request_number: Some(102),
                            children: vec![],
                        }],
                    },
                    TreeNode {
                        branch_name: "feat/billing".into(),
                        is_current: false,
                        pull_request_number: None,
                        children: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn filters_tree_to_selected_branch_subtree() {
        let view = TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![TreeNode {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: Some(101),
                children: vec![
                    TreeNode {
                        branch_name: "feat/auth-api".into(),
                        is_current: false,
                        pull_request_number: Some(102),
                        children: vec![TreeNode {
                            branch_name: "feat/auth-api-tests".into(),
                            is_current: false,
                            pull_request_number: Some(103),
                            children: vec![],
                        }],
                    },
                    TreeNode {
                        branch_name: "feat/auth-ui".into(),
                        is_current: true,
                        pull_request_number: None,
                        children: vec![],
                    },
                ],
            }],
        };

        assert_eq!(
            filter_tree_view(view, Some("feat/auth")).unwrap(),
            TreeView {
                root_label: Some(TreeLabel {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: Some(101),
                }),
                roots: vec![
                    TreeNode {
                        branch_name: "feat/auth-api".into(),
                        is_current: false,
                        pull_request_number: Some(102),
                        children: vec![TreeNode {
                            branch_name: "feat/auth-api-tests".into(),
                            is_current: false,
                            pull_request_number: Some(103),
                            children: vec![],
                        }],
                    },
                    TreeNode {
                        branch_name: "feat/auth-ui".into(),
                        is_current: true,
                        pull_request_number: None,
                        children: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn focuses_tree_to_selected_branch_context() {
        let view = TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: Some(101),
                    children: vec![
                        TreeNode {
                            branch_name: "feat/auth-api".into(),
                            is_current: false,
                            pull_request_number: Some(102),
                            children: vec![TreeNode {
                                branch_name: "feat/auth-api-tests".into(),
                                is_current: false,
                                pull_request_number: Some(103),
                                children: vec![],
                            }],
                        },
                        TreeNode {
                            branch_name: "feat/auth-ui".into(),
                            is_current: false,
                            pull_request_number: None,
                            children: vec![],
                        },
                    ],
                },
                TreeNode {
                    branch_name: "feat/billing".into(),
                    is_current: false,
                    pull_request_number: None,
                    children: vec![],
                },
            ],
        };

        assert_eq!(
            focus_tree_view(view, "feat/auth-api").unwrap(),
            TreeView {
                root_label: Some(TreeLabel {
                    branch_name: "main".into(),
                    is_current: false,
                    pull_request_number: None,
                }),
                roots: vec![TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: Some(101),
                    children: vec![TreeNode {
                        branch_name: "feat/auth-api".into(),
                        is_current: true,
                        pull_request_number: Some(102),
                        children: vec![TreeNode {
                            branch_name: "feat/auth-api-tests".into(),
                            is_current: false,
                            pull_request_number: Some(103),
                            children: vec![],
                        }],
                    }],
                }],
            }
        );
    }
}
