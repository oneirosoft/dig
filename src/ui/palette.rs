use ratatui::style::Color;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_STRIKETHROUGH: &str = "\x1b[9m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    HeadMarker,
    BranchRef,
    CommitHash,
    TagRef,
    InFlight,
    SyncInFlight,
    Success,
    Failure,
}

impl Accent {
    pub const fn ansi(self) -> &'static str {
        match self {
            Self::HeadMarker => "\x1b[34m",
            Self::BranchRef => "\x1b[32m",
            Self::CommitHash => "\x1b[33m",
            Self::TagRef => "\x1b[33m",
            Self::InFlight => "\x1b[34m",
            Self::SyncInFlight => "\x1b[38;5;208m",
            Self::Success => "\x1b[32m",
            Self::Failure => "\x1b[31m",
        }
    }

    // TODO: Remove this allowance once the ratatui views start consuming the shared accent palette.
    #[allow(dead_code)]
    pub const fn tui(self) -> Color {
        match self {
            Self::HeadMarker => Color::Blue,
            Self::BranchRef => Color::Green,
            Self::CommitHash => Color::Yellow,
            Self::TagRef => Color::Yellow,
            Self::InFlight => Color::Blue,
            Self::SyncInFlight => Color::Indexed(208),
            Self::Success => Color::Green,
            Self::Failure => Color::Red,
        }
    }

    pub fn paint_ansi(self, text: &str) -> String {
        format!("{}{}{}", self.ansi(), text, ANSI_RESET)
    }

    pub fn paint_struck_ansi(self, text: &str) -> String {
        format!(
            "{}{}{}{}",
            self.ansi(),
            ANSI_STRIKETHROUGH,
            text,
            ANSI_RESET
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Accent;
    use ratatui::style::Color;

    #[test]
    fn maps_accents_to_ansi_and_tui_colors() {
        assert_eq!(Accent::HeadMarker.ansi(), "\x1b[34m");
        assert_eq!(Accent::BranchRef.ansi(), "\x1b[32m");
        assert_eq!(Accent::CommitHash.ansi(), "\x1b[33m");
        assert_eq!(Accent::Failure.ansi(), "\x1b[31m");
        assert_eq!(Accent::TagRef.tui(), Color::Yellow);
        assert_eq!(
            Accent::Failure.paint_struck_ansi("branch"),
            "\u{1b}[31m\u{1b}[9mbranch\u{1b}[0m"
        );
    }
}
