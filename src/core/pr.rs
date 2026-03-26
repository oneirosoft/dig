use std::collections::HashMap;
use std::io;
use std::process::ExitStatus;

use crate::core::gh::{self, CreatePullRequestOptions, PullRequestDetails, PullRequestSummary};
use crate::core::git;
use crate::core::graph::BranchGraph;
use crate::core::store::{
    BranchPullRequestTrackedSource, TrackedPullRequest, open_initialized,
    record_branch_pull_request_tracked,
};
use crate::core::tree::{self, TreeOptions};
use crate::core::workflow;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrOptions {
    pub title: Option<String>,
    pub body: Option<String>,
    pub draft: bool,
    pub push_if_needed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrOutcomeKind {
    AlreadyTracked,
    Created,
    Adopted,
}

#[derive(Debug)]
pub struct PrOutcome {
    pub status: ExitStatus,
    pub kind: PrOutcomeKind,
    pub branch_name: String,
    pub base_branch_name: String,
    pub pull_request: TrackedPullRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPullRequestListNode {
    pub pull_request: PullRequestDetails,
    pub children: Vec<TrackedPullRequestListNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPullRequestListView {
    pub root_label: Option<String>,
    pub roots: Vec<TrackedPullRequestListNode>,
}

#[derive(Debug)]
pub struct PrListOutcome {
    pub status: ExitStatus,
    pub view: TrackedPullRequestListView,
    pub pull_requests: Vec<PullRequestDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrTrackingAction {
    Create,
    Adopt(PullRequestSummary),
}

pub fn run(options: &PrOptions) -> io::Result<PrOutcome> {
    let mut session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_no_pending_operation(&session.paths, "pr")?;
    git::ensure_no_in_progress_operations(&session.repo, "pr")?;

    let branch_name = git::current_branch_name_if_any()?.ok_or_else(|| {
        io::Error::other("dig pr requires a named branch; detached HEAD is not supported")
    })?;
    let node = session
        .state
        .find_branch_by_name(&branch_name)
        .cloned()
        .ok_or_else(|| {
            io::Error::other(format!("branch '{}' is not tracked by dig", branch_name))
        })?;

    let base_branch_name = BranchGraph::new(&session.state)
        .parent_branch_name(&node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent branch for '{}' was not found",
                branch_name
            ))
        })?;

    if let Some(pull_request) = node.pull_request.clone() {
        return Ok(PrOutcome {
            status: git::success_status()?,
            kind: PrOutcomeKind::AlreadyTracked,
            branch_name,
            base_branch_name,
            pull_request,
        });
    }

    let open_pull_requests = gh::list_open_pull_requests_for_head(&branch_name)?;
    match resolve_tracking_action(&branch_name, &base_branch_name, &open_pull_requests)? {
        PrTrackingAction::Create => {
            if let Some(push_target) = git::branch_push_target_if_needed(&branch_name)? {
                if !options.push_if_needed {
                    return Err(io::Error::other(format!(
                        "branch '{}' is not pushed to '{}'",
                        push_target.branch_name, push_target.remote_name
                    )));
                }

                let push_output = git::push_branch_to_remote(&push_target)?;
                if !push_output.status.success() {
                    let combined_output = push_output.combined_output();
                    return Err(io::Error::other(if combined_output.is_empty() {
                        format!(
                            "git push to '{}' failed for branch '{}'",
                            push_target.remote_name, push_target.branch_name
                        )
                    } else {
                        format!(
                            "git push to '{}' failed for branch '{}': {}",
                            push_target.remote_name, push_target.branch_name, combined_output
                        )
                    }));
                }
            }

            let created_pull_request = gh::create_pull_request(&CreatePullRequestOptions {
                base_branch_name: base_branch_name.clone(),
                title: options.title.clone(),
                body: options.body.clone(),
                draft: options.draft,
            })?;
            let pull_request = TrackedPullRequest {
                number: created_pull_request.number,
            };

            record_branch_pull_request_tracked(
                &mut session,
                node.id,
                node.branch_name.clone(),
                pull_request.clone(),
                BranchPullRequestTrackedSource::Created,
            )?;

            Ok(PrOutcome {
                status: git::success_status()?,
                kind: PrOutcomeKind::Created,
                branch_name,
                base_branch_name,
                pull_request,
            })
        }
        PrTrackingAction::Adopt(existing_pull_request) => {
            let pull_request = TrackedPullRequest {
                number: existing_pull_request.number,
            };

            record_branch_pull_request_tracked(
                &mut session,
                node.id,
                node.branch_name.clone(),
                pull_request.clone(),
                BranchPullRequestTrackedSource::Adopted,
            )?;

            Ok(PrOutcome {
                status: git::success_status()?,
                kind: PrOutcomeKind::Adopted,
                branch_name,
                base_branch_name,
                pull_request,
            })
        }
    }
}

pub fn current_branch_push_target_for_create() -> io::Result<Option<git::BranchPushTarget>> {
    let session = open_initialized("dig is not initialized; run 'dig init' first")?;
    workflow::ensure_no_pending_operation(&session.paths, "pr")?;
    git::ensure_no_in_progress_operations(&session.repo, "pr")?;

    let branch_name = git::current_branch_name_if_any()?.ok_or_else(|| {
        io::Error::other("dig pr requires a named branch; detached HEAD is not supported")
    })?;
    let node = session
        .state
        .find_branch_by_name(&branch_name)
        .ok_or_else(|| {
            io::Error::other(format!("branch '{}' is not tracked by dig", branch_name))
        })?;

    if node.pull_request.is_some() {
        return Ok(None);
    }

    let base_branch_name = BranchGraph::new(&session.state)
        .parent_branch_name(node, &session.config.trunk_branch)
        .ok_or_else(|| {
            io::Error::other(format!(
                "tracked parent branch for '{}' was not found",
                branch_name
            ))
        })?;

    let open_pull_requests = gh::list_open_pull_requests_for_head(&branch_name)?;
    match resolve_tracking_action(&branch_name, &base_branch_name, &open_pull_requests)? {
        PrTrackingAction::Create => git::branch_push_target_if_needed(&branch_name),
        PrTrackingAction::Adopt(_) => Ok(None),
    }
}

pub fn list_open_tracked_pull_requests() -> io::Result<PrListOutcome> {
    open_initialized("dig is not initialized; run 'dig init' first")?;
    let open_pull_requests = gh::list_open_pull_requests()?;
    let pull_request_lookup = open_pull_requests
        .into_iter()
        .map(|pull_request| (pull_request.number, pull_request))
        .collect::<HashMap<_, _>>();
    let tree_outcome = tree::run(&TreeOptions::default())?;
    let roots = tree_outcome
        .view
        .roots
        .iter()
        .flat_map(|node| build_pull_request_list_nodes(node, &pull_request_lookup))
        .collect::<Vec<_>>();
    let mut ordered_pull_requests = Vec::new();
    collect_pull_requests_in_order(&roots, &mut ordered_pull_requests);

    Ok(PrListOutcome {
        status: tree_outcome.status,
        view: TrackedPullRequestListView {
            root_label: tree_outcome.view.root_label.map(|label| label.branch_name),
            roots,
        },
        pull_requests: ordered_pull_requests,
    })
}

pub fn open_current_pull_request_in_browser() -> io::Result<()> {
    let session = open_initialized("dig is not initialized; run 'dig init' first")?;
    let branch_name = git::current_branch_name_if_any()?.ok_or_else(|| {
        io::Error::other("dig pr requires a named branch; detached HEAD is not supported")
    })?;

    if let Some(pull_request) = session
        .state
        .find_branch_by_name(&branch_name)
        .and_then(|node| node.pull_request.as_ref())
    {
        return gh::open_pull_request_in_browser(pull_request.number);
    }

    gh::open_current_pull_request_in_browser()
}

pub fn open_pull_request_in_browser(number: u64) -> io::Result<()> {
    gh::open_pull_request_in_browser(number)
}

pub fn open_pull_requests_in_browser(pull_requests: &[PullRequestDetails]) -> io::Result<()> {
    for pull_request in pull_requests {
        gh::open_pull_request_in_browser(pull_request.number)?;
    }

    Ok(())
}

fn resolve_tracking_action(
    branch_name: &str,
    base_branch_name: &str,
    open_pull_requests: &[PullRequestSummary],
) -> io::Result<PrTrackingAction> {
    match open_pull_requests {
        [] => Ok(PrTrackingAction::Create),
        [pull_request] if pull_request.base_ref_name == base_branch_name => {
            Ok(PrTrackingAction::Adopt(pull_request.clone()))
        }
        [pull_request] => Err(io::Error::other(format!(
            "branch '{}' already has open pull request #{} into '{}', but dig expects base '{}'",
            branch_name, pull_request.number, pull_request.base_ref_name, base_branch_name
        ))),
        _ => Err(io::Error::other(format!(
            "branch '{}' has multiple open pull requests on GitHub; dig pr cannot choose automatically",
            branch_name
        ))),
    }
}

fn build_pull_request_list_nodes(
    node: &crate::core::tree::TreeNode,
    pull_request_lookup: &HashMap<u64, PullRequestDetails>,
) -> Vec<TrackedPullRequestListNode> {
    let children = node
        .children
        .iter()
        .flat_map(|child| build_pull_request_list_nodes(child, pull_request_lookup))
        .collect::<Vec<_>>();

    let Some(number) = node.pull_request_number else {
        return children;
    };
    let Some(pull_request) = pull_request_lookup.get(&number) else {
        return children;
    };

    vec![TrackedPullRequestListNode {
        pull_request: pull_request.clone(),
        children,
    }]
}

fn collect_pull_requests_in_order(
    nodes: &[TrackedPullRequestListNode],
    ordered_pull_requests: &mut Vec<PullRequestDetails>,
) {
    for node in nodes {
        ordered_pull_requests.push(node.pull_request.clone());
        collect_pull_requests_in_order(&node.children, ordered_pull_requests);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PrTrackingAction, TrackedPullRequestListNode, build_pull_request_list_nodes,
        collect_pull_requests_in_order, resolve_tracking_action,
    };
    use crate::core::gh::PullRequestDetails;
    use crate::core::gh::PullRequestSummary;
    use crate::core::tree::TreeNode;
    use std::collections::HashMap;

    #[test]
    fn resolves_create_when_no_open_pull_requests_exist() {
        let action = resolve_tracking_action("feat/auth", "main", &[]).unwrap();

        assert_eq!(action, PrTrackingAction::Create);
    }

    #[test]
    fn resolves_adopt_for_matching_open_pull_request() {
        let action = resolve_tracking_action(
            "feat/auth",
            "main",
            &[PullRequestSummary {
                number: 123,
                base_ref_name: "main".into(),
                url: "https://github.com/acme/dig/pull/123".into(),
            }],
        )
        .unwrap();

        assert_eq!(
            action,
            PrTrackingAction::Adopt(PullRequestSummary {
                number: 123,
                base_ref_name: "main".into(),
                url: "https://github.com/acme/dig/pull/123".into(),
            })
        );
    }

    #[test]
    fn rejects_open_pull_request_with_mismatched_base() {
        let error = resolve_tracking_action(
            "feat/auth",
            "main",
            &[PullRequestSummary {
                number: 123,
                base_ref_name: "develop".into(),
                url: "https://github.com/acme/dig/pull/123".into(),
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains("expects base 'main'"));
    }

    #[test]
    fn rejects_multiple_open_pull_requests() {
        let error = resolve_tracking_action(
            "feat/auth",
            "main",
            &[
                PullRequestSummary {
                    number: 123,
                    base_ref_name: "main".into(),
                    url: "https://github.com/acme/dig/pull/123".into(),
                },
                PullRequestSummary {
                    number: 124,
                    base_ref_name: "main".into(),
                    url: "https://github.com/acme/dig/pull/124".into(),
                },
            ],
        )
        .unwrap_err();

        assert!(error.to_string().contains("multiple open pull requests"));
    }

    #[test]
    fn builds_pull_request_list_nodes_by_collapsing_non_pr_branches() {
        let nodes = build_pull_request_list_nodes(
            &TreeNode {
                branch_name: "feat/auth".into(),
                is_current: false,
                pull_request_number: None,
                children: vec![TreeNode {
                    branch_name: "feat/auth-ui".into(),
                    is_current: false,
                    pull_request_number: Some(123),
                    children: vec![],
                }],
            },
            &HashMap::from([(
                123,
                PullRequestDetails {
                    number: 123,
                    title: "Auth UI".into(),
                    url: "https://github.com/acme/dig/pull/123".into(),
                },
            )]),
        );

        assert_eq!(
            nodes,
            vec![TrackedPullRequestListNode {
                pull_request: PullRequestDetails {
                    number: 123,
                    title: "Auth UI".into(),
                    url: "https://github.com/acme/dig/pull/123".into(),
                },
                children: vec![],
            }]
        );
    }

    #[test]
    fn collects_pull_requests_in_tree_order() {
        let mut ordered_pull_requests = Vec::new();
        collect_pull_requests_in_order(
            &[TrackedPullRequestListNode {
                pull_request: PullRequestDetails {
                    number: 123,
                    title: "Auth".into(),
                    url: "https://github.com/acme/dig/pull/123".into(),
                },
                children: vec![TrackedPullRequestListNode {
                    pull_request: PullRequestDetails {
                        number: 124,
                        title: "Auth UI".into(),
                        url: "https://github.com/acme/dig/pull/124".into(),
                    },
                    children: vec![],
                }],
            }],
            &mut ordered_pull_requests,
        );

        assert_eq!(
            ordered_pull_requests
                .iter()
                .map(|pr| pr.number)
                .collect::<Vec<_>>(),
            vec![123, 124]
        );
    }
}
