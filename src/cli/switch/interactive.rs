use std::cmp;
use std::env;
use std::io;
use std::io::Write;

use ratatui::crossterm::QueueableCommand;
use ratatui::crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::terminal::{self, Clear, ClearType};

use crate::core::tree::TreeView;
use crate::ui::markers;
use crate::ui::palette::Accent;

use super::super::tree::{StackTreeRow, stack_tree_rows};

const MAX_VISIBLE_ROWS: usize = 12;
const SCRIPTED_EVENTS_ENV: &str = "DGR_SWITCH_TEST_EVENTS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InteractiveOutcome {
    Selected(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InputEvent {
    Up,
    Down,
    Confirm,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InteractiveState {
    rows: Vec<StackTreeRow>,
    selected_index: usize,
    untracked_current_branch: Option<String>,
}

impl InteractiveState {
    fn new(view: &TreeView) -> Self {
        let rows = stack_tree_rows(view);
        let selected_index = view
            .current_branch_name
            .as_deref()
            .and_then(|branch_name| rows.iter().position(|row| row.branch_name == branch_name))
            .unwrap_or_default();
        let untracked_current_branch = view
            .current_branch_name
            .as_ref()
            .filter(|branch_name| rows.iter().all(|row| row.branch_name != **branch_name))
            .cloned();

        Self {
            rows,
            selected_index,
            untracked_current_branch,
        }
    }

    fn apply_event(&mut self, event: InputEvent) -> Option<InteractiveOutcome> {
        match event {
            InputEvent::Up => {
                self.selected_index = self.selected_index.saturating_sub(1);
                None
            }
            InputEvent::Down => {
                let last_index = self.rows.len().saturating_sub(1);
                self.selected_index = cmp::min(self.selected_index + 1, last_index);
                None
            }
            InputEvent::Confirm => Some(InteractiveOutcome::Selected(
                self.selected_branch_name().to_string(),
            )),
            InputEvent::Cancel => Some(InteractiveOutcome::Cancelled),
        }
    }

    fn render(&self) -> String {
        let range = visible_range(self.rows.len(), self.selected_index, MAX_VISIBLE_ROWS);
        let range_start = range.start;
        let mut lines = self.rows[range]
            .iter()
            .enumerate()
            .map(|(offset, row)| {
                let row_index = range_start + offset;
                let is_selected = row_index == self.selected_index;
                let selector = if is_selected {
                    Accent::HeadMarker.paint_ansi(markers::HEAD)
                } else {
                    " ".to_string()
                };
                let line = if is_selected {
                    Accent::HeadMarker.paint_ansi(&row.line)
                } else {
                    row.line.clone()
                };

                format!("{selector} {line}")
            })
            .collect::<Vec<_>>();

        lines.push(String::new());
        lines.push("↑/↓ or j/k move • Enter switch • Esc/q cancel".into());

        if let Some(branch_name) = &self.untracked_current_branch {
            lines.push(format!(
                "Current branch '{}' is untracked; starting on '{}'.",
                branch_name, self.rows[0].branch_name
            ));
        }

        lines.join("\n")
    }

    fn selected_branch_name(&self) -> &str {
        &self.rows[self.selected_index].branch_name
    }
}

pub(super) fn run(view: &TreeView) -> io::Result<InteractiveOutcome> {
    let mut state = InteractiveState::new(view);
    let mut terminal = InteractiveTerminal::start()?;

    loop {
        terminal.render(&state.render())?;

        let Some(event) = read_input_event()? else {
            continue;
        };

        if let Some(outcome) = state.apply_event(event) {
            terminal.finish()?;
            return Ok(outcome);
        }
    }
}

pub(super) fn run_scripted(
    view: &TreeView,
    events: &[InputEvent],
) -> io::Result<InteractiveOutcome> {
    let mut state = InteractiveState::new(view);

    for event in events {
        if let Some(outcome) = state.apply_event(*event) {
            return Ok(outcome);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "scripted switch session ended without Enter, Esc, or q",
    ))
}

pub(super) fn scripted_events_from_env() -> io::Result<Option<Vec<InputEvent>>> {
    let Some(value) = env::var_os(SCRIPTED_EVENTS_ENV) else {
        return Ok(None);
    };

    let mut events = Vec::new();
    for token in value.to_string_lossy().split(',').map(str::trim) {
        if token.is_empty() {
            continue;
        }

        events.push(parse_scripted_event(token)?);
    }

    Ok(Some(events))
}

fn parse_scripted_event(token: &str) -> io::Result<InputEvent> {
    match token {
        "up" | "k" => Ok(InputEvent::Up),
        "down" | "j" => Ok(InputEvent::Down),
        "enter" => Ok(InputEvent::Confirm),
        "esc" | "q" | "cancel" => Ok(InputEvent::Cancel),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported scripted switch event '{token}' in {SCRIPTED_EVENTS_ENV}"),
        )),
    }
}

fn read_input_event() -> io::Result<Option<InputEvent>> {
    loop {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                return Ok(match key_event.code {
                    KeyCode::Up | KeyCode::Char('k') => Some(InputEvent::Up),
                    KeyCode::Down | KeyCode::Char('j') => Some(InputEvent::Down),
                    KeyCode::Enter => Some(InputEvent::Confirm),
                    KeyCode::Esc | KeyCode::Char('q') => Some(InputEvent::Cancel),
                    _ => None,
                });
            }
            Event::Key(_) => continue,
            _ => return Ok(None),
        }
    }
}

fn visible_range(row_count: usize, selected_index: usize, limit: usize) -> std::ops::Range<usize> {
    let visible_count = cmp::min(row_count, limit);
    let half = visible_count / 2;
    let mut start = selected_index.saturating_sub(half);

    if start + visible_count > row_count {
        start = row_count.saturating_sub(visible_count);
    }

    start..start + visible_count
}

struct InteractiveTerminal {
    stdout: io::Stdout,
    rendered_line_count: usize,
    active: bool,
}

impl InteractiveTerminal {
    fn start() -> io::Result<Self> {
        terminal::enable_raw_mode()?;

        let mut stdout = io::stdout();
        stdout.queue(Hide)?;
        stdout.flush()?;

        Ok(Self {
            stdout,
            rendered_line_count: 0,
            active: true,
        })
    }

    fn render(&mut self, frame: &str) -> io::Result<()> {
        self.move_to_frame_top()?;
        self.stdout.queue(Clear(ClearType::FromCursorDown))?;
        write!(self.stdout, "{}", terminal_frame_text(frame))?;
        self.stdout.flush()?;
        self.rendered_line_count = frame_line_count(frame);
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.clear_rendered()?;
        self.restore_terminal()
    }

    fn clear_rendered(&mut self) -> io::Result<()> {
        self.move_to_frame_top()?;
        self.stdout.queue(Clear(ClearType::FromCursorDown))?;
        self.stdout.flush()?;
        self.rendered_line_count = 0;
        Ok(())
    }

    fn move_to_frame_top(&mut self) -> io::Result<()> {
        self.stdout.queue(MoveToColumn(0))?;

        if self.rendered_line_count > 1 {
            self.stdout
                .queue(MoveUp((self.rendered_line_count - 1) as u16))?;
        }

        Ok(())
    }

    fn restore_terminal(&mut self) -> io::Result<()> {
        self.stdout.queue(Show)?;
        self.stdout.flush()?;
        terminal::disable_raw_mode()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for InteractiveTerminal {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let _ = self.clear_rendered();
        let _ = self.stdout.queue(Show);
        let _ = self.stdout.flush();
        let _ = terminal::disable_raw_mode();
    }
}

fn frame_line_count(frame: &str) -> usize {
    cmp::max(frame.lines().count(), 1)
}

fn terminal_frame_text(frame: &str) -> String {
    frame.replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::{
        InputEvent, InteractiveOutcome, InteractiveState, terminal_frame_text, visible_range,
    };
    use crate::core::tree::{TreeLabel, TreeNode, TreeView};
    use crate::ui::markers;
    use crate::ui::palette::Accent;

    #[test]
    fn starts_on_current_tracked_branch() {
        let state = InteractiveState::new(&sample_view(Some("feat/auth-ui")));

        assert_eq!(state.selected_branch_name(), "feat/auth-ui");
        assert_eq!(state.untracked_current_branch, None);
        let rendered = state.render();
        assert!(rendered.contains(&Accent::HeadMarker.paint_ansi(markers::HEAD)));
        assert!(rendered.contains("↑/↓ or j/k move • Enter switch • Esc/q cancel"));
        assert!(rendered.contains(Accent::HeadMarker.ansi()));
    }

    #[test]
    fn starts_on_trunk_when_current_branch_is_untracked() {
        let state = InteractiveState::new(&sample_view(Some("scratch")));

        assert_eq!(state.selected_branch_name(), "main");
        assert_eq!(state.untracked_current_branch.as_deref(), Some("scratch"));
        assert!(
            state
                .render()
                .contains("Current branch 'scratch' is untracked")
        );
    }

    #[test]
    fn clamps_navigation_to_top_and_bottom() {
        let mut state = InteractiveState::new(&sample_view(Some("main")));

        state.apply_event(InputEvent::Up);
        assert_eq!(state.selected_branch_name(), "main");

        for _ in 0..10 {
            state.apply_event(InputEvent::Down);
        }
        assert_eq!(state.selected_branch_name(), "feat/billing");
    }

    #[test]
    fn supports_arrow_and_vim_style_vertical_navigation() {
        let mut state = InteractiveState::new(&sample_view(Some("main")));

        state.apply_event(InputEvent::Down);
        assert_eq!(state.selected_branch_name(), "feat/auth");

        state.apply_event(InputEvent::Up);
        assert_eq!(state.selected_branch_name(), "main");
    }

    #[test]
    fn confirms_or_cancels_selection() {
        let mut state = InteractiveState::new(&sample_view(Some("main")));
        state.apply_event(InputEvent::Down);

        assert_eq!(
            state.apply_event(InputEvent::Confirm),
            Some(InteractiveOutcome::Selected("feat/auth".into()))
        );
        assert_eq!(
            InteractiveState::new(&sample_view(Some("main"))).apply_event(InputEvent::Cancel),
            Some(InteractiveOutcome::Cancelled)
        );
    }

    #[test]
    fn limits_rendered_rows_to_viewport() {
        let mut rows = Vec::new();
        for index in 0..20 {
            rows.push(TreeNode {
                branch_name: format!("feat/{index}"),
                is_current: index == 10,
                pull_request_number: None,
                children: vec![],
            });
        }

        let state = InteractiveState::new(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: rows,
            current_branch_name: Some("feat/10".into()),
            is_current_visible: true,
            current_branch_suffix: None,
        });

        let rendered = state.render();
        let branch_lines = rendered
            .lines()
            .take_while(|line| !line.is_empty())
            .collect::<Vec<_>>();

        assert_eq!(branch_lines.len(), 12);
        assert!(branch_lines.iter().any(|line| line.contains("feat/10")));
    }

    #[test]
    fn centers_selected_row_within_visible_range_when_possible() {
        assert_eq!(visible_range(20, 0, 12), 0..12);
        assert_eq!(visible_range(20, 10, 12), 4..16);
        assert_eq!(visible_range(20, 19, 12), 8..20);
    }

    #[test]
    fn terminal_output_uses_carriage_return_line_breaks() {
        assert_eq!(terminal_frame_text("one\ntwo"), "one\r\ntwo");
    }

    fn sample_view(current_branch_name: Option<&str>) -> TreeView {
        TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: current_branch_name == Some("main"),
                pull_request_number: None,
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: current_branch_name == Some("feat/auth"),
                    pull_request_number: Some(101),
                    children: vec![TreeNode {
                        branch_name: "feat/auth-ui".into(),
                        is_current: current_branch_name == Some("feat/auth-ui"),
                        pull_request_number: Some(102),
                        children: vec![],
                    }],
                },
                TreeNode {
                    branch_name: "feat/billing".into(),
                    is_current: current_branch_name == Some("feat/billing"),
                    pull_request_number: None,
                    children: vec![],
                },
            ],
            current_branch_name: current_branch_name.map(str::to_string),
            is_current_visible: current_branch_name != Some("scratch"),
            current_branch_suffix: None,
        }
    }
}
