mod support;

use support::{
    dig_ok, find_node, initialize_main_repo, load_state_json, strip_ansi, with_temp_repo,
};

#[test]
fn branch_command_renders_marked_lineage_and_tracks_parent() {
    with_temp_repo("dig-branch-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);

        let output = dig_ok(repo, &["branch", "feat/auth"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Created and switched to 'feat/auth'."));
        assert!(stdout.contains("✓ feat/auth\n│ \n*  main"));

        let state = load_state_json(repo);
        let node = find_node(&state, "feat/auth").unwrap();
        assert_eq!(node["base_ref"], "main");
        assert_eq!(node["parent"]["kind"], "trunk");
    });
}

#[test]
fn init_reuses_marked_lineage_output_for_current_branch() {
    with_temp_repo("dig-branch-cli", |repo| {
        initialize_main_repo(repo);
        dig_ok(repo, &["init"]);
        dig_ok(repo, &["branch", "feat/auth"]);

        let output = dig_ok(repo, &["init"]);
        let stdout = strip_ansi(&String::from_utf8(output.stdout).unwrap());

        assert!(stdout.contains("Using existing Git repository."));
        assert!(stdout.contains("Dig is already initialized."));
        assert!(stdout.contains("✓ feat/auth\n│ \n*  main"));
    });
}
