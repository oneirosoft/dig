use crate::ui::markers;
use crate::ui::palette::Accent;

pub fn render_branch_lineage(lineage: &[String]) -> String {
    let mut lines = Vec::new();

    for (index, branch_name) in lineage.iter().enumerate() {
        if index == 0 {
            lines.push(format!(
                "{} {}",
                Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
                Accent::BranchRef.paint_ansi(branch_name)
            ));
        } else {
            lines.push(branch_name.clone());
        }

        if index + 1 < lineage.len() {
            lines.push(markers::LINEAGE_PIPE.to_string());
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::render_branch_lineage;

    #[test]
    fn renders_linear_branch_lineage_as_vertical_path() {
        let tree = render_branch_lineage(&[
            "feature/api-followup".into(),
            "feature/api".into(),
            "main".into(),
        ]);

        assert_eq!(
            tree,
            "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeature/api-followup\u{1b}[0m\n│\nfeature/api\n│\nmain"
        );
    }
}
