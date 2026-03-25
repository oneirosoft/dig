use crate::core::tree::{TreeLabel, TreeNode, TreeView};
use crate::ui::markers;
use crate::ui::palette::Accent;

pub fn render_branch_lineage(lineage: &[String]) -> String {
    let mut lines = Vec::new();

    for (index, branch_name) in lineage.iter().enumerate() {
        lines.push(format_lineage_branch(branch_name, index == 0));

        if index + 1 < lineage.len() {
            lines.push(format!("{} ", markers::LINEAGE_PIPE));
        }
    }

    lines.join("\n")
}

pub fn render_stack_tree(view: &TreeView) -> String {
    let mut lines = Vec::new();

    if let Some(root_label) = &view.root_label {
        lines.push(format_tree_label(root_label));
    }

    for (index, root) in view.roots.iter().enumerate() {
        render_tree_node(root, "", index + 1 == view.roots.len(), &mut lines);
    }

    lines.join("\n")
}

fn format_tree_label(root_label: &TreeLabel) -> String {
    format_branch_label(&root_label.branch_name, root_label.is_current)
}

fn render_tree_node(node: &TreeNode, prefix: &str, is_last: bool, lines: &mut Vec<String>) {
    let connector = if is_last { "└──" } else { "├──" };
    lines.push(format!(
        "{prefix}{connector} {}",
        format_branch_label(&node.branch_name, node.is_current)
    ));

    let child_prefix = if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}│   ")
    };

    for (index, child) in node.children.iter().enumerate() {
        render_tree_node(
            child,
            &child_prefix,
            index + 1 == node.children.len(),
            lines,
        );
    }
}

fn format_branch_label(branch_name: &str, is_current: bool) -> String {
    if is_current {
        format!(
            "{} {}",
            Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
            Accent::BranchRef.paint_ansi(branch_name)
        )
    } else {
        branch_name.to_string()
    }
}

fn format_lineage_branch(branch_name: &str, is_current: bool) -> String {
    if is_current {
        format!(
            "{} {}",
            Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
            Accent::BranchRef.paint_ansi(branch_name)
        )
    } else {
        format!("{}  {}", markers::NON_CURRENT_BRANCH, branch_name)
    }
}

#[cfg(test)]
mod tests {
    use super::{render_branch_lineage, render_stack_tree};
    use crate::core::tree::{TreeLabel, TreeNode, TreeView};

    #[test]
    fn renders_linear_branch_lineage_as_vertical_path() {
        let tree = render_branch_lineage(&[
            "feature/api-followup".into(),
            "feature/api".into(),
            "main".into(),
        ]);

        assert_eq!(
            tree,
            concat!(
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeature/api-followup\u{1b}[0m\n",
                "│ \n",
                "*  feature/api\n",
                "│ \n",
                "*  main"
            )
        );
    }

    #[test]
    fn renders_single_branch_lineage_without_connectors() {
        let tree = render_branch_lineage(&["main".into()]);

        assert_eq!(tree, "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mmain\u{1b}[0m");
    }

    #[test]
    fn renders_shared_root_stack_tree() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    children: vec![
                        TreeNode {
                            branch_name: "feat/auth-api".into(),
                            is_current: false,
                            children: vec![TreeNode {
                                branch_name: "feat/auth-api-tests".into(),
                                is_current: false,
                                children: vec![],
                            }],
                        },
                        TreeNode {
                            branch_name: "feat/auth-ui".into(),
                            is_current: true,
                            children: vec![],
                        },
                    ],
                },
                TreeNode {
                    branch_name: "feat/billing".into(),
                    is_current: false,
                    children: vec![TreeNode {
                        branch_name: "feat/billing-retry".into(),
                        is_current: false,
                        children: vec![],
                    }],
                },
                TreeNode {
                    branch_name: "docs/readme".into(),
                    is_current: false,
                    children: vec![],
                },
            ],
        });

        assert_eq!(
            rendered,
            concat!(
                "main\n",
                "├── feat/auth\n",
                "│   ├── feat/auth-api\n",
                "│   │   └── feat/auth-api-tests\n",
                "│   └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m\n",
                "├── feat/billing\n",
                "│   └── feat/billing-retry\n",
                "└── docs/readme"
            )
        );
    }

    #[test]
    fn renders_filtered_branch_tree_without_trunk_header() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "feat/auth".into(),
                is_current: false,
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth-api".into(),
                    is_current: false,
                    children: vec![TreeNode {
                        branch_name: "feat/auth-api-tests".into(),
                        is_current: false,
                        children: vec![],
                    }],
                },
                TreeNode {
                    branch_name: "feat/auth-ui".into(),
                    is_current: true,
                    children: vec![],
                },
            ],
        });

        assert_eq!(
            rendered,
            concat!(
                "feat/auth\n",
                "├── feat/auth-api\n",
                "│   └── feat/auth-api-tests\n",
                "└── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
    }
}
