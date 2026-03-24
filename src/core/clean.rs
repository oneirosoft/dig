use std::collections::HashSet;
use std::io;
use std::process::ExitStatus;

use uuid::Uuid;

use crate::core::git::{self, CherryMarker, CommitMetadata};
use crate::core::restack::{self, RestackPreview};
use crate::core::store::types::DigState;
use crate::core::store::{
    append_event, dig_paths, load_config, load_state, now_unix_timestamp_secs, save_state,
    BranchArchiveReason, BranchArchivedEvent, BranchNode, BranchReparentedEvent, DigEvent,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanOptions {
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanPlan {
    pub trunk_branch: String,
    pub current_branch: String,
    pub requested_branch_name: Option<String>,
    pub candidates: Vec<CleanCandidate>,
    pub blocked: Vec<BlockedBranch>,
}

impl CleanPlan {
    pub fn targets_current_branch(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| candidate.branch_name == self.current_branch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanCandidate {
    pub node_id: Uuid,
    pub branch_name: String,
    pub parent_branch_name: String,
    pub reason: CleanReason,
    pub tree: CleanTreeNode,
    pub restack_plan: Vec<RestackPreview>,
    pub(crate) depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanTreeNode {
    pub branch_name: String,
    pub children: Vec<CleanTreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanReason {
    IntegratedIntoParent { parent_branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedBranch {
    pub branch_name: String,
    pub reason: CleanBlockReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanBlockReason {
    BranchNotTracked,
    BranchMissingLocally,
    ParentMissingLocally { parent_branch: String },
    ParentMissingFromDig,
    NotIntegrated { parent_branch: String },
    DescendantsMissingLocally { branch_names: Vec<String> },
}

#[derive(Debug)]
pub struct CleanApplyOutcome {
    pub status: ExitStatus,
    pub switched_to_trunk_from: Option<String>,
    pub restored_original_branch: Option<String>,
    pub deleted_branches: Vec<String>,
    pub restacked_branches: Vec<RestackPreview>,
    pub failure_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanEvent {
    SwitchingToTrunk { from_branch: String, to_branch: String },
    SwitchedToTrunk { from_branch: String, to_branch: String },
    RebaseStarted { branch_name: String, onto_branch: String },
    RebaseProgress {
        branch_name: String,
        onto_branch: String,
        current_commit: usize,
        total_commits: usize,
    },
    RebaseCompleted { branch_name: String, onto_branch: String },
    DeleteStarted { branch_name: String },
    DeleteCompleted { branch_name: String },
}

#[derive(Debug)]
enum BranchEvaluation {
    Cleanable(CleanCandidate),
    Blocked(BlockedBranch),
}

pub fn plan(options: &CleanOptions) -> io::Result<CleanPlan> {
    let repo = git::resolve_repo_context()?;
    let store_paths = dig_paths(&repo.git_dir);
    let config = load_config(&store_paths)?
        .ok_or_else(|| io::Error::other("dig is not initialized; run 'dig init' first"))?;
    let state = load_state(&store_paths)?;
    let current_branch = git::current_branch_name()?;
    let requested_branch_name = options
        .branch_name
        .as_deref()
        .map(str::trim)
        .filter(|branch_name| !branch_name.is_empty())
        .map(str::to_string);

    match &requested_branch_name {
        Some(branch_name) => plan_for_requested_branch(&state, &config.trunk_branch, &current_branch, branch_name),
        None => plan_for_all_branches(&state, &config.trunk_branch, &current_branch),
    }
}

pub fn apply(plan: &CleanPlan) -> io::Result<CleanApplyOutcome> {
    apply_with_reporter(plan, &mut |_| Ok(()))
}

pub fn apply_with_reporter<F>(plan: &CleanPlan, reporter: &mut F) -> io::Result<CleanApplyOutcome>
where
    F: FnMut(CleanEvent) -> io::Result<()>,
{
    if plan.candidates.is_empty() {
        return Ok(CleanApplyOutcome {
            status: git::success_status()?,
            switched_to_trunk_from: None,
            restored_original_branch: None,
            deleted_branches: Vec::new(),
            restacked_branches: Vec::new(),
            failure_output: None,
        });
    }

    let repo = git::resolve_repo_context()?;
    git::ensure_clean_worktree()?;
    git::ensure_no_in_progress_operations(&repo)?;

    let store_paths = dig_paths(&repo.git_dir);
    let config = load_config(&store_paths)?
        .ok_or_else(|| io::Error::other("dig is not initialized; run 'dig init' first"))?;
    let mut state = load_state(&store_paths)?;
    let current_branch = git::current_branch_name()?;
    let original_branch = current_branch.clone();

    let mut switched_to_trunk_from = None;
    if plan.targets_current_branch() && current_branch != config.trunk_branch {
        reporter(CleanEvent::SwitchingToTrunk {
            from_branch: current_branch.clone(),
            to_branch: config.trunk_branch.clone(),
        })?;
        let status = git::switch_branch(&config.trunk_branch)?;
        if !status.success() {
            return Ok(CleanApplyOutcome {
                status,
                switched_to_trunk_from: None,
                restored_original_branch: None,
                deleted_branches: Vec::new(),
                restacked_branches: Vec::new(),
                failure_output: None,
            });
        }

        reporter(CleanEvent::SwitchedToTrunk {
            from_branch: current_branch.clone(),
            to_branch: config.trunk_branch.clone(),
        })?;
        switched_to_trunk_from = Some(current_branch);
    }

    let mut deleted_branches = Vec::new();
    let mut restacked_branches = Vec::new();
    let mut last_status = git::success_status()?;

    for candidate in &plan.candidates {
        let Some(node) = state.find_branch_by_id(candidate.node_id).cloned() else {
            continue;
        };

        let Some(parent_branch_name) = state.resolve_parent_branch_name(&node, &config.trunk_branch) else {
            return Err(io::Error::other(format!(
                "tracked parent for '{}' is missing from dig",
                node.branch_name
            )));
        };

        let restack_actions = restack::plan_after_branch_removal(
            &state,
            node.id,
            &node.branch_name,
            &parent_branch_name,
            &node.parent,
        )?;

        for action in &restack_actions {
            reporter(CleanEvent::RebaseStarted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base_branch_name.clone(),
            })?;
            let outcome = restack::apply_action(&mut state, action, |progress| {
                reporter(CleanEvent::RebaseProgress {
                    branch_name: action.branch_name.clone(),
                    onto_branch: action.new_base_branch_name.clone(),
                    current_commit: progress.current,
                    total_commits: progress.total,
                })
            })?;
            if !outcome.status.success() {
                return Ok(CleanApplyOutcome {
                    status: outcome.status,
                    switched_to_trunk_from,
                    restored_original_branch: None,
                    deleted_branches,
                    restacked_branches,
                    failure_output: Some(outcome.stderr),
                });
            }

            reporter(CleanEvent::RebaseCompleted {
                branch_name: action.branch_name.clone(),
                onto_branch: action.new_base_branch_name.clone(),
            })?;

            if let Some(parent_change) = outcome.parent_change {
                save_state(&store_paths, &state)?;
                append_event(
                    &store_paths,
                    &DigEvent::BranchReparented(BranchReparentedEvent {
                        occurred_at_unix_secs: now_unix_timestamp_secs(),
                        branch_id: parent_change.branch_id,
                        branch_name: parent_change.branch_name,
                        old_parent: parent_change.old_parent,
                        new_parent: parent_change.new_parent,
                        old_base_ref: parent_change.old_base_ref,
                        new_base_ref: parent_change.new_base_ref,
                    }),
                )?;
            }

            restacked_branches.push(RestackPreview {
                branch_name: outcome.branch_name,
                onto_branch: outcome.onto_branch,
                parent_changed: action.new_parent.is_some(),
            });
        }

        reporter(CleanEvent::DeleteStarted {
            branch_name: node.branch_name.clone(),
        })?;
        let status = git::delete_branch_force(&node.branch_name)?;
        if !status.success() {
            return Ok(CleanApplyOutcome {
                status,
                switched_to_trunk_from,
                restored_original_branch: None,
                deleted_branches,
                restacked_branches,
                failure_output: None,
            });
        }

        state.archive_branch(node.id)?;
        save_state(&store_paths, &state)?;
        append_event(
            &store_paths,
            &DigEvent::BranchArchived(BranchArchivedEvent {
                occurred_at_unix_secs: now_unix_timestamp_secs(),
                branch_id: node.id,
                branch_name: node.branch_name.clone(),
                reason: BranchArchiveReason::IntegratedIntoParent {
                    parent_branch: parent_branch_name.clone(),
                },
            }),
        )?;

        reporter(CleanEvent::DeleteCompleted {
            branch_name: node.branch_name.clone(),
        })?;
        deleted_branches.push(node.branch_name);
        last_status = status;
    }

    let mut restored_original_branch = None;
    if git::branch_exists(&original_branch)? {
        let current_branch = git::current_branch_name_if_any()?;
        if current_branch.as_deref() != Some(original_branch.as_str()) {
            let status = git::switch_branch(&original_branch)?;
            if !status.success() {
                return Ok(CleanApplyOutcome {
                    status,
                    switched_to_trunk_from,
                    restored_original_branch: None,
                    deleted_branches,
                    restacked_branches,
                    failure_output: None,
                });
            }

            restored_original_branch = Some(original_branch);
            last_status = status;
        }
    }

    Ok(CleanApplyOutcome {
        status: last_status,
        switched_to_trunk_from,
        restored_original_branch,
        deleted_branches,
        restacked_branches,
        failure_output: None,
    })
}

fn plan_for_requested_branch(
    state: &DigState,
    trunk_branch: &str,
    current_branch: &str,
    branch_name: &str,
) -> io::Result<CleanPlan> {
    let evaluation = match state.find_branch_by_name(branch_name) {
        Some(node) => evaluate_branch(state, trunk_branch, node)?,
        None => BranchEvaluation::Blocked(BlockedBranch {
            branch_name: branch_name.to_string(),
            reason: CleanBlockReason::BranchNotTracked,
        }),
    };

    let (candidates, blocked) = match evaluation {
        BranchEvaluation::Cleanable(candidate) => (vec![candidate], Vec::new()),
        BranchEvaluation::Blocked(blocked) => (Vec::new(), vec![blocked]),
    };

    Ok(CleanPlan {
        trunk_branch: trunk_branch.to_string(),
        current_branch: current_branch.to_string(),
        requested_branch_name: Some(branch_name.to_string()),
        candidates,
        blocked,
    })
}

fn plan_for_all_branches(
    state: &DigState,
    trunk_branch: &str,
    current_branch: &str,
) -> io::Result<CleanPlan> {
    let mut cleanable = Vec::new();
    let mut blocked = Vec::new();

    for node in state.nodes.iter().filter(|node| !node.archived) {
        match evaluate_branch(state, trunk_branch, node)? {
            BranchEvaluation::Cleanable(candidate) => cleanable.push(candidate),
            BranchEvaluation::Blocked(blocked_branch) => blocked.push(blocked_branch),
        }
    }

    let cleanable_ids = cleanable
        .iter()
        .map(|candidate| candidate.node_id)
        .collect::<HashSet<_>>();

    let mut candidates = cleanable
        .into_iter()
        .filter(|candidate| {
            !state
                .active_descendant_ids(candidate.node_id)
                .iter()
                .any(|descendant_id| cleanable_ids.contains(descendant_id))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .depth
            .cmp(&left.depth)
            .then_with(|| left.branch_name.cmp(&right.branch_name))
    });

    Ok(CleanPlan {
        trunk_branch: trunk_branch.to_string(),
        current_branch: current_branch.to_string(),
        requested_branch_name: None,
        candidates,
        blocked,
    })
}

fn evaluate_branch(
    state: &DigState,
    trunk_branch: &str,
    node: &BranchNode,
) -> io::Result<BranchEvaluation> {
    if !git::branch_exists(&node.branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::BranchMissingLocally,
        }));
    }

    let Some(parent_branch_name) = state.resolve_parent_branch_name(node, trunk_branch) else {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::ParentMissingFromDig,
        }));
    };

    if !git::branch_exists(&parent_branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::ParentMissingLocally {
                parent_branch: parent_branch_name,
            },
        }));
    }

    let missing_descendants = missing_local_descendants(state, node.id)?;
    if !missing_descendants.is_empty() {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::DescendantsMissingLocally {
                branch_names: missing_descendants,
            },
        }));
    }

    if !branch_is_integrated(&parent_branch_name, &node.branch_name)? {
        return Ok(BranchEvaluation::Blocked(BlockedBranch {
            branch_name: node.branch_name.clone(),
            reason: CleanBlockReason::NotIntegrated {
                parent_branch: parent_branch_name,
            },
        }));
    }

    let restack_actions = restack::plan_after_branch_removal(
        state,
        node.id,
        &node.branch_name,
        &parent_branch_name,
        &node.parent,
    )?;

    Ok(BranchEvaluation::Cleanable(CleanCandidate {
        node_id: node.id,
        branch_name: node.branch_name.clone(),
        parent_branch_name: parent_branch_name.clone(),
        reason: CleanReason::IntegratedIntoParent {
            parent_branch: parent_branch_name,
        },
        tree: build_clean_tree(state, node.id)?,
        restack_plan: restack::previews_for_actions(&restack_actions),
        depth: state.branch_depth(node.id),
    }))
}

fn build_clean_tree(state: &DigState, node_id: Uuid) -> io::Result<CleanTreeNode> {
    let node = state.find_branch_by_id(node_id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "tracked branch was not found")
    })?;

    let mut children = Vec::new();
    for child_id in state.active_children_ids(node_id) {
        children.push(build_clean_tree(state, child_id)?);
    }

    Ok(CleanTreeNode {
        branch_name: node.branch_name.clone(),
        children,
    })
}

fn missing_local_descendants(state: &DigState, node_id: Uuid) -> io::Result<Vec<String>> {
    let mut missing = Vec::new();

    for descendant_id in state.active_descendant_ids(node_id) {
        let Some(descendant) = state.find_branch_by_id(descendant_id) else {
            continue;
        };

        if !git::branch_exists(&descendant.branch_name)? {
            missing.push(descendant.branch_name.clone());
        }
    }

    Ok(missing)
}

fn branch_is_integrated(parent_branch_name: &str, branch_name: &str) -> io::Result<bool> {
    if branch_is_integrated_by_cherry(parent_branch_name, branch_name)? {
        return Ok(true);
    }

    branch_is_integrated_by_squash_message(parent_branch_name, branch_name)
}

fn branch_is_integrated_by_cherry(parent_branch_name: &str, branch_name: &str) -> io::Result<bool> {
    let markers = git::cherry_markers(parent_branch_name, branch_name)?;

    Ok(markers
        .into_iter()
        .all(|marker| matches!(marker, CherryMarker::Equivalent)))
}

fn branch_is_integrated_by_squash_message(parent_branch_name: &str, branch_name: &str) -> io::Result<bool> {
    let merge_base = git::merge_base(parent_branch_name, branch_name)?;
    let branch_commits = git::commit_metadata_in_range(&format!("{merge_base}..{branch_name}"))?;

    if branch_commits.is_empty() {
        return Ok(true);
    }

    let parent_commits =
        git::commit_metadata_in_range(&format!("{merge_base}..{parent_branch_name}"))?;

    Ok(parent_commits
        .iter()
        .any(|parent_commit| parent_commit_mentions_all_branch_commits(parent_commit, &branch_commits)))
}

fn parent_commit_mentions_all_branch_commits(
    parent_commit: &CommitMetadata,
    branch_commits: &[CommitMetadata],
) -> bool {
    if branch_commits
        .iter()
        .all(|branch_commit| parent_commit.body.contains(&branch_commit.sha))
    {
        return true;
    }

    let message_lines = parent_commit
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<HashSet<_>>();

    branch_commits
        .iter()
        .all(|branch_commit| message_lines.contains(branch_commit.subject.as_str()))
}

#[cfg(test)]
mod tests {
    use super::{
        apply, parent_commit_mentions_all_branch_commits, plan, BlockedBranch, CleanBlockReason,
        CleanOptions, CleanReason,
    };
    use crate::core::branch::{self, BranchOptions};
    use crate::core::git::{self, CommitMetadata};
    use crate::core::store::{dig_paths, load_state, ParentRef};
    use std::env;
    use std::fs;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};
    use uuid::Uuid;

    #[test]
    fn reports_non_integrated_branch_reason() {
        let blocked = BlockedBranch {
            branch_name: "feat/auth".into(),
            reason: CleanBlockReason::NotIntegrated {
                parent_branch: "main".into(),
            },
        };

        assert_eq!(
            blocked.reason,
            CleanBlockReason::NotIntegrated {
                parent_branch: "main".into()
            }
        );
    }

    #[test]
    fn tracks_integrated_clean_reason() {
        let reason = CleanReason::IntegratedIntoParent {
            parent_branch: "main".into(),
        };

        assert_eq!(
            reason,
            CleanReason::IntegratedIntoParent {
                parent_branch: "main".into()
            }
        );
    }

    #[test]
    fn detects_squash_commit_message_that_mentions_branch_commits() {
        assert!(parent_commit_mentions_all_branch_commits(
            &CommitMetadata {
                sha: "parent".into(),
                subject: "feat: stacked branch creation".into(),
                body: concat!(
                    "feat: stacked branch creation\n\n",
                    "commit 1de9e06d1174402332fdd5343a387249b0a5ef66\n",
                    "    feat: new parent flag to specify the parent mannualy of a branch\n\n",
                    "commit 2099fdf424816e61eceff1a98db2d00fee0f76ac\n",
                    "    feat: stacked-branches\n"
                )
                .into(),
            },
            &[
                CommitMetadata {
                    sha: "2099fdf424816e61eceff1a98db2d00fee0f76ac".into(),
                    subject: "feat: stacked-branches".into(),
                    body: String::new(),
                },
                CommitMetadata {
                    sha: "1de9e06d1174402332fdd5343a387249b0a5ef66".into(),
                    subject: "feat: new parent flag to specify the parent mannualy of a branch".into(),
                    body: String::new(),
                },
            ]
        ));
    }

    #[test]
    fn cleans_squash_merged_parent_and_restacks_descendants() {
        with_temp_repo(|repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            append_file(repo, "auth.txt", "auth second line\n", "feat: auth follow-up");
            create_tracked_branch("feat/auth-api");
            commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");
            create_tracked_branch("feat/auth-api-tests");
            commit_file(repo, "auth-api-tests.txt", "tests\n", "feat: auth api tests");

            squash_merge_branch_with_commit_listing(repo, "main", "feat/auth", "feat: merge auth");
            git_ok(repo, &["checkout", "feat/auth"]);

            let plan = plan(&CleanOptions {
                branch_name: Some("feat/auth".into()),
            })
            .unwrap();

            assert_eq!(plan.candidates.len(), 1);
            assert_eq!(plan.candidates[0].branch_name, "feat/auth");
            assert_eq!(
                plan.candidates[0]
                    .restack_plan
                    .iter()
                    .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                    .collect::<Vec<_>>(),
                vec![
                    "feat/auth-api->main".to_string(),
                    "feat/auth-api-tests->feat/auth-api".to_string(),
                ]
            );

            let outcome = apply(&plan).unwrap();

            assert!(outcome.status.success());
            assert_eq!(outcome.switched_to_trunk_from.as_deref(), Some("feat/auth"));
            assert_eq!(outcome.restored_original_branch, None);
            assert_eq!(outcome.deleted_branches, vec!["feat/auth".to_string()]);
            assert_eq!(
                outcome
                    .restacked_branches
                    .iter()
                    .map(|step| format!("{}->{}", step.branch_name, step.onto_branch))
                    .collect::<Vec<_>>(),
                vec![
                    "feat/auth-api->main".to_string(),
                    "feat/auth-api-tests->feat/auth-api".to_string(),
                ]
            );

            assert!(!git::branch_exists("feat/auth").unwrap());
            assert!(git::branch_exists("feat/auth-api").unwrap());
            assert!(git::branch_exists("feat/auth-api-tests").unwrap());

            let repo_context = git::resolve_repo_context().unwrap();
            let state = load_state(&dig_paths(&repo_context.git_dir)).unwrap();
            let restacked_child = state.find_branch_by_name("feat/auth-api").unwrap();
            let grandchild = state.find_branch_by_name("feat/auth-api-tests").unwrap();

            assert_eq!(restacked_child.parent, ParentRef::Trunk);
            assert_eq!(restacked_child.base_ref, "main");
            assert_eq!(
                grandchild.parent,
                ParentRef::Branch {
                    node_id: restacked_child.id
                }
            );
            assert_eq!(grandchild.base_ref, "feat/auth-api");
            assert!(
                state
                    .nodes
                    .iter()
                    .any(|node| node.branch_name == "feat/auth" && node.archived)
            );
        });
    }

    #[test]
    fn returns_to_original_branch_after_cleaning_from_another_checkout() {
        with_temp_repo(|repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-api");
            commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

            squash_merge_branch_with_commit_listing(repo, "main", "feat/auth", "feat: merge auth");
            git_ok(repo, &["checkout", "main"]);

            let plan = plan(&CleanOptions {
                branch_name: Some("feat/auth".into()),
            })
            .unwrap();

            let outcome = apply(&plan).unwrap();

            assert!(outcome.status.success());
            assert_eq!(outcome.switched_to_trunk_from, None);
            assert_eq!(outcome.restored_original_branch.as_deref(), Some("main"));
            assert_eq!(git::current_branch_name().unwrap(), "main");
            assert!(!git::branch_exists("feat/auth").unwrap());
        });
    }

    #[test]
    fn full_clean_plan_only_lists_deepest_cleanable_branches() {
        with_temp_repo(|repo| {
            initialize_main_repo(repo);
            create_tracked_branch("feat/auth");
            commit_file(repo, "auth.txt", "auth\n", "feat: auth");
            create_tracked_branch("feat/auth-api");
            commit_file(repo, "auth-api.txt", "api\n", "feat: auth api");

            git_ok(repo, &["checkout", "feat/auth"]);
            git_ok(repo, &["merge", "--squash", "feat/auth-api"]);
            git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth api"]);
            git_ok(repo, &["checkout", "main"]);
            git_ok(repo, &["merge", "--squash", "feat/auth"]);
            git_ok(repo, &["commit", "--quiet", "-m", "feat: merge auth"]);

            let plan = plan(&CleanOptions::default()).unwrap();

            assert_eq!(
                plan.candidates
                    .iter()
                    .map(|candidate| candidate.branch_name.clone())
                    .collect::<Vec<_>>(),
                vec!["feat/auth-api".to_string()]
            );
        });
    }

    fn with_temp_repo(test: impl FnOnce(&Path)) {
        static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        let guard = CWD_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let original_dir = env::current_dir().unwrap();
        let repo_dir = env::temp_dir().join(format!("dig-clean-{}", Uuid::new_v4()));
        fs::create_dir_all(&repo_dir).unwrap();

        let result = catch_unwind(AssertUnwindSafe(|| {
            env::set_current_dir(&repo_dir).unwrap();
            test(&repo_dir);
        }));

        env::set_current_dir(original_dir).unwrap();
        fs::remove_dir_all(&repo_dir).unwrap();
        drop(guard);

        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    fn initialize_main_repo(repo: &Path) {
        git_ok(repo, &["init", "--quiet"]);
        git_ok(repo, &["checkout", "-b", "main"]);
        git_ok(repo, &["config", "user.name", "Dig Test"]);
        git_ok(repo, &["config", "user.email", "dig@example.com"]);
        git_ok(repo, &["config", "commit.gpgsign", "false"]);
        commit_file(repo, "README.md", "root\n", "chore: init");
    }

    fn create_tracked_branch(branch_name: &str) {
        branch::run(&BranchOptions {
            name: branch_name.into(),
            parent_branch_name: None,
        })
        .unwrap();
    }

    fn commit_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
        fs::write(repo.join(file_name), contents).unwrap();
        git_ok(repo, &["add", file_name]);
        git_ok(repo, &["-c", "commit.gpgsign=false", "commit", "--quiet", "-m", message]);
    }

    fn append_file(repo: &Path, file_name: &str, contents: &str, message: &str) {
        let path = repo.join(file_name);
        let mut existing = fs::read_to_string(&path).unwrap();
        existing.push_str(contents);
        fs::write(&path, existing).unwrap();
        git_ok(repo, &["add", file_name]);
        git_ok(repo, &["-c", "commit.gpgsign=false", "commit", "--quiet", "-m", message]);
    }

    fn squash_merge_branch_with_commit_listing(
        repo: &Path,
        target_branch: &str,
        source_branch: &str,
        subject: &str,
    ) {
        let commits = git_output(
            repo,
            &["log", "--reverse", "--format=%H%x1f%s", &format!("{target_branch}..{source_branch}")],
        );

        git_ok(repo, &["checkout", target_branch]);
        git_ok(repo, &["merge", "--squash", source_branch]);

        let mut message = String::from(subject);
        message.push_str("\n\n");
        for record in commits.lines().filter(|line| !line.trim().is_empty()) {
            let (sha, commit_subject) = record.split_once('\u{1f}').unwrap();
            message.push_str(&format!("commit {sha}\n    {commit_subject}\n\n"));
        }

        let message_path = repo.join("SQUASH_MSG");
        fs::write(&message_path, message).unwrap();
        git_ok(
            repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-F",
                message_path.to_str().unwrap(),
            ],
        );
        fs::remove_file(message_path).unwrap();
    }

    fn git_ok(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo)
            .args(args)
            .status()
            .unwrap();

        assert!(status.success(), "git {:?} failed", args);
    }

    fn git_output(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();

        assert!(output.status.success(), "git {:?} failed", args);

        String::from_utf8(output.stdout).unwrap()
    }
}
