use std::io;
use std::io::Write;

use crate::core::clean::{CleanCandidate, CleanEvent, CleanPlan, CleanTreeNode};
use crate::ui::markers;
use crate::ui::palette::Accent;

const ANSI_HIDE_CURSOR: &str = "\x1b[?25l";
const ANSI_SHOW_CURSOR: &str = "\x1b[?25h";
const ANSI_CLEAR_TO_END: &str = "\x1b[J";

pub struct AnimationTerminal {
    stdout: io::Stdout,
    active: bool,
    rendered_line_count: usize,
}

impl AnimationTerminal {
    pub fn start() -> io::Result<Self> {
        let mut stdout = io::stdout();
        write!(stdout, "{ANSI_HIDE_CURSOR}")?;
        stdout.flush()?;

        Ok(Self {
            stdout,
            active: true,
            rendered_line_count: 0,
        })
    }

    pub fn render(&mut self, frame: &str) -> io::Result<()> {
        if self.rendered_line_count > 0 {
            write!(self.stdout, "\r")?;

            if self.rendered_line_count > 1 {
                write!(self.stdout, "\x1b[{}A", self.rendered_line_count - 1)?;
            }
        }

        write!(self.stdout, "{ANSI_CLEAR_TO_END}{frame}")?;
        self.stdout.flush()
            .map(|_| self.rendered_line_count = frame_line_count(frame))
    }

    pub fn finish(&mut self, frame: &str) -> io::Result<()> {
        self.render(frame)?;
        write!(self.stdout, "{ANSI_SHOW_CURSOR}\n")?;
        self.stdout.flush()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for AnimationTerminal {
    fn drop(&mut self) {
        if self.active {
            let _ = write!(self.stdout, "{ANSI_SHOW_CURSOR}");
            let _ = self.stdout.flush();
        }
    }
}

pub struct CleanAnimation {
    sections: Vec<CandidateSection>,
}

impl CleanAnimation {
    pub fn new(plan: &CleanPlan) -> Self {
        Self {
            sections: plan
                .candidates
                .iter()
                .map(CandidateSection::from_candidate)
                .collect(),
        }
    }

    pub fn apply_event(&mut self, event: &CleanEvent) -> bool {
        match event {
            CleanEvent::SwitchingToTrunk { .. } | CleanEvent::SwitchedToTrunk { .. } => false,
            CleanEvent::RebaseStarted {
                branch_name,
                onto_branch: _,
            } => {
                if let Some(node) = self.find_node_mut(branch_name) {
                    node.status = BranchStatus::InFlight {
                        frame_index: 0,
                        current_commit: None,
                        total_commits: None,
                    };
                    true
                } else {
                    false
                }
            }
            CleanEvent::RebaseProgress {
                branch_name,
                onto_branch: _,
                current_commit,
                total_commits,
            } => {
                if let Some(node) = self.find_node_mut(branch_name) {
                    let next_frame = match node.status {
                        BranchStatus::InFlight { frame_index, .. } => {
                            (frame_index + 1) % markers::THROBBER_FRAMES.len()
                        }
                        _ => 0,
                    };

                    node.status = BranchStatus::InFlight {
                        frame_index: next_frame,
                        current_commit: Some(*current_commit),
                        total_commits: Some(*total_commits),
                    };
                    true
                } else {
                    false
                }
            }
            CleanEvent::RebaseCompleted {
                branch_name,
                onto_branch: _,
            } => {
                if let Some(node) = self.find_node_mut(branch_name) {
                    node.status = BranchStatus::Succeeded;
                    true
                } else {
                    false
                }
            }
            CleanEvent::DeleteStarted { branch_name } => {
                if let Some(node) = self.find_node_mut(branch_name) {
                    node.status = BranchStatus::InFlight {
                        frame_index: 0,
                        current_commit: None,
                        total_commits: None,
                    };
                    true
                } else {
                    false
                }
            }
            CleanEvent::DeleteCompleted { branch_name } => {
                if let Some(node) = self.find_node_mut(branch_name) {
                    node.status = BranchStatus::Deleted;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn render_active(&self) -> String {
        self.sections
            .iter()
            .map(|section| render_section(section, false))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn render_final(&self) -> String {
        self.sections
            .iter()
            .map(|section| render_section(section, true))
            .collect::<Vec<_>>()
            .join("\n\n")
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

#[derive(Debug)]
struct CandidateSection {
    parent_branch_name: String,
    root: VisualNode,
}

impl CandidateSection {
    fn from_candidate(candidate: &CleanCandidate) -> Self {
        Self {
            parent_branch_name: candidate.parent_branch_name.clone(),
            root: VisualNode::from_tree(&candidate.tree),
        }
    }
}

#[derive(Debug)]
struct VisualNode {
    branch_name: String,
    status: BranchStatus,
    children: Vec<VisualNode>,
}

impl VisualNode {
    fn from_tree(tree: &CleanTreeNode) -> Self {
        Self {
            branch_name: tree.branch_name.clone(),
            status: BranchStatus::Pending,
            children: tree.children.iter().map(Self::from_tree).collect(),
        }
    }

    fn find_mut(&mut self, branch_name: &str) -> Option<&mut VisualNode> {
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

#[derive(Debug)]
enum BranchStatus {
    Pending,
    InFlight {
        frame_index: usize,
        current_commit: Option<usize>,
        total_commits: Option<usize>,
    },
    Succeeded,
    Deleted,
}

fn render_section(section: &CandidateSection, final_view: bool) -> String {
    let mut lines = vec![section.parent_branch_name.clone()];

    if final_view && matches!(section.root.status, BranchStatus::Deleted) {
        for (index, child) in section.root.children.iter().enumerate() {
            render_node(child, "", index + 1 == section.root.children.len(), &mut lines);
        }
    } else {
        render_node(&section.root, "", true, &mut lines);
    }

    lines.join("\n")
}

fn render_node(node: &VisualNode, prefix: &str, is_last: bool, lines: &mut Vec<String>) {
    let connector = if is_last { "└──" } else { "├──" };
    lines.push(format!(
        "{prefix}{connector} {}",
        format_branch_label(node)
    ));

    let child_prefix = if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    for (index, child) in node.children.iter().enumerate() {
        render_node(child, &child_prefix, index + 1 == node.children.len(), lines);
    }
}

fn format_branch_label(node: &VisualNode) -> String {
    match &node.status {
        BranchStatus::Pending => node.branch_name.clone(),
        BranchStatus::InFlight {
            frame_index,
            current_commit,
            total_commits,
        } => {
            let marker = Accent::InFlight.paint_ansi(
                markers::THROBBER_FRAMES[*frame_index % markers::THROBBER_FRAMES.len()],
            );
            let progress = match (current_commit, total_commits) {
                (Some(current), Some(total)) => format!(" [{current}/{total}]"),
                _ => String::new(),
            };

            format!("{marker} {}{progress}", node.branch_name)
        }
        BranchStatus::Succeeded => {
            format!(
                "{} {}",
                Accent::Success.paint_ansi(markers::SUCCESS),
                node.branch_name
            )
        }
        BranchStatus::Deleted => {
            format!(
                "{} {}",
                Accent::Failure.paint_ansi(markers::DELETED),
                Accent::Failure.paint_struck_ansi(&node.branch_name)
            )
        }
    }
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
            concat!(
                "main\n",
                "└── \u{1b}[32m✓\u{1b}[0m feat/auth-api"
            )
        );
    }
}

fn frame_line_count(frame: &str) -> usize {
    frame.lines().count().max(1)
}
