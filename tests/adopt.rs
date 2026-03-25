mod support;

use support::{
    active_rebase_head_name, commit_file, dig, dig_ok, events_contain_type, find_node, git_ok,
    git_stdout, initialize_main_repo, load_operation_json, load_state_json, overwrite_file,
    strip_ansi, with_temp_repo,
};

#[test]
fn adopts_current_branch_onto_trunk_without_rebase() {
    with_temp_repo("dig-adopt-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        git_ok(repo, &["checkout", "-b", "feat/adopted"]);
        commit_file(repo, "adopted.txt", "adopted\n", "feat: adopted");

        let output = dig_ok(repo, &["adopt", "-p", "main"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Adopted 'feat/adopted' under 'main'."));
        assert!(stdout.contains("main\n└── ✓ feat/adopted"));

        let state = load_state_json(repo);
        let adopted = find_node(&state, "feat/adopted").unwrap();
        assert_eq!(adopted["base_ref"], "main");
        assert_eq!(adopted["parent"]["kind"], "trunk");
    });
}

#[test]
fn adopts_named_branch_with_rebase_and_restores_original_branch() {
    with_temp_repo("dig-adopt-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        commit_file(repo, "auth.txt", "auth\n", "feat: auth");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["checkout", "-b", "feat/auth-ui"]);
        commit_file(repo, "ui.txt", "ui\n", "feat: auth ui");
        git_ok(repo, &["checkout", "feat/auth"]);

        let output = dig_ok(repo, &["adopt", "feat/auth-ui", "-p", "feat/auth"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Adopted 'feat/auth-ui' under 'feat/auth'."));
        assert!(stdout.contains("Restacked 'feat/auth-ui' onto 'feat/auth'."));
        assert!(stdout.contains("Returned to 'feat/auth' after adopt."));
        assert!(stdout.contains("main\n└── feat/auth\n    └── ✓ feat/auth-ui"));

        let merge_base = git_stdout(repo, &["merge-base", "feat/auth", "feat/auth-ui"]);
        let parent_head = git_stdout(repo, &["rev-parse", "feat/auth"]);
        assert_eq!(merge_base, parent_head);
        assert_eq!(git_stdout(repo, &["branch", "--show-current"]), "feat/auth");

        let state = load_state_json(repo);
        let adopted = find_node(&state, "feat/auth-ui").unwrap();
        assert_eq!(adopted["base_ref"], "feat/auth");
        assert_eq!(adopted["parent"]["kind"], "branch");
    });
}

#[test]
fn leaves_rebase_open_when_adopt_rebase_conflicts() {
    with_temp_repo("dig-adopt-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);
        overwrite_file(repo, "shared.txt", "parent\n", "feat: parent change");
        git_ok(repo, &["checkout", "main"]);
        git_ok(repo, &["checkout", "-b", "feat/auth-ui"]);
        overwrite_file(repo, "shared.txt", "child\n", "feat: child change");
        git_ok(repo, &["checkout", "feat/auth"]);

        let output = dig(repo, &["adopt", "feat/auth-ui", "-p", "feat/auth"]);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!output.status.success());

        let state = load_state_json(repo);
        assert!(find_node(&state, "feat/auth-ui").is_none());
        assert!(!events_contain_type(repo, "branch_adopted"));
        assert!(stderr.contains("dig sync --continue"));
        assert!(repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists());
        assert!(load_operation_json(repo).is_some());
        assert!(
            active_rebase_head_name(repo).contains("feat/auth-ui"),
            "expected rebase head-name to reference feat/auth-ui"
        );
    });
}
