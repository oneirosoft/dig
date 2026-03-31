mod support;

use support::{
    commit_file, dgr_ok, git_ok, git_stdout, initialize_main_repo, strip_ansi, with_temp_repo,
};

#[test]
fn tree_branch_main_shows_checked_out_trunk_below_filtered_tree() {
    with_temp_repo("dgr-tree-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);

        let output = dgr_ok(repo, &["tree", "--branch", "main"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert_eq!(stdout.trim_end(), "└── * feat/auth\n\n✓ main");
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "main");
    });
}

#[test]
fn tree_branch_shows_hidden_checked_out_branch_below_selected_stack() {
    with_temp_repo("dgr-tree-cli", |repo| {
        initialize_main_repo(repo);
        dgr_ok(repo, &["init"]);
        dgr_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        dgr_ok(repo, &["branch", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "main"]);
        dgr_ok(repo, &["branch", "feat/billing"]);
        commit_file(repo, "billing.txt", "billing\n", "feat: billing");
        git_ok(repo, &["checkout", "feat/auth-ui"]);

        let output = dgr_ok(repo, &["tree", "--branch", "feat/billing"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert_eq!(stdout.trim_end(), "* feat/billing\n\n✓ feat/auth-ui");
        assert_eq!(
            git_stdout(repo, &["branch", "--show-current"]),
            "feat/auth-ui"
        );
    });
}
