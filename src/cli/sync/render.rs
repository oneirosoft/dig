use crate::cli::common;
use crate::core::restack::RestackPreview;
use crate::core::sync::{SyncEvent, SyncStage};
use crate::core::tree::{TreeNode, TreeView};
use crate::ui::markers;
use crate::ui::palette::Accent;

pub use super::super::operation::AnimationTerminal;

pub struct SyncAnimation {
    root_label: Option<String>,
    roots: Vec<VisualTreeNode>,
}

impl SyncAnimation {
    pub fn new(view: &TreeView) -> Self {
        Self {
            root_label: view
                .root_label
                .as_ref()
                .map(|label| label.branch_name.clone()),
            roots: view.roots.iter().map(visual_node_from_tree).collect(),
        }
    }

    pub fn apply_event(&mut self, event: &SyncEvent) -> bool {
        match event {
            SyncEvent::StageStarted(SyncStage::LocalSync {
                active_branch_name,
                deleted_branches,
                restacked_branches,
                ..
            }) => {
                self.prime_resume(restacked_branches, deleted_branches, active_branch_name);
                true
            }
            SyncEvent::BranchArchived { branch_name } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = SyncBranchStatus::Archived)
                .is_some(),
            SyncEvent::RestackStarted { branch_name, .. } => {
                self.clear_in_flight();
                self.find_node_mut(branch_name)
                    .map(|node| node.status = SyncBranchStatus::start_in_flight())
                    .is_some()
            }
            SyncEvent::RestackProgress {
                branch_name,
                current_commit,
                total_commits,
                ..
            } => self
                .find_node_mut(branch_name)
                .map(|node| {
                    node.status = node
                        .status
                        .advance_progress(*current_commit, *total_commits)
                })
                .is_some(),
            SyncEvent::RestackCompleted { branch_name, .. } => self
                .find_node_mut(branch_name)
                .map(|node| node.status = SyncBranchStatus::Succeeded)
                .is_some(),
            SyncEvent::StageStarted(SyncStage::CleanupResume { .. }) | SyncEvent::Cleanup(_) => {
                false
            }
        }
    }

    pub fn render_active(&self) -> String {
        common::render_tree(
            self.root_label.clone(),
            &self.roots,
            &format_branch_label,
            &|node| node.children.as_slice(),
        )
    }

    pub fn render_final(&self) -> String {
        let roots = prune_final_nodes(&self.roots);

        common::render_tree(
            self.root_label.clone(),
            &roots,
            &format_branch_label,
            &|node| node.children.as_slice(),
        )
    }

    fn prime_resume(
        &mut self,
        restacked_branches: &[RestackPreview],
        deleted_branches: &[String],
        active_branch_name: &str,
    ) {
        self.clear_in_flight();

        for branch in restacked_branches {
            if let Some(node) = self.find_node_mut(&branch.branch_name) {
                node.status = SyncBranchStatus::Succeeded;
            }
        }

        for branch_name in deleted_branches {
            if let Some(node) = self.find_node_mut(branch_name) {
                node.status = SyncBranchStatus::Archived;
            }
        }

        if let Some(node) = self.find_node_mut(active_branch_name) {
            node.status = SyncBranchStatus::start_in_flight();
        }
    }

    fn find_node_mut(&mut self, branch_name: &str) -> Option<&mut VisualTreeNode> {
        for root in &mut self.roots {
            if let Some(node) = root.find_mut(branch_name) {
                return Some(node);
            }
        }

        None
    }

    fn clear_in_flight(&mut self) {
        for root in &mut self.roots {
            clear_in_flight(root);
        }
    }
}

pub fn render_completed_tree(view: &TreeView) -> String {
    common::render_tree(
        view.root_label
            .as_ref()
            .map(|label| format_completed_label(&label.branch_name, label.pull_request_number)),
        &view.roots,
        &format_completed_tree_node_label,
        &|node| node.children.as_slice(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisualTreeNode {
    branch_name: String,
    pull_request_number: Option<u64>,
    status: SyncBranchStatus,
    children: Vec<VisualTreeNode>,
}

impl VisualTreeNode {
    fn find_mut(&mut self, branch_name: &str) -> Option<&mut VisualTreeNode> {
        if self.branch_name == branch_name {
            return Some(self);
        }

        for child in &mut self.children {
            if let Some(found) = child.find_mut(branch_name) {
                return Some(found);
            }
        }

        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SyncBranchStatus {
    Pending,
    InFlight {
        frame_index: usize,
        current_commit: Option<usize>,
        total_commits: Option<usize>,
    },
    Succeeded,
    Archived,
}

impl SyncBranchStatus {
    fn start_in_flight() -> Self {
        Self::InFlight {
            frame_index: 0,
            current_commit: None,
            total_commits: None,
        }
    }

    fn advance_progress(&self, current_commit: usize, total_commits: usize) -> Self {
        let frame_index = match self {
            Self::InFlight { frame_index, .. } => {
                (frame_index + 1) % markers::THROBBER_FRAMES.len()
            }
            _ => 0,
        };

        Self::InFlight {
            frame_index,
            current_commit: Some(current_commit),
            total_commits: Some(total_commits),
        }
    }
}

fn visual_node_from_tree(node: &TreeNode) -> VisualTreeNode {
    VisualTreeNode {
        branch_name: node.branch_name.clone(),
        pull_request_number: node.pull_request_number,
        status: SyncBranchStatus::Pending,
        children: node.children.iter().map(visual_node_from_tree).collect(),
    }
}

fn clear_in_flight(node: &mut VisualTreeNode) {
    if matches!(node.status, SyncBranchStatus::InFlight { .. }) {
        node.status = SyncBranchStatus::Pending;
    }

    for child in &mut node.children {
        clear_in_flight(child);
    }
}

fn prune_final_nodes(nodes: &[VisualTreeNode]) -> Vec<VisualTreeNode> {
    let mut pruned = Vec::new();

    for node in nodes {
        let children = prune_final_nodes(&node.children);
        if matches!(node.status, SyncBranchStatus::Archived) {
            pruned.extend(children);
            continue;
        }

        pruned.push(VisualTreeNode {
            branch_name: node.branch_name.clone(),
            pull_request_number: node.pull_request_number,
            status: node.status.clone(),
            children,
        });
    }

    pruned
}

fn format_branch_text(branch_name: &str, pull_request_number: Option<u64>) -> String {
    match pull_request_number {
        Some(number) => format!("{branch_name} (#{number})"),
        None => branch_name.to_string(),
    }
}

fn format_completed_label(branch_name: &str, pull_request_number: Option<u64>) -> String {
    let label = format_branch_text(branch_name, pull_request_number);
    format!(
        "{} {}",
        Accent::Success.paint_ansi(markers::SUCCESS),
        Accent::Success.paint_ansi(&label)
    )
}

fn format_completed_tree_node_label(node: &TreeNode) -> String {
    format_completed_label(&node.branch_name, node.pull_request_number)
}

fn format_branch_label(node: &VisualTreeNode) -> String {
    let label = format_branch_text(&node.branch_name, node.pull_request_number);

    match &node.status {
        SyncBranchStatus::Pending => label,
        SyncBranchStatus::InFlight {
            frame_index,
            current_commit,
            total_commits,
        } => {
            let marker = Accent::SyncInFlight.paint_ansi(
                markers::THROBBER_FRAMES[*frame_index % markers::THROBBER_FRAMES.len()],
            );
            let progress = match (current_commit, total_commits) {
                (Some(current), Some(total)) => format!(" [{current}/{total}]"),
                _ => String::new(),
            };

            format!(
                "{marker} {}{progress}",
                Accent::SyncInFlight.paint_ansi(&label)
            )
        }
        SyncBranchStatus::Succeeded => {
            format!(
                "{} {}",
                Accent::Success.paint_ansi(markers::SUCCESS),
                Accent::Success.paint_ansi(&label)
            )
        }
        SyncBranchStatus::Archived => {
            format!(
                "{} {}",
                Accent::TagRef.paint_ansi(markers::ARCHIVED),
                Accent::TagRef.paint_struck_ansi(&label)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncAnimation, render_completed_tree};
    use crate::core::restack::RestackPreview;
    use crate::core::store::PendingSyncPhase;
    use crate::core::sync::{SyncEvent, SyncStage};
    use crate::core::tree::{TreeLabel, TreeNode, TreeView};

    fn sample_view() -> TreeView {
        TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![TreeNode {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: Some(42),
                children: vec![
                    TreeNode {
                        branch_name: "feat/auth-api".into(),
                        is_current: false,
                        pull_request_number: None,
                        children: vec![TreeNode {
                            branch_name: "feat/auth-api-tests".into(),
                            is_current: false,
                            pull_request_number: None,
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
            }],
            current_branch_name: Some("main".into()),
            is_current_visible: true,
            current_branch_suffix: None,
        }
    }

    #[test]
    fn renders_pending_tree_without_status_markers() {
        let animation = SyncAnimation::new(&sample_view());

        assert_eq!(
            animation.render_active(),
            concat!(
                "main\n",
                "└── feat/auth (#42)\n",
                "    ├── feat/auth-api\n",
                "    │   └── feat/auth-api-tests\n",
                "    └── feat/auth-ui"
            )
        );
    }

    #[test]
    fn renders_completed_tree_with_all_green_checkmarks() {
        assert_eq!(
            render_completed_tree(&sample_view()),
            concat!(
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mmain\u{1b}[0m\n",
                "└── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth (#42)\u{1b}[0m\n",
                "    ├── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-api\u{1b}[0m\n",
                "    │   └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-api-tests\u{1b}[0m\n",
                "    └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
    }

    #[test]
    fn highlights_active_branch_in_orange() {
        let mut animation = SyncAnimation::new(&sample_view());

        animation.apply_event(&SyncEvent::StageStarted(SyncStage::LocalSync {
            phase: PendingSyncPhase::RestackOutdatedLocalStacks,
            step_branch_name: "feat/auth".into(),
            active_branch_name: "feat/auth-api".into(),
            deleted_branches: Vec::new(),
            restacked_branches: Vec::new(),
        }));

        assert_eq!(
            animation.render_active(),
            concat!(
                "main\n",
                "└── feat/auth (#42)\n",
                "    ├── \u{1b}[38;5;208m|\u{1b}[0m \u{1b}[38;5;208mfeat/auth-api\u{1b}[0m\n",
                "    │   └── feat/auth-api-tests\n",
                "    └── feat/auth-ui"
            )
        );
    }

    #[test]
    fn keeps_completed_branches_green_and_prunes_archived_nodes_from_final_view() {
        let mut animation = SyncAnimation::new(&sample_view());

        animation.apply_event(&SyncEvent::StageStarted(SyncStage::LocalSync {
            phase: PendingSyncPhase::ReconcileDeletedLocalBranches,
            step_branch_name: "feat/auth-api".into(),
            active_branch_name: "feat/auth-api-tests".into(),
            deleted_branches: Vec::new(),
            restacked_branches: vec![RestackPreview {
                branch_name: "feat/auth-ui".into(),
                onto_branch: "feat/auth".into(),
                parent_changed: false,
            }],
        }));
        animation.apply_event(&SyncEvent::BranchArchived {
            branch_name: "feat/auth-api".into(),
        });
        animation.apply_event(&SyncEvent::RestackCompleted {
            branch_name: "feat/auth-api-tests".into(),
            onto_branch: "feat/auth".into(),
        });

        assert_eq!(
            animation.render_active(),
            concat!(
                "main\n",
                "└── feat/auth (#42)\n",
                "    ├── \u{1b}[33m~\u{1b}[0m \u{1b}[33m\u{1b}[9mfeat/auth-api\u{1b}[0m\n",
                "    │   └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-api-tests\u{1b}[0m\n",
                "    └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
        assert_eq!(
            animation.render_final(),
            concat!(
                "main\n",
                "└── feat/auth (#42)\n",
                "    ├── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-api-tests\u{1b}[0m\n",
                "    └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
    }
}
