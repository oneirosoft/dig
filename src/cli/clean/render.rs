use crate::core::clean::{CleanCandidate, CleanEvent, CleanPlan, CleanTreeNode};

pub use super::super::operation::AnimationTerminal;
use super::super::operation::{BranchStatus, OperationSection, VisualNode, render_sections};

pub struct CleanAnimation {
    sections: Vec<OperationSection>,
}

impl CleanAnimation {
    pub fn new(plan: &CleanPlan) -> Self {
        Self {
            sections: plan.candidates.iter().map(section_from_candidate).collect(),
        }
    }

    pub fn apply_event(&mut self, event: &CleanEvent) -> bool {
        match event {
            CleanEvent::SwitchingToTrunk { .. } | CleanEvent::SwitchedToTrunk { .. } => false,
            CleanEvent::RebaseStarted {
                branch_name,
                onto_branch: _,
            } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::start_in_flight())
                .is_some(),
            CleanEvent::RebaseProgress {
                branch_name,
                onto_branch: _,
                current_commit,
                total_commits,
            } => self
                .find_node_mut(branch_name)
                .map(|node| {
                    node.status = node
                        .status
                        .advance_progress(*current_commit, *total_commits)
                })
                .is_some(),
            CleanEvent::RebaseCompleted {
                branch_name,
                onto_branch: _,
            } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::Succeeded)
                .is_some(),
            CleanEvent::DeleteStarted { branch_name } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::start_in_flight())
                .is_some(),
            CleanEvent::DeleteCompleted { branch_name } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::Deleted)
                .is_some(),
            CleanEvent::ArchiveStarted { branch_name } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::start_in_flight())
                .is_some(),
            CleanEvent::ArchiveCompleted { branch_name } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = BranchStatus::Archived)
                .is_some(),
        }
    }

    pub fn render_active(&self) -> String {
        render_sections(&self.sections, false)
    }

    pub fn render_final(&self) -> String {
        render_sections(&self.sections, true)
    }

    fn find_node_mut(&mut self, branch_name: &str) -> Option<&mut VisualNode> {
        for section in &mut self.sections {
            if let Some(node) = section.root.find_mut(branch_name) {
                return Some(node);
            }
        }

        None
    }
}

fn section_from_candidate(candidate: &CleanCandidate) -> OperationSection {
    OperationSection {
        root_label: candidate.parent_branch_name.clone(),
        root: visual_node_from_tree(&candidate.tree),
        promote_children_on_deleted_root: true,
    }
}

fn visual_node_from_tree(tree: &CleanTreeNode) -> VisualNode {
    VisualNode::new(
        tree.branch_name.clone(),
        tree.children.iter().map(visual_node_from_tree).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::CleanAnimation;
    use crate::core::clean::{CleanCandidate, CleanEvent, CleanPlan, CleanReason, CleanTreeNode};
    use uuid::Uuid;

    #[test]
    fn renders_deleted_branch_then_final_promoted_children() {
        let mut animation = CleanAnimation::new(&CleanPlan {
            trunk_branch: "main".into(),
            current_branch: "feat/auth".into(),
            requested_branch_name: Some("feat/auth".into()),
            candidates: vec![CleanCandidate {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "main".into(),
                reason: CleanReason::IntegratedIntoParent {
                    parent_branch: "main".into(),
                },
                tree: CleanTreeNode {
                    branch_name: "feat/auth".into(),
                    children: vec![CleanTreeNode {
                        branch_name: "feat/auth-api".into(),
                        children: vec![],
                    }],
                },
                restack_plan: vec![],
                depth: 0,
            }],
            blocked: vec![],
        });

        animation.apply_event(&CleanEvent::RebaseStarted {
            branch_name: "feat/auth-api".into(),
            onto_branch: "main".into(),
        });
        animation.apply_event(&CleanEvent::RebaseProgress {
            branch_name: "feat/auth-api".into(),
            onto_branch: "main".into(),
            current_commit: 2,
            total_commits: 5,
        });
        animation.apply_event(&CleanEvent::RebaseCompleted {
            branch_name: "feat/auth-api".into(),
            onto_branch: "main".into(),
        });
        animation.apply_event(&CleanEvent::DeleteCompleted {
            branch_name: "feat/auth".into(),
        });

        assert_eq!(
            animation.render_active(),
            concat!(
                "main\n",
                "└── \u{1b}[31m✕\u{1b}[0m \u{1b}[31m\u{1b}[9mfeat/auth\u{1b}[0m\n",
                "    └── \u{1b}[32m✓\u{1b}[0m feat/auth-api"
            )
        );
        assert_eq!(
            animation.render_final(),
            concat!("main\n", "└── \u{1b}[32m✓\u{1b}[0m feat/auth-api")
        );
    }

    #[test]
    fn renders_archived_branch_then_final_promoted_children() {
        let mut animation = CleanAnimation::new(&CleanPlan {
            trunk_branch: "main".into(),
            current_branch: "main".into(),
            requested_branch_name: None,
            candidates: vec![CleanCandidate {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "main".into(),
                reason: CleanReason::DeletedLocally,
                tree: CleanTreeNode {
                    branch_name: "feat/auth".into(),
                    children: vec![CleanTreeNode {
                        branch_name: "feat/users".into(),
                        children: vec![],
                    }],
                },
                restack_plan: vec![],
                depth: 0,
            }],
            blocked: vec![],
        });

        animation.apply_event(&CleanEvent::ArchiveCompleted {
            branch_name: "feat/auth".into(),
        });

        assert_eq!(
            animation.render_active(),
            concat!(
                "main\n",
                "└── \u{1b}[33m~\u{1b}[0m \u{1b}[33m\u{1b}[9mfeat/auth\u{1b}[0m\n",
                "    └── feat/users"
            )
        );
        assert_eq!(
            animation.render_final(),
            concat!("main\n", "└── feat/users")
        );
    }
}
