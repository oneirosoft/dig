use crate::cli::common;
use crate::core::clean::CleanEvent;
use crate::core::restack::RestackPreview;
use crate::core::sync::{RemotePushActionKind, SyncEvent, SyncStage, SyncStatus};
use crate::core::tree::{TreeNode, TreeView};
use crate::ui::markers;
use crate::ui::palette::Accent;

pub use super::super::operation::AnimationTerminal;

pub fn render_active_frame(status: Option<(&SyncStatus, usize)>, body: Option<&str>) -> String {
    let Some((status, frame_index)) = status else {
        return body.unwrap_or_default().to_string();
    };

    let header = render_status_header(status, frame_index);

    match body.filter(|body| !body.is_empty()) {
        Some(body) => format!("{header}\n\n{body}"),
        None => header,
    }
}

pub fn render_status_header(status: &SyncStatus, frame_index: usize) -> String {
    let label = format_status_text(status);
    let throbber = markers::THROBBER_FRAMES[frame_index % markers::THROBBER_FRAMES.len()];

    format!(
        "{} {}",
        Accent::SyncInFlight.paint_ansi(&label),
        Accent::SyncInFlight.paint_ansi(throbber)
    )
}

pub fn status_from_clean_event(event: &CleanEvent) -> Option<SyncStatus> {
    match event {
        CleanEvent::RebaseStarted {
            branch_name,
            onto_branch,
        }
        | CleanEvent::RebaseProgress {
            branch_name,
            onto_branch,
            ..
        } => Some(SyncStatus::RestackingBranch {
            branch_name: branch_name.clone(),
            onto_branch: onto_branch.clone(),
        }),
        CleanEvent::DeleteStarted { branch_name } => Some(SyncStatus::DeletingBranch {
            branch_name: branch_name.clone(),
        }),
        CleanEvent::ArchiveStarted { branch_name } => Some(SyncStatus::ArchivingBranch {
            branch_name: branch_name.clone(),
        }),
        CleanEvent::SwitchingToTrunk { .. }
        | CleanEvent::SwitchedToTrunk { .. }
        | CleanEvent::RebaseCompleted { .. }
        | CleanEvent::DeleteCompleted { .. }
        | CleanEvent::ArchiveCompleted { .. } => None,
    }
}

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
            SyncEvent::StatusChanged(_) => false,
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

    pub fn tick(&mut self) -> bool {
        let mut changed = false;

        for root in &mut self.roots {
            changed |= tick_in_flight(root);
        }

        changed
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

    fn tick(&mut self) -> bool {
        let Self::InFlight { frame_index, .. } = self else {
            return false;
        };

        *frame_index = (*frame_index + 1) % markers::THROBBER_FRAMES.len();
        true
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

fn tick_in_flight(node: &mut VisualTreeNode) -> bool {
    let mut changed = node.status.tick();

    for child in &mut node.children {
        changed |= tick_in_flight(child);
    }

    changed
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

fn format_status_text(status: &SyncStatus) -> String {
    match status {
        SyncStatus::FetchingRemotes => "Fetching remotes".to_string(),
        SyncStatus::RepairingClosedPullRequests => "Repairing closed pull requests".to_string(),
        SyncStatus::RemovingMergedLocalBranches => "Removing merged local branches".to_string(),
        SyncStatus::ReconcilingDeletedLocalBranch { step_branch_name } => {
            format!("Reconciling deleted local branch {step_branch_name}")
        }
        SyncStatus::PreparingRestack { step_branch_name } => {
            format!("Preparing restack for {step_branch_name}")
        }
        SyncStatus::RestackingBranch {
            branch_name,
            onto_branch,
        } => format!("Restacking {branch_name} onto {onto_branch}"),
        SyncStatus::InspectingPullRequestUpdates => "Inspecting pull request updates".to_string(),
        SyncStatus::UpdatingPullRequestBase {
            branch_name,
            pull_request_number,
        } => format!("Updating pull request #{pull_request_number} base for {branch_name}"),
        SyncStatus::PushingRemoteBranch {
            branch_name,
            remote_name,
            kind,
        } => format!(
            "{} {branch_name} on {remote_name}",
            match kind {
                RemotePushActionKind::Create => "Creating remote branch",
                RemotePushActionKind::Update => "Pushing",
                RemotePushActionKind::ForceUpdate => "Force-pushing",
            }
        ),
        SyncStatus::DeletingBranch { branch_name } => format!("Deleting {branch_name}"),
        SyncStatus::ArchivingBranch { branch_name } => format!("Archiving {branch_name}"),
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
    use super::{
        SyncAnimation, render_active_frame, render_completed_tree, render_status_header,
        status_from_clean_event,
    };
    use crate::core::clean::CleanEvent;
    use crate::core::restack::RestackPreview;
    use crate::core::store::PendingSyncPhase;
    use crate::core::sync::{SyncEvent, SyncStage, SyncStatus};
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
    fn renders_header_only_status_bar() {
        assert_eq!(
            render_active_frame(Some((&SyncStatus::FetchingRemotes, 0)), None),
            "\u{1b}[38;5;208mFetching remotes\u{1b}[0m \u{1b}[38;5;208m|\u{1b}[0m"
        );
    }

    #[test]
    fn renders_status_bar_above_existing_tree_body() {
        let animation = SyncAnimation::new(&sample_view());

        assert_eq!(
            render_active_frame(
                Some((
                    &SyncStatus::PreparingRestack {
                        step_branch_name: "feat/auth".into(),
                    },
                    1
                )),
                Some(&animation.render_active()),
            ),
            concat!(
                "\u{1b}[38;5;208mPreparing restack for feat/auth\u{1b}[0m ",
                "\u{1b}[38;5;208m/\u{1b}[0m\n\n",
                "main\n",
                "└── feat/auth (#42)\n",
                "    ├── feat/auth-api\n",
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

    #[test]
    fn tick_advances_in_flight_throbber_without_changing_progress() {
        let mut animation = SyncAnimation::new(&sample_view());

        animation.apply_event(&SyncEvent::StageStarted(SyncStage::LocalSync {
            phase: PendingSyncPhase::RestackOutdatedLocalStacks,
            step_branch_name: "feat/auth".into(),
            active_branch_name: "feat/auth-api".into(),
            deleted_branches: Vec::new(),
            restacked_branches: Vec::new(),
        }));
        animation.apply_event(&SyncEvent::RestackProgress {
            branch_name: "feat/auth-api".into(),
            onto_branch: "feat/auth".into(),
            current_commit: 2,
            total_commits: 5,
        });

        let before = animation.render_active();

        assert!(animation.tick());

        let after = animation.render_active();

        assert!(before.contains("\u{1b}[38;5;208m/\u{1b}[0m"));
        assert!(after.contains("\u{1b}[38;5;208m-\u{1b}[0m"));
        assert!(before.contains("[2/5]"));
        assert!(after.contains("[2/5]"));
        assert!(after.contains("\u{1b}[38;5;208mfeat/auth-api\u{1b}[0m"));
    }

    #[test]
    fn header_throbber_advances_without_changing_tree_body() {
        let mut animation = SyncAnimation::new(&sample_view());

        animation.apply_event(&SyncEvent::StageStarted(SyncStage::LocalSync {
            phase: PendingSyncPhase::RestackOutdatedLocalStacks,
            step_branch_name: "feat/auth".into(),
            active_branch_name: "feat/auth-api".into(),
            deleted_branches: Vec::new(),
            restacked_branches: Vec::new(),
        }));
        animation.apply_event(&SyncEvent::RestackProgress {
            branch_name: "feat/auth-api".into(),
            onto_branch: "feat/auth".into(),
            current_commit: 2,
            total_commits: 5,
        });

        let body = animation.render_active();
        let before = render_active_frame(Some((&SyncStatus::FetchingRemotes, 0)), Some(&body));
        let after = render_active_frame(Some((&SyncStatus::FetchingRemotes, 1)), Some(&body));

        assert!(before.contains("Fetching remotes"));
        assert!(after.contains("Fetching remotes"));
        assert!(before.contains("\u{1b}[38;5;208m|\u{1b}[0m"));
        assert!(after.contains("\u{1b}[38;5;208m/\u{1b}[0m"));
        assert_eq!(
            before.split_once("\n\n").map(|(_, body)| body),
            after.split_once("\n\n").map(|(_, body)| body)
        );
        assert!(after.contains("[2/5]"));
    }

    #[test]
    fn final_frames_omit_status_header() {
        let animation = SyncAnimation::new(&sample_view());

        assert_eq!(
            render_active_frame(None, Some(&animation.render_active())),
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
    fn maps_cleanup_events_to_sync_status_bar_actions() {
        assert_eq!(
            status_from_clean_event(&CleanEvent::DeleteStarted {
                branch_name: "feat/auth".into(),
            }),
            Some(SyncStatus::DeletingBranch {
                branch_name: "feat/auth".into(),
            })
        );
        assert_eq!(
            status_from_clean_event(&CleanEvent::RebaseProgress {
                branch_name: "feat/auth-ui".into(),
                onto_branch: "main".into(),
                current_commit: 1,
                total_commits: 2,
            }),
            Some(SyncStatus::RestackingBranch {
                branch_name: "feat/auth-ui".into(),
                onto_branch: "main".into(),
            })
        );
        assert_eq!(
            status_from_clean_event(&CleanEvent::ArchiveCompleted {
                branch_name: "feat/auth".into(),
            }),
            None
        );
    }

    #[test]
    fn renders_status_header_with_right_hand_throbber() {
        assert_eq!(
            render_status_header(
                &SyncStatus::UpdatingPullRequestBase {
                    branch_name: "feat/auth".into(),
                    pull_request_number: 42,
                },
                2,
            ),
            concat!(
                "\u{1b}[38;5;208mUpdating pull request #42 base for feat/auth\u{1b}[0m ",
                "\u{1b}[38;5;208m-\u{1b}[0m"
            )
        );
    }
}
