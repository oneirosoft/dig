use crate::cli::common;
use crate::core::graph::BranchLineageNode;
use crate::core::tree::{TreeLabel, TreeView};
use crate::ui::markers;
use crate::ui::palette::Accent;

pub fn render_branch_lineage(lineage: &[BranchLineageNode]) -> String {
    let mut lines = Vec::new();

    for (index, branch) in lineage.iter().enumerate() {
        lines.push(format_lineage_branch(branch, index == 0));

        if index + 1 < lineage.len() {
            lines.push(format!("{} ", markers::LINEAGE_PIPE));
        }
    }

    lines.join("\n")
}

pub fn render_stack_tree(view: &TreeView) -> String {
    let mut rendered = common::render_tree(
        view.root_label.as_ref().map(format_tree_label),
        &view.roots,
        &|node| format_branch_label(&node.branch_name, node.is_current, node.pull_request_number),
        &|node| node.children.as_slice(),
    );

    if !view.is_current_visible {
        if let Some(current_branch) = &view.current_branch_name {
            let label = match &view.current_branch_suffix {
                Some(suffix) => format!("{current_branch} {suffix}"),
                None => current_branch.clone(),
            };

            rendered.push_str("\n\n");
            rendered.push_str(&format!(
                "{} {}",
                Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
                Accent::BranchRef.paint_ansi(&label)
            ));
        }
    }

    rendered
}

fn format_tree_label(root_label: &TreeLabel) -> String {
    format_branch_label(
        &root_label.branch_name,
        root_label.is_current,
        root_label.pull_request_number,
    )
}

fn format_branch_text(branch_name: &str, pull_request_number: Option<u64>) -> String {
    match pull_request_number {
        Some(number) => format!("{branch_name} (#{number})"),
        None => branch_name.to_string(),
    }
}

fn format_branch_label(
    branch_name: &str,
    is_current: bool,
    pull_request_number: Option<u64>,
) -> String {
    let label = format_branch_text(branch_name, pull_request_number);

    if is_current {
        format!(
            "{} {}",
            Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
            Accent::BranchRef.paint_ansi(&label)
        )
    } else {
        format!("{} {}", markers::NON_CURRENT_BRANCH, label)
    }
}

fn format_lineage_branch(branch: &BranchLineageNode, is_current: bool) -> String {
    let label = format_branch_text(&branch.branch_name, branch.pull_request_number);

    if is_current {
        format!(
            "{} {}",
            Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
            Accent::BranchRef.paint_ansi(&label)
        )
    } else {
        format!("{} {}", markers::NON_CURRENT_BRANCH, label)
    }
}

#[cfg(test)]
mod tests {
    use super::{render_branch_lineage, render_stack_tree};
    use crate::core::graph::BranchLineageNode;
    use crate::core::tree::{TreeLabel, TreeNode, TreeView};

    #[test]
    fn renders_linear_branch_lineage_as_vertical_path() {
        let tree = render_branch_lineage(&[
            BranchLineageNode {
                branch_name: "feature/api-followup".into(),
                pull_request_number: None,
            },
            BranchLineageNode {
                branch_name: "feature/api".into(),
                pull_request_number: None,
            },
            BranchLineageNode {
                branch_name: "main".into(),
                pull_request_number: None,
            },
        ]);

        assert_eq!(
            tree,
            concat!(
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeature/api-followup\u{1b}[0m\n",
                "│ \n",
                "* feature/api\n",
                "│ \n",
                "* main"
            )
        );
    }

    #[test]
    fn renders_single_branch_lineage_without_connectors() {
        let tree = render_branch_lineage(&[BranchLineageNode {
            branch_name: "main".into(),
            pull_request_number: None,
        }]);

        assert_eq!(tree, "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mmain\u{1b}[0m");
    }

    #[test]
    fn renders_pull_request_numbers_in_lineage_view() {
        let tree = render_branch_lineage(&[
            BranchLineageNode {
                branch_name: "feature/api-followup".into(),
                pull_request_number: Some(43),
            },
            BranchLineageNode {
                branch_name: "feature/api".into(),
                pull_request_number: Some(42),
            },
            BranchLineageNode {
                branch_name: "main".into(),
                pull_request_number: None,
            },
        ]);

        assert_eq!(
            tree,
            concat!(
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeature/api-followup (#43)\u{1b}[0m\n",
                "│ \n",
                "* feature/api (#42)\n",
                "│ \n",
                "* main"
            )
        );
    }

    #[test]
    fn renders_shared_root_stack_tree() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: None,
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
                            is_current: true,
                            pull_request_number: None,
                            children: vec![],
                        },
                    ],
                },
                TreeNode {
                    branch_name: "feat/billing".into(),
                    is_current: false,
                    pull_request_number: None,
                    children: vec![TreeNode {
                        branch_name: "feat/billing-retry".into(),
                        is_current: false,
                        pull_request_number: None,
                        children: vec![],
                    }],
                },
                TreeNode {
                    branch_name: "docs/readme".into(),
                    is_current: false,
                    pull_request_number: None,
                    children: vec![],
                },
            ],
            current_branch_name: Some("feat/auth-ui".into()),
            is_current_visible: true,
            current_branch_suffix: None,
        });

        assert_eq!(
            rendered,
            concat!(
                "* main\n",
                "├── * feat/auth\n",
                "│   ├── * feat/auth-api\n",
                "│   │   └── * feat/auth-api-tests\n",
                "│   └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m\n",
                "├── * feat/billing\n",
                "│   └── * feat/billing-retry\n",
                "└── * docs/readme"
            )
        );
    }

    #[test]
    fn renders_filtered_branch_tree_without_trunk_header() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![
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
                    is_current: true,
                    pull_request_number: None,
                    children: vec![],
                },
            ],
            current_branch_name: Some("feat/auth-ui".into()),
            is_current_visible: true,
            current_branch_suffix: None,
        });

        assert_eq!(
            rendered,
            concat!(
                "* feat/auth\n",
                "├── * feat/auth-api\n",
                "│   └── * feat/auth-api-tests\n",
                "└── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
    }

    #[test]
    fn renders_pull_request_numbers_for_normal_current_and_filtered_root_labels() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: Some(42),
            }),
            roots: vec![
                TreeNode {
                    branch_name: "feat/auth-api".into(),
                    is_current: false,
                    pull_request_number: Some(43),
                    children: vec![],
                },
                TreeNode {
                    branch_name: "feat/auth-ui".into(),
                    is_current: true,
                    pull_request_number: Some(44),
                    children: vec![],
                },
            ],
            current_branch_name: Some("feat/auth-ui".into()),
            is_current_visible: true,
            current_branch_suffix: None,
        });

        assert_eq!(
            rendered,
            concat!(
                "* feat/auth (#42)\n",
                "├── * feat/auth-api (#43)\n",
                "└── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui (#44)\u{1b}[0m"
            )
        );
    }

    #[test]
    fn renders_hidden_tracked_current_branch_at_bottom() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "feat/billing".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![],
            current_branch_name: Some("feat/auth-ui".into()),
            is_current_visible: false,
            current_branch_suffix: None,
        });

        assert_eq!(
            rendered,
            concat!(
                "* feat/billing\n\n",
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m"
            )
        );
    }

    #[test]
    fn renders_hidden_orphaned_current_branch_at_bottom() {
        let rendered = render_stack_tree(&TreeView {
            root_label: Some(TreeLabel {
                branch_name: "main".into(),
                is_current: false,
                pull_request_number: None,
            }),
            roots: vec![TreeNode {
                branch_name: "feat/tracked".into(),
                is_current: false,
                pull_request_number: None,
                children: vec![],
            }],
            current_branch_name: Some("feat/untracked".into()),
            is_current_visible: false,
            current_branch_suffix: Some("(orphaned)".into()),
        });

        assert_eq!(
            rendered,
            concat!(
                "* main\n",
                "└── * feat/tracked\n\n",
                "\u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/untracked (orphaned)\u{1b}[0m"
            )
        );
    }
}
