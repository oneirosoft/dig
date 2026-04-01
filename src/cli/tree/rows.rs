use crate::core::tree::{TreeLabel, TreeNode, TreeView};
use crate::ui::markers;
use crate::ui::palette::Accent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StackTreeRow {
    pub branch_name: String,
    pub is_current: bool,
    pub pull_request_number: Option<u64>,
    pub line: String,
}

pub(crate) fn stack_tree_rows(view: &TreeView) -> Vec<StackTreeRow> {
    let mut rows = Vec::new();

    if let Some(root_label) = &view.root_label {
        rows.push(row_from_label(root_label));
    }

    append_tree_rows(&mut rows, &view.roots, "");

    rows
}

fn append_tree_rows(rows: &mut Vec<StackTreeRow>, nodes: &[TreeNode], prefix: &str) {
    for (index, node) in nodes.iter().enumerate() {
        let is_last = index + 1 == nodes.len();
        let connector = if is_last { "└──" } else { "├──" };
        rows.push(row_from_node(node, format!("{prefix}{connector} ")));

        let child_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        append_tree_rows(rows, &node.children, &child_prefix);
    }
}

fn row_from_label(label: &TreeLabel) -> StackTreeRow {
    StackTreeRow {
        branch_name: label.branch_name.clone(),
        is_current: label.is_current,
        pull_request_number: label.pull_request_number,
        line: format_branch_label(
            "",
            &label.branch_name,
            label.is_current,
            label.pull_request_number,
        ),
    }
}

fn row_from_node(node: &TreeNode, prefix: String) -> StackTreeRow {
    StackTreeRow {
        branch_name: node.branch_name.clone(),
        is_current: node.is_current,
        pull_request_number: node.pull_request_number,
        line: format_branch_label(
            &prefix,
            &node.branch_name,
            node.is_current,
            node.pull_request_number,
        ),
    }
}

fn format_branch_text(branch_name: &str, pull_request_number: Option<u64>) -> String {
    match pull_request_number {
        Some(number) => format!("{branch_name} (#{number})"),
        None => branch_name.to_string(),
    }
}

fn format_branch_label(
    prefix: &str,
    branch_name: &str,
    is_current: bool,
    pull_request_number: Option<u64>,
) -> String {
    let label = format_branch_text(branch_name, pull_request_number);

    if is_current {
        format!(
            "{prefix}{} {}",
            Accent::BranchRef.paint_ansi(markers::CURRENT_BRANCH),
            Accent::BranchRef.paint_ansi(&label)
        )
    } else {
        format!("{prefix}{} {label}", markers::NON_CURRENT_BRANCH)
    }
}

#[cfg(test)]
mod tests {
    use super::{StackTreeRow, stack_tree_rows};
    use crate::core::tree::{TreeLabel, TreeNode, TreeView};

    #[test]
    fn builds_rows_in_tree_order_with_connectors_and_metadata() {
        assert_eq!(
            stack_tree_rows(&TreeView {
                root_label: Some(TreeLabel {
                    branch_name: "main".into(),
                    is_current: false,
                    pull_request_number: None,
                }),
                roots: vec![
                    TreeNode {
                        branch_name: "feat/auth".into(),
                        is_current: false,
                        pull_request_number: Some(101),
                        children: vec![TreeNode {
                            branch_name: "feat/auth-ui".into(),
                            is_current: true,
                            pull_request_number: None,
                            children: vec![],
                        }],
                    },
                    TreeNode {
                        branch_name: "feat/billing".into(),
                        is_current: false,
                        pull_request_number: None,
                        children: vec![],
                    },
                ],
                current_branch_name: Some("feat/auth-ui".into()),
                is_current_visible: true,
                current_branch_suffix: None,
            }),
            vec![
                StackTreeRow {
                    branch_name: "main".into(),
                    is_current: false,
                    pull_request_number: None,
                    line: "* main".into(),
                },
                StackTreeRow {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: Some(101),
                    line: "├── * feat/auth (#101)".into(),
                },
                StackTreeRow {
                    branch_name: "feat/auth-ui".into(),
                    is_current: true,
                    pull_request_number: None,
                    line: "│   └── \u{1b}[32m✓\u{1b}[0m \u{1b}[32mfeat/auth-ui\u{1b}[0m".into(),
                },
                StackTreeRow {
                    branch_name: "feat/billing".into(),
                    is_current: false,
                    pull_request_number: None,
                    line: "└── * feat/billing".into(),
                },
            ]
        );
    }

    #[test]
    fn omits_root_row_when_view_is_filtered_to_main_branch() {
        assert_eq!(
            stack_tree_rows(&TreeView {
                root_label: None,
                roots: vec![TreeNode {
                    branch_name: "feat/auth".into(),
                    is_current: false,
                    pull_request_number: None,
                    children: vec![],
                }],
                current_branch_name: Some("main".into()),
                is_current_visible: false,
                current_branch_suffix: None,
            }),
            vec![StackTreeRow {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: None,
                line: "└── * feat/auth".into(),
            }]
        );
    }
}
