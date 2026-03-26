use std::io;
use std::io::Write;

use super::common;
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
        self.stdout
            .flush()
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

#[derive(Debug)]
pub struct OperationSection {
    pub root_label: String,
    pub root: VisualNode,
    pub promote_children_on_deleted_root: bool,
}

#[derive(Debug)]
pub struct VisualNode {
    pub branch_name: String,
    pub status: BranchStatus,
    pub children: Vec<VisualNode>,
}

impl VisualNode {
    pub fn new(branch_name: String, children: Vec<VisualNode>) -> Self {
        Self {
            branch_name,
            status: BranchStatus::Pending,
            children,
        }
    }

    pub fn find_mut(&mut self, branch_name: &str) -> Option<&mut VisualNode> {
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
pub enum BranchStatus {
    Pending,
    InFlight {
        frame_index: usize,
        current_commit: Option<usize>,
        total_commits: Option<usize>,
    },
    Succeeded,
    Deleted,
    Archived,
}

impl BranchStatus {
    pub fn start_in_flight() -> Self {
        Self::InFlight {
            frame_index: 0,
            current_commit: None,
            total_commits: None,
        }
    }

    pub fn advance_progress(&self, current_commit: usize, total_commits: usize) -> Self {
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

pub fn render_sections(sections: &[OperationSection], final_view: bool) -> String {
    common::join_sections(
        &sections
            .iter()
            .map(|section| render_section(section, final_view))
            .collect::<Vec<_>>(),
    )
}

fn render_section(section: &OperationSection, final_view: bool) -> String {
    let roots = if final_view
        && section.promote_children_on_deleted_root
        && matches!(
            section.root.status,
            BranchStatus::Deleted | BranchStatus::Archived
        ) {
        section.root.children.as_slice()
    } else {
        std::slice::from_ref(&section.root)
    };

    common::render_tree(
        Some(section.root_label.clone()),
        roots,
        &format_branch_label,
        &|node| node.children.as_slice(),
    )
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
        BranchStatus::Archived => {
            format!(
                "{} {}",
                Accent::TagRef.paint_ansi(markers::ARCHIVED),
                Accent::TagRef.paint_struck_ansi(&node.branch_name)
            )
        }
    }
}

fn frame_line_count(frame: &str) -> usize {
    frame.lines().count().max(1)
}
