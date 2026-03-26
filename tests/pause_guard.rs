mod support;

use std::path::Path;

use serde_json::Value;

use support::{
    active_rebase_head_name, dig, load_operation_json, pause_commit_restack, with_temp_repo,
};

fn assert_command_rejected_while_commit_is_paused(
    repo: &Path,
    args: &[&str],
    command_name: &str,
    operation_before: &Value,
) {
    let rebase_head_before = active_rebase_head_name(repo);
    let output = dig(repo, args);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let expected_message = format!(
        "dig {command_name} cannot run while a dig commit operation is paused; run 'dig sync --continue'"
    );

    assert!(
        !output.status.success(),
        "expected dig {command_name} to fail while commit restack is paused"
    );
    assert!(
        stdout.is_empty(),
        "expected no stdout while blocking {command_name}\nstdout:\n{stdout}"
    );
    assert!(
        stderr.contains(&expected_message),
        "missing paused-operation message for {command_name}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("detached HEAD"),
        "unexpected detached-HEAD error while blocking {command_name}\nstderr:\n{stderr}"
    );
    assert_eq!(
        load_operation_json(repo),
        Some(operation_before.clone()),
        "pending operation changed while blocking {command_name}"
    );
    assert_eq!(
        active_rebase_head_name(repo),
        rebase_head_before,
        "active rebase head changed while blocking {command_name}"
    );
    assert!(
        repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists(),
        "expected git rebase state to remain active while blocking {command_name}"
    );
}

#[test]
fn commit_rejects_immediately_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(
            repo,
            &["commit", "-m", "feat: extra follow-up"],
            "commit",
            &operation,
        );
    });
}

#[test]
fn adopt_rejects_immediately_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(
            repo,
            &["adopt", "-p", "main"],
            "adopt",
            &operation,
        );
    });
}

#[test]
fn merge_rejects_before_rendering_plan_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(
            repo,
            &["merge", "feat/auth"],
            "merge",
            &operation,
        );
    });
}

#[test]
fn clean_rejects_before_rendering_plan_or_prompt_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(repo, &["clean"], "clean", &operation);
    });
}

#[test]
fn orphan_rejects_immediately_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(repo, &["orphan"], "orphan", &operation);
    });
}

#[test]
fn reparent_rejects_immediately_while_commit_restack_is_paused() {
    with_temp_repo("dig-pause-guard", |repo| {
        let operation = pause_commit_restack(repo);

        assert_command_rejected_while_commit_is_paused(
            repo,
            &["reparent", "-p", "main"],
            "reparent",
            &operation,
        );
    });
}
