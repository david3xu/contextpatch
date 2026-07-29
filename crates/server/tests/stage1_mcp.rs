use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn contextpatch_server() -> &'static str {
    env!("CARGO_BIN_EXE_contextpatch-server")
}

#[test]
fn stage1_mcp_tools_work_together() {
    let root = git_repo("stage1_mcp_tools_work_together");
    fs::write(root.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();
    fs::write(root.join("scratch.log"), "temporary\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":2,"end_line":3}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"diff_preview","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"create_directory","arguments":{"path":"native-plugins/background-audio","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"native-plugins/background-audio/plugin.ts","content":"new file\n"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"fixture.bin","content_base64":"AAEC/w==","expected_bytes":4}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"artifact_write_text","arguments":{"path":"stage1/tool.txt","content":"sidecar\n","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"delete_untracked_exact","arguments":{"paths":["scratch.log"],"dry_run":false,"confirm":"delete untracked files"}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"git_remote_list","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
        ],
    );

    let list = &responses[0]["result"]["tools"];
    for name in [
        "capability_manifest",
        "preflight_health",
        "read_range",
        "diff_preview",
        "replace_exact",
        "status_guard",
        "write_new_file",
        "write_new_file_base64",
        "write_existing_file_exact_hash",
        "file_info",
        "list_directory",
        "read_file_bytes",
        "artifact_write_text",
        "artifact_write_base64",
        "bulk_write_new_files_base64",
        "create_directory",
        "run_guarded_command",
        "artifact_python_run",
        "image_cleanliness_check_run",
        "docker_image_inspect",
        "fixture_generator_run",
        "base_image_check_run",
        "fixture_manifest_verify",
        "fixture_manifest_refresh",
        "read_command_log",
        "validation_profile_run",
        "setup_profile_run",
        "native_build_run",
        "native_device_run",
        "git_commit_exact",
        "git_commit_scoped",
        "git_commit_prefix",
        "git_stage_exact",
        "git_restore_exact",
        "delete_untracked_exact",
        "delete_generated_prefix",
        "git_remote_list",
        "git_remote_check",
        "git_branch_prepare",
        "git_merge_readiness",
        "git_push_exact",
        "github_pr_run",
        "github_fork_prepare",
    ] {
        assert!(
            list.as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name),
            "tools/list did not include {name}: {list}"
        );
    }

    assert_text(&responses[1], "clean: no Git changes under sample.txt");
    assert_text(&responses[2], "2. beta\n3. gamma\n");
    assert_text(&responses[3], "-beta\n+delta");
    assert_text(&responses[4], "created");
    assert_text(&responses[5], "created");
    assert_text(&responses[6], "created");
    assert_text(&responses[7], "\"repo_mutation\": false");
    assert_text(&responses[8], "\"deleted\": true");
    assert_text(&responses[9], "\"tool\": \"git_remote_list\"");
    assert_text(&responses[10], "replaced bytes");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("native-plugins/background-audio/plugin.ts")).unwrap(),
        "new file\n"
    );
    assert_eq!(
        fs::read(root.join("fixture.bin")).unwrap(),
        vec![0, 1, 2, 255]
    );
    assert!(!root.join("scratch.log").exists());

    assert_eq!(responses[11]["result"]["isError"], true);
    assert_text(&responses[11], "status_guard refused");
    assert_text(&responses[11], "sample.txt");
}

#[test]
fn stage2_git_branch_prepare_creates_branch_from_remote_base() {
    let origin = bare_repo("stage2_git_branch_prepare_creates_branch_origin");
    let seed = git_repo("stage2_git_branch_prepare_creates_branch_seed");
    fs::write(
        seed.join("azure-pipelines.foundry-adapter.yml"),
        "pipeline\n",
    )
    .unwrap();
    git(&seed, &["add", "azure-pipelines.foundry-adapter.yml"]);
    git(&seed, &["commit", "--quiet", "-m", "initial"]);
    git(&seed, &["branch", "-M", "Develop"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "--quiet", "-u", "origin", "Develop"]);
    let root = temp_root("stage2_git_branch_prepare_creates_branch");
    git(&root, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&root, &["config", "user.name", "Contextpatch Test"]);
    git(
        &root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"chore/personal-fresh-redeploy-20260704-085732","required_files":["azure-pipelines.foundry-adapter.yml"]}}}"#,
        ],
    );

    assert_text(&responses[0], "\"prepared\": true");
    assert_text(&responses[0], "\"action\": \"created_branch\"");
    assert_text(
        &responses[0],
        "\"current_branch\": \"chore/personal-fresh-redeploy-20260704-085732\"",
    );
    assert_text(&responses[0], "\"remote_base_is_ancestor\": true");
    assert_eq!(
        git_stdout(&root, &["branch", "--show-current"]).trim(),
        "chore/personal-fresh-redeploy-20260704-085732"
    );
    assert_eq!(git_stdout(&root, &["status", "--short"]), "");
}

#[test]
fn stage2_git_branch_prepare_refuses_missing_required_file_before_switch() {
    let origin = bare_repo("stage2_git_branch_prepare_missing_file_origin");
    let seed = git_repo("stage2_git_branch_prepare_missing_file_seed");
    fs::write(seed.join("README.md"), "readme\n").unwrap();
    git(&seed, &["add", "README.md"]);
    git(&seed, &["commit", "--quiet", "-m", "initial"]);
    git(&seed, &["branch", "-M", "Develop"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "--quiet", "-u", "origin", "Develop"]);
    let root = temp_root("stage2_git_branch_prepare_missing_file");
    git(&root, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&root, &["config", "user.name", "Contextpatch Test"]);
    git(
        &root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
    let branch_before = git_stdout(&root, &["branch", "--show-current"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"chore/personal-fresh-redeploy-20260704-085732","required_files":["azure-pipelines.foundry-adapter.yml"]}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "required file");
    assert_text(&responses[0], "is missing from");
    assert_eq!(
        git_stdout(&root, &["branch", "--show-current"]),
        branch_before
    );
}

#[test]
fn stage2_git_branch_prepare_requires_confirmation_to_reset_existing_branch() {
    let origin = bare_repo("stage2_git_branch_prepare_reset_confirmation_origin");
    let seed = git_repo("stage2_git_branch_prepare_reset_confirmation_seed");
    fs::write(seed.join("pipeline.yml"), "pipeline\n").unwrap();
    git(&seed, &["add", "pipeline.yml"]);
    git(&seed, &["commit", "--quiet", "-m", "initial"]);
    git(&seed, &["branch", "-M", "Develop"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "--quiet", "-u", "origin", "Develop"]);
    let root = temp_root("stage2_git_branch_prepare_reset_confirmation");
    git(&root, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&root, &["config", "user.name", "Contextpatch Test"]);
    git(
        &root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
    git(
        &root,
        &["checkout", "--quiet", "-b", "Develop", "origin/Develop"],
    );
    git(&root, &["checkout", "--quiet", "--orphan", "feature"]);
    git(&root, &["rm", "-rf", "."]);
    fs::write(root.join("other.txt"), "other\n").unwrap();
    git(&root, &["add", "other.txt"]);
    git(&root, &["commit", "--quiet", "-m", "other root"]);
    git(&root, &["checkout", "--quiet", "Develop"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature","reset_existing":true}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature","reset_existing":true,"confirm":"reset branch from remote base","required_files":["pipeline.yml"]}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "is not based on");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"prepared\": true");
    assert_text(&responses[2], "\"action\": \"reset_existing_branch\"");
    assert_eq!(
        git_stdout(&root, &["branch", "--show-current"]).trim(),
        "feature"
    );
    assert!(root.join("pipeline.yml").is_file());
    assert!(!root.join("other.txt").exists());
}

#[test]
fn stage2_git_merge_readiness_reports_changed_on_both_sides() {
    let root = git_repo("stage2_git_merge_readiness_reports_changed_on_both_sides");
    fs::write(root.join("app.txt"), "initial\n").unwrap();
    git(&root, &["add", "app.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    git(&root, &["branch", "-M", "main"]);
    git(&root, &["checkout", "--quiet", "-b", "feature"]);
    fs::write(root.join("app.txt"), "feature\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "feature change"]);
    git(&root, &["checkout", "--quiet", "main"]);
    fs::write(root.join("app.txt"), "main\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "main change"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_merge_readiness","arguments":{"base_ref":"main","target_ref":"feature"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_merge_readiness","arguments":{"base_ref":"main..bad","target_ref":"feature"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"read_only\": true");
    assert_text(&responses[0], "\"changed_on_both_sides_count\": 1");
    assert_text(&responses[0], "\"has_likely_conflict_candidates\": true");
    assert_text(&responses[0], "app.txt");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "invalid ref");
}

#[test]
fn stage2_git_merge_readiness_fetches_one_branch_without_source_changes() {
    let origin = bare_repo("stage2_git_merge_readiness_fetches_one_branch_origin");
    let seed = git_repo("stage2_git_merge_readiness_fetches_one_branch_seed");
    fs::write(seed.join("app.txt"), "initial\n").unwrap();
    git(&seed, &["add", "app.txt"]);
    git(&seed, &["commit", "--quiet", "-m", "initial"]);
    git(&seed, &["branch", "-M", "main"]);
    git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&seed, &["push", "--quiet", "-u", "origin", "main"]);

    let root = temp_root("stage2_git_merge_readiness_fetches_one_branch");
    git(&root, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&root, &["config", "user.name", "Contextpatch Test"]);
    git(
        &root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );

    let other = temp_root("stage2_git_merge_readiness_fetches_one_branch_other");
    git(&other, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&other, &["config", "user.name", "Contextpatch Test"]);
    git(
        &other,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
    git(&other, &["checkout", "--quiet", "-b", "feature"]);
    fs::write(other.join("feature.txt"), "feature\n").unwrap();
    git(&other, &["add", "feature.txt"]);
    git(&other, &["commit", "--quiet", "-m", "feature change"]);
    git(&other, &["push", "--quiet", "-u", "origin", "feature"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_merge_readiness","arguments":{"base_ref":"main","target_ref":"origin/feature","fetch":true,"remote":"origin"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"fetch_performed\": true");
    assert_text(&responses[0], "\"fetched_branch\": \"feature\"");
    assert_text(&responses[0], "\"target_ahead_count\": 1");
    assert_text(&responses[0], "\"source_status_unchanged\": true");
    assert_eq!(git_stdout(&root, &["status", "--short"]), "");
}

#[test]
fn stage2_git_remote_check_and_push_exact_are_gated() {
    let origin = bare_repo("stage2_git_remote_check_and_push_exact_are_gated_origin");
    let root = git_repo("stage2_git_remote_check_and_push_exact_are_gated");
    fs::write(root.join("sample.txt"), "initial\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    git(&root, &["branch", "-M", "main"]);
    git(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&root, &["push", "--quiet", "-u", "origin", "main"]);

    fs::write(root.join("sample.txt"), "changed\n").unwrap();
    let commit_responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt"],"subject":"test: local change","dry_run":false,"confirm":"commit exact paths"}}}"#,
        ],
    );
    assert_text(&commit_responses[0], "\"committed\": true");
    let head = git_stdout(&root, &["rev-parse", "HEAD"]);

    let remote_check = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_remote_check","arguments":{"remote":"origin","branch":"main"}}}"#;
    let push_without_confirm = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"git_push_exact","arguments":{{"remote":"origin","branch":"main","expected_head":"{}","confirm":"wrong"}}}}}}"#,
        head.trim()
    );
    let push = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"git_push_exact","arguments":{{"remote":"origin","branch":"main","expected_head":"{}","confirm":"push exact commit"}}}}}}"#,
        head.trim()
    );

    let responses = run_server(&root, &[remote_check, &push_without_confirm, &push]);

    assert_text(&responses[0], "\"head_to_remote_empty\": true");
    assert_text(&responses[0], "\"local_ahead_count\": 1");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "confirm must be");
    assert_text(&responses[2], "\"pushed\": true");
    assert_text(&responses[2], "\"force\": false");

    let remote_head = git_stdout(&root, &["ls-remote", "origin", "refs/heads/main"]);
    assert!(
        remote_head.starts_with(head.trim()),
        "remote head {remote_head:?} did not match {head:?}"
    );
}

#[test]
fn stage2_git_push_exact_refuses_remote_ahead() {
    let origin = bare_repo("stage2_git_push_exact_refuses_remote_ahead_origin");
    let root = git_repo("stage2_git_push_exact_refuses_remote_ahead");
    fs::write(root.join("sample.txt"), "initial\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    git(&root, &["branch", "-M", "main"]);
    git(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&root, &["push", "--quiet", "-u", "origin", "main"]);

    let other = temp_root("stage2_git_push_exact_refuses_remote_ahead_other");
    git(&other, &["clone", "--quiet", origin.to_str().unwrap(), "."]);
    git(&other, &["config", "user.name", "Contextpatch Test"]);
    git(
        &other,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
    fs::write(other.join("sample.txt"), "remote\n").unwrap();
    git(&other, &["commit", "--quiet", "-am", "remote change"]);
    git(&other, &["push", "--quiet", "origin", "main"]);

    let head = git_stdout(&root, &["rev-parse", "HEAD"]);
    let push = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"git_push_exact","arguments":{{"remote":"origin","branch":"main","expected_head":"{}","confirm":"push exact commit"}}}}}}"#,
        head.trim()
    );
    let responses = run_server(&root, &[&push]);

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "remote `refs/remotes/origin/main` is ahead");
}

#[test]
fn stage2_git_commit_exact_dry_run_and_commit_are_gated() {
    let root = git_repo("stage2_git_commit_exact_dry_run_and_commit_are_gated");
    fs::write(root.join("sample.txt"), "before\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("sample.txt"), "after\n").unwrap();
    fs::write(root.join("created.txt"), "new\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths","body":"Co-authored-by: Contextpatch <contextpatch@example.invalid>","dry_run":false,"confirm":"commit exact paths"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"would_commit\": true");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"committed\": true");
    assert_text(&responses[2], "\"push\": false");

    let log = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "-1", "--pretty=%s%n%b"])
        .output()
        .unwrap();
    let log = String::from_utf8(log.stdout).unwrap();
    assert!(log.contains("test: commit exact paths"));
    assert!(log.contains("Co-authored-by: Contextpatch"));

    let status = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["status", "--short"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "");
}

#[test]
fn stage2_git_commit_exact_refuses_partial_dirty_path_set() {
    let root = git_repo("stage2_git_commit_exact_refuses_partial_dirty_path_set");
    fs::write(root.join("one.txt"), "one\n").unwrap();
    fs::write(root.join("two.txt"), "two\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["one.txt"],"subject":"test: partial"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "provided paths must exactly match");
    assert_text(&responses[0], "two.txt");
}

#[test]
fn stage2_git_commit_scoped_commits_subset_and_preserves_other_dirty_paths() {
    let root = git_repo("stage2_git_commit_scoped_commits_subset_and_preserves_other_dirty_paths");
    fs::write(root.join("included.txt"), "old\n").unwrap();
    fs::write(root.join("kept.txt"), "old\n").unwrap();
    git(&root, &["add", "included.txt", "kept.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("included.txt"), "new\n").unwrap();
    fs::write(root.join("kept.txt"), "new\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_scoped","arguments":{"paths":["included.txt"],"subject":"test: scoped commit"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_commit_scoped","arguments":{"paths":["included.txt"],"subject":"test: scoped commit","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_commit_scoped","arguments":{"paths":["included.txt"],"subject":"test: scoped commit","dry_run":false,"confirm":"commit scoped paths"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"would_commit\": true");
    assert_text(&responses[0], "kept.txt");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"committed\": true");
    assert_text(&responses[2], "\"paths\"");
    assert_text(&responses[2], "included.txt");
    assert_text(&responses[2], "\"remaining_dirty_paths\"");
    assert_text(&responses[2], "kept.txt");

    let committed_files = git_stdout(&root, &["show", "--name-only", "--pretty=", "HEAD"]);
    assert!(committed_files.lines().any(|line| line == "included.txt"));
    assert!(!committed_files.lines().any(|line| line == "kept.txt"));

    let status = git_stdout(&root, &["status", "--short"]);
    assert_eq!(status, " M kept.txt\n");
}

#[test]
fn stage2_git_commit_scoped_refuses_preexisting_staged_paths() {
    let root = git_repo("stage2_git_commit_scoped_refuses_preexisting_staged_paths");
    fs::write(root.join("included.txt"), "old\n").unwrap();
    fs::write(root.join("staged.txt"), "old\n").unwrap();
    git(&root, &["add", "included.txt", "staged.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("included.txt"), "new\n").unwrap();
    fs::write(root.join("staged.txt"), "new\n").unwrap();
    git(&root, &["add", "staged.txt"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_scoped","arguments":{"paths":["included.txt"],"subject":"test: scoped commit","dry_run":false,"confirm":"commit scoped paths"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "index must be clean");
    assert_text(&responses[0], "staged.txt");

    let log = git_stdout(&root, &["log", "--oneline", "-1"]);
    assert!(log.contains("initial"));
}

#[test]
fn stage2_git_commit_prefix_expands_dirty_paths_under_prefixes() {
    let root = git_repo("stage2_git_commit_prefix_expands_dirty_paths_under_prefixes");
    fs::create_dir_all(root.join("task/tests")).unwrap();
    fs::create_dir_all(root.join("task/solution")).unwrap();
    fs::write(root.join("task/tests/a.txt"), "old\n").unwrap();
    fs::write(root.join("task/solution/solve.py"), "old\n").unwrap();
    fs::write(root.join("kept.txt"), "old\n").unwrap();
    git(&root, &["add", "task", "kept.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("task/tests/a.txt"), "new\n").unwrap();
    fs::write(root.join("task/solution/solve.py"), "new\n").unwrap();
    fs::write(root.join("kept.txt"), "new\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_prefix","arguments":{"prefixes":["task"],"subject":"test: prefix commit"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_commit_prefix","arguments":{"prefixes":["task"],"subject":"test: prefix commit","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_commit_prefix","arguments":{"prefixes":["task"],"subject":"test: prefix commit","dry_run":false,"confirm":"commit prefix paths"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"expanded_path_count\": 2");
    assert_text(&responses[0], "task/tests/a.txt");
    assert_text(&responses[0], "task/solution/solve.py");
    assert_text(&responses[0], "kept.txt");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"committed\": true");
    assert_text(&responses[2], "task/tests/a.txt");
    assert_text(&responses[2], "task/solution/solve.py");

    let committed_files = git_stdout(&root, &["show", "--name-only", "--pretty=", "HEAD"]);
    assert!(committed_files
        .lines()
        .any(|line| line == "task/tests/a.txt"));
    assert!(committed_files
        .lines()
        .any(|line| line == "task/solution/solve.py"));
    assert!(!committed_files.lines().any(|line| line == "kept.txt"));
    assert_eq!(git_stdout(&root, &["status", "--short"]), " M kept.txt\n");
}

#[test]
fn stage2_git_restore_exact_restores_only_requested_tracked_dirty_paths() {
    let root = git_repo("stage2_git_restore_exact_restores_only_requested_tracked_dirty_paths");
    fs::write(root.join("generated.txt"), "old\n").unwrap();
    fs::write(root.join("kept.txt"), "old\n").unwrap();
    git(&root, &["add", "generated.txt", "kept.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("generated.txt"), "new\n").unwrap();
    fs::write(root.join("kept.txt"), "new\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_restore_exact","arguments":{"paths":["generated.txt"]}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_restore_exact","arguments":{"paths":["generated.txt"],"dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_restore_exact","arguments":{"paths":["generated.txt"],"dry_run":false,"confirm":"restore exact paths"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"would_restore_paths\"");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"restored\": true");
    assert_eq!(
        fs::read_to_string(root.join("generated.txt")).unwrap(),
        "old\n"
    );
    assert_eq!(fs::read_to_string(root.join("kept.txt")).unwrap(), "new\n");
}

#[test]
fn stage2_delete_generated_prefix_dry_run_matches_only_generated_paths() {
    let root = git_repo("stage2_delete_generated_prefix_dry_run_matches_only_generated_paths");
    fs::create_dir_all(root.join("task/tests/__pycache__")).unwrap();
    fs::create_dir_all(root.join("task/tests/empty")).unwrap();
    fs::create_dir_all(root.join("task/tests/nested/empty")).unwrap();
    fs::write(root.join(".gitignore"), "__pycache__/\n*.pyc\n").unwrap();
    fs::write(root.join("task/tests/test_outputs.py"), "tracked\n").unwrap();
    git(&root, &["add", ".gitignore", "task/tests/test_outputs.py"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(
        root.join("task/tests/__pycache__/test_outputs.pyc"),
        "cache\n",
    )
    .unwrap();
    fs::write(root.join("task/tests/new.log"), "scratch\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delete_generated_prefix","arguments":{"prefixes":["task/tests"]}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"delete_generated_prefix","arguments":{"prefixes":["task/tests"],"dry_run":false}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "task/tests/__pycache__/test_outputs.pyc");
    assert_text(&responses[0], "task/tests/new.log");
    assert_text(&responses[0], "task/tests/empty");
    assert_text(&responses[0], "task/tests/nested/empty");
    assert!(!response_text(&responses[0]).contains("task/tests/test_outputs.py"));
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert!(root
        .join("task/tests/__pycache__/test_outputs.pyc")
        .exists());
    assert!(root.join("task/tests/test_outputs.py").exists());
}

#[test]
fn stage1_mcp_refusals_are_tool_results() {
    let root = git_repo("stage1_mcp_refusals_are_tool_results");
    fs::write(root.join("sample.txt"), "beta beta\n").unwrap();
    fs::create_dir(root.join("existing")).unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"sample.txt","content":"replacement"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_directory","arguments":{"path":"existing"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"bad.bin","content_base64":"not base64!"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"mismatch.bin","content_base64":"AAEC","expected_bytes":4}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"artifact_write_base64","arguments":{"path":"bad.bin","content_base64":"not base64!"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"delete_untracked_exact","arguments":{"paths":["sample.txt"],"dry_run":false,"confirm":"delete untracked files"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "expected exactly one match");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "already exists");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "already exists");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "invalid characters");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "expected 4");
    assert_eq!(responses[5]["result"]["isError"], true);
    assert_text(&responses[5], "invalid characters");
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "not_untracked_paths");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta beta\n"
    );
    assert!(!root.join("bad.bin").exists());
    assert!(!root.join("mismatch.bin").exists());
}

#[test]
fn stage2_image_cleanliness_check_run_plans_bounded_docker_find() {
    let root = git_repo("stage2_image_cleanliness_check_run_plans_bounded_docker_find");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"image_cleanliness_check_run","arguments":{"image":"example/task:latest"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"image_cleanliness_check_run","arguments":{"image":"example/task:latest","filename":"solve.sh","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"docker_image_inspect","arguments":{"image":"example/task:latest"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"docker\"");
    assert_text(&responses[0], "\"run\"");
    assert_text(&responses[0], "\"--network\"");
    assert_text(&responses[0], "\"none\"");
    assert_text(&responses[0], "\"--entrypoint\"");
    assert_text(&responses[0], "\"find\"");
    assert_text(&responses[0], "\"/\"");
    assert_text(&responses[0], "\"-name\"");
    assert_text(&responses[0], "\"solve.sh\"");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"docker\"");
    assert_text(&responses[2], "\"image\"");
    assert_text(&responses[2], "\"inspect\"");
    assert_text(&responses[2], "\"example/task:latest\"");
}

#[test]
fn stage2_file_inspection_tools_report_digest_listing_and_binary_ranges() {
    let root = git_repo("stage2_file_inspection_tools_report_digest_listing_and_binary_ranges");
    fs::create_dir(root.join("data")).unwrap();
    fs::write(root.join("data/sample.txt"), "alpha\nbeta\n").unwrap();
    fs::write(root.join("data/blob.bin"), [0, 1, 2, 255, 16, 32]).unwrap();
    let digest = Sha256::digest(b"alpha\nbeta\n")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"data/sample.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"data"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file_bytes","arguments":{"path":"data/blob.bin","offset":1,"max_bytes":3,"encoding":"hex"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"sha256\"");
    assert_text(&responses[0], &digest);
    assert_text(&responses[0], "\"line_count\": 2");
    assert_text(&responses[0], "\"is_symlink\": false");
    assert_text(&responses[1], "\"entry_count\": 2");
    assert_text(&responses[1], "\"path\": \"data/blob.bin\"");
    assert_text(&responses[1], "\"size_bytes\": 6");
    assert_text(&responses[2], "\"total_bytes\": 6");
    assert_text(&responses[2], "\"bytes_returned\": 3");
    assert_text(&responses[2], "\"data\": \"0102ff\"");

    #[cfg(unix)]
    {
        let outside = temp_root("stage2_file_inspection_outside_target");
        fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("data/outside-link"))
            .unwrap();
        let symlink_responses = run_server(
            &root,
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"data/outside-link"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file_bytes","arguments":{"path":"data/outside-link","max_bytes":4}}}"#,
            ],
        );

        assert_text(&symlink_responses[0], "\"is_symlink\": true");
        assert_text(
            &symlink_responses[0],
            "\"symlink_resolves_inside_repo\": false",
        );
        assert_text(&symlink_responses[0], "\"sha256\": null");
        assert_eq!(symlink_responses[1]["result"]["isError"], true);
        assert_text(&symlink_responses[1], "resolves outside the repository");
    }
}

#[test]
fn stage2_git_stage_exact_stages_without_committing() {
    let root = git_repo("stage2_git_stage_exact_stages_without_committing");
    fs::write(root.join("tracked.txt"), "before\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("tracked.txt"), "after\n").unwrap();
    fs::write(root.join("other.txt"), "other\n").unwrap();

    let head_before = git_stdout(&root, &["rev-parse", "HEAD"]);
    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_stage_exact","arguments":{"paths":["tracked.txt"],"dry_run":true}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_stage_exact","arguments":{"paths":["tracked.txt"],"dry_run":false,"confirm":"stage exact paths"}}}"#,
        ],
    );

    assert_text(&responses[0], "\"would_commit\": false");
    assert_text(&responses[1], "\"staged\": true");
    assert_text(&responses[1], "\"committed\": false");
    assert_eq!(git_stdout(&root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_stdout(&root, &["diff", "--cached", "--name-only"]),
        "tracked.txt\n"
    );
    assert_eq!(git_stdout(&root, &["diff", "--name-only"]), "");
    assert_text(&responses[1], "?? other.txt");
}

#[test]
fn stage2_command_log_offset_pages_long_logs() {
    let root = git_repo("stage2_command_log_offset_pages_long_logs");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["status","--porcelain=v1"],"timeout_secs":30}}}"#,
        ],
    );
    let text = response_text(&responses[0]);
    let log_id = text
        .lines()
        .find_map(|line| line.strip_prefix("log_id: "))
        .unwrap();
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"read_command_log","arguments":{{"log_id":"{log_id}","offset":5,"max_chars":20}}}}}}"#
    );
    let paged = run_server(&root, &[&request]);

    assert_text(&paged[0], "offset: 5");
    assert_text(&paged[0], "chars_returned: 20");
    assert_text(&paged[0], "total_chars:");
}

#[test]
fn stage2_artifact_python_run_executes_scratch_outside_repo() {
    let root = git_repo("stage2_artifact_python_run_executes_scratch_outside_repo");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"artifact_write_text","arguments":{"path":"scratch.py","content":"print('scratch-ok')\n","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"artifact_python_run","arguments":{"script":"scratch.py","timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "\"repo_mutation\": false");
    assert_text(&responses[1], "scratch-ok");
    assert!(!root.join("scratch.py").exists());
    assert_eq!(git_stdout(&root, &["status", "--short"]), "");
}

#[test]
fn stage2_mcp_reports_capabilities_and_runs_guarded_commands() {
    let root = git_repo("stage2_mcp_reports_capabilities_and_runs_guarded_commands");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capability_manifest","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"preflight_health","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["status","--porcelain=v1"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["reset"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_create","base":"main","head":"feature/task","title":"Add task solution","body":"Ready for review.","dry_run":true}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"github_fork_prepare","arguments":{"dry_run":true}}}"#,
        ],
    );

    assert_text(&responses[0], "\"process_execution\"");
    assert_text(&responses[0], "\"mode\": \"allowlisted_no_shell\"");
    assert_text(&responses[0], "\"setup_profiles\"");
    assert_text(&responses[0], "\"node-capacitor-shell\"");
    assert_text(&responses[0], "\"supported_package_managers\"");
    assert_text(&responses[0], "\"pnpm\"");
    assert_text(&responses[0], "\"native_build\"");
    assert_text(&responses[0], "\"native_device\"");
    assert_text(&responses[0], "\"examples\"");
    assert_text(&responses[0], "\"tool\": \"setup_profile_run\"");
    assert_text(&responses[0], "\"tool\": \"native_build_run\"");
    assert_text(&responses[0], "\"tool\": \"native_device_run\"");
    assert_text(&responses[0], "\"github_workflows\"");
    assert_text(&responses[0], "\"pr_create\"");
    assert_text(&responses[0], "\"github_fork_prepare\"");
    assert_text(&responses[0], "\"fixture_generator_run\"");
    assert_text(&responses[0], "\"base_image_check_run\"");
    assert_text(&responses[0], "\"bulk_write_new_files_base64\"");
    assert_text(&responses[0], "\"write_existing_file_exact_hash\"");
    assert_text(&responses[0], "\"fixture_manifest_verify\"");
    assert_text(&responses[0], "\"fixture_manifest_refresh\"");
    assert_text(&responses[0], "\"dynamo-harbor-task\"");
    assert_text(&responses[0], "\"action\": \"ios_build\"");
    assert_text(&responses[0], "\"action\": \"android_read_logcat\"");
    assert_text(&responses[1], "\"guarded_process_execution\"");
    assert_text(&responses[1], "\"validation_tools\"");
    assert_text(&responses[1], "\"python3\"");
    assert_text(&responses[1], "\"pytest\"");
    assert_text(&responses[1], "\"harbor\"");
    assert_text(&responses[1], "\"base_image_check\"");
    assert_text(&responses[1], "\"dynamo-harbor-task\"");
    assert_text(&responses[1], "do not claim Harbor scoring succeeded");
    assert_text(&responses[1], "\"setup_profiles\"");
    assert_text(&responses[1], "\"native_build\"");
    assert_text(&responses[1], "\"probe\": \"xcodebuild -version\"");
    assert_text(&responses[1], "\"native_device\"");
    assert_text(&responses[2], "allowlist: git/status");
    assert_text(&responses[2], "exit_code: 0");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "not allowlisted");
    assert_text(&responses[4], "\"tool\": \"github_pr_run\"");
    assert_text(&responses[4], "\"dry_run\": true");
    assert_text(&responses[4], "\"pr\"");
    assert_text(&responses[4], "\"create\"");
    assert_text(&responses[5], "\"tool\": \"github_fork_prepare\"");
    assert_text(&responses[5], "\"dry_run\": true");
    assert_text(&responses[5], "\"fork\"");
}

#[test]
fn stage2_fixture_workflow_tools_cover_generator_base_image_and_bulk_import() {
    let root = git_repo("stage2_fixture_workflow_tools_cover_generator_base_image_and_bulk_import");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(root.join("README.md"), "readme\n").unwrap();
    fs::write(
        root.join("scripts/gen.py"),
        r#"import pathlib
import sys

target = pathlib.Path(sys.argv[1])
target.parent.mkdir(parents=True, exist_ok=True)
target.write_bytes(b"\x00fixture")
if len(sys.argv) > 2:
    pathlib.Path(sys.argv[2]).write_text("rogue\n")
"#,
    )
    .unwrap();
    fs::write(
        root.join("references/check-base-image.sh"),
        "#!/usr/bin/env bash\nif [ \"${1:-}\" = task ]; then exit 0; fi\nexit 1\n",
    )
    .unwrap();
    git(
        &root,
        &["add", "README.md", "references/check-base-image.sh"],
    );
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"parents":true,"entries":[{"path":"task/environment/data/a.bin","content_base64":"AQI=","expected_bytes":2},{"path":"task/environment/data/b.txt","content_base64":"aGkK","expected_bytes":3}]}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"entries":[{"path":"task/environment/data/a.bin","content_base64":"AA=="}]}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"entries":[{"path":"../escape.bin","content_base64":"AA=="}]}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/dry.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"confirm":"run fixture generator","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out2.bin","rogue.txt"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","generated/out.bin","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"confirm":"run fixture generator","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":false,"confirm":"run base image check","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"bash","args":["scripts/gen.py"],"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "\"file_count\": 2");
    assert_eq!(
        fs::read(root.join("task/environment/data/a.bin")).unwrap(),
        [1, 2]
    );
    assert_eq!(
        fs::read_to_string(root.join("task/environment/data/b.txt")).unwrap(),
        "hi\n"
    );
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "target already exists");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "normalized relative path");
    assert_text(&responses[3], "\"dry_run\": true");
    assert!(!root.join("generated/dry.bin").exists());
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "confirm must be");
    assert_text(&responses[5], "\"ran\": true");
    assert_eq!(
        fs::read(root.join("generated/out.bin")).unwrap(),
        b"\0fixture"
    );
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "outside declared outputs");
    assert_text(&responses[6], "rogue.txt");
    assert_text(&responses[7], "\"dry_run\": true");
    assert_text(&responses[7], "references/check-base-image.sh");
    assert_text(&responses[7], "task");
    assert_eq!(responses[8]["result"]["isError"], true);
    assert_text(&responses[8], "confirm must be");
    assert_text(&responses[9], "\"ran\": true");
    assert_eq!(responses[10]["result"]["isError"], true);
    assert_text(&responses[10], "not allowlisted");
}

#[test]
fn stage2_manifest_and_hash_tools_cover_fixture_integrity_workflow() {
    let root = git_repo("stage2_manifest_and_hash_tools_cover_fixture_integrity_workflow");
    fs::create_dir_all(root.join("task/environment/data")).unwrap();
    fs::create_dir_all(root.join("task/tests")).unwrap();
    fs::write(root.join("task/environment/data/a.bin"), [1_u8, 2, 3]).unwrap();
    fs::write(root.join("task/environment/data/b.txt"), "hello\n").unwrap();
    fs::write(root.join("README.md"), "before\n").unwrap();
    git(&root, &["add", "README.md", "task/environment/data"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let readme_sha = sha256_hex_for_test(b"before\n");
    let dry_write = run_server(
        &root,
        &[&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":true}}}}}}"#
        )],
    );
    assert_text(&dry_write[0], "\"dry_run\": true");
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "before\n"
    );

    let refresh_confirm = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":false,"confirm":"refresh fixture manifest"}}}"#;
    let responses = run_server(
        &root,
        &[
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":false}}}}}}"#
            ),
            &format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":false,"confirm":"write exact hash"}}}}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":true}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":false}}}"#,
            refresh_confirm,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fixture_manifest_verify","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"]}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "requires confirm");
    assert_text(&responses[1], "\"wrote\": true");
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "after\n"
    );
    assert_text(&responses[2], "\"dry_run\": true");
    assert_text(&responses[2], "\"file_count\": 2");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "requires confirm");
    assert_text(&responses[4], "\"refreshed\": true");
    assert_text(&responses[5], "\"verified\": true");

    fs::write(root.join("task/environment/data/b.txt"), "changed\n").unwrap();
    let verify_again = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fixture_manifest_verify","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"]}}}"#,
        ],
    );
    assert_eq!(verify_again[0]["result"]["isError"], true);
    assert_text(&verify_again[0], "\"verified\": false");
    assert_text(&verify_again[0], "\"modified_files\"");
}

#[test]
fn stage2_setup_profile_run_plans_capacitor_shell_without_mutating() {
    let root = git_repo("stage2_setup_profile_run_plans_capacitor_shell_without_mutating");
    fs::write(root.join("package.json"), "{}\n").unwrap();
    fs::create_dir_all(root.join("ios/App")).unwrap();
    fs::write(root.join("ios/App/Podfile"), "target 'App' do\nend\n").unwrap();
    git(&root, &["add", "package.json", "ios/App/Podfile"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_init","params":{"app_id":"com.example.app","app_name":"Example","web_dir":"dist"},"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"ios"}}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"windows"}}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"npm","args":["install","@capacitor/core"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"ios_pod_install","cwd":"ios/App"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"pnpm","args":["add","@capacitor/core"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_dependencies","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"ios_pod_install"}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_filesystem","dry_run":true,"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "profile: node-capacitor-shell");
    assert_text(&responses[0], "action: cap_init");
    assert_text(
        &responses[0],
        "command: npm exec -- cap init Example com.example.app --web-dir dist",
    );
    assert_text(&responses[0], "external_mutator: true");
    assert_text(
        &responses[0],
        "required_confirm_for_mutation: \"run setup profile\"",
    );
    assert_text(&responses[1], "command: npm exec -- cap sync ios");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "requires confirm");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "unsupported cap_sync platform");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "not allowlisted");
    assert_text(&responses[5], "command: pod install");
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "not allowlisted");
    assert_text(
        &responses[7],
        "commands: npm install \"@capacitor/core\" \"@capacitor/ios\" \"@capacitor/android\"",
    );
    assert_text(&responses[7], "npm install --save-dev \"@capacitor/cli\"");
    assert_eq!(responses[8]["result"]["isError"], true);
    assert_text(&responses[8], "requires a Podfile");
    assert_text(
        &responses[9],
        "command: npm install \"@capacitor/filesystem\"",
    );
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "setup_profile_run dry-runs must not mutate the repository"
    );
}

#[test]
fn stage2_setup_profile_run_uses_pnpm_for_pnpm_projects_without_raw_allowlist() {
    let root =
        git_repo("stage2_setup_profile_run_uses_pnpm_for_pnpm_projects_without_raw_allowlist");
    fs::write(root.join("package.json"), "{}\n").unwrap();
    fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
    git(&root, &["add", "package.json", "pnpm-lock.yaml"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_dependencies","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"ios"},"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"pnpm","args":["add","@capacitor/core"],"timeout_secs":30}}}"#,
        ],
    );

    assert_text(
        &responses[0],
        "commands: pnpm add \"@capacitor/core\" \"@capacitor/ios\" \"@capacitor/android\"",
    );
    assert_text(&responses[0], "pnpm add --save-dev \"@capacitor/cli\"");
    assert_text(&responses[1], "command: pnpm exec cap sync ios");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "not allowlisted");
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "setup_profile_run dry-runs must not mutate pnpm projects"
    );
}

#[test]
fn stage2_native_build_run_plans_builds_without_raw_commands() {
    let root = git_repo("stage2_native_build_run_plans_builds_without_raw_commands");
    fs::write(root.join("gradlew"), "#!/bin/sh\nexit 0\n").unwrap();
    git(&root, &["add", "gradlew"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"ios/App/App.xcworkspace","scheme":"App","derived_data_path":".contextpatch-derived-data"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"android_assemble_debug","params":{},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"../App.xcworkspace","scheme":"App"},"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "action: ios_build");
    assert_text(
        &responses[0],
        "command: xcodebuild -workspace \"ios/App/App.xcworkspace\" -scheme App -configuration Debug -sdk iphonesimulator -derivedDataPath .contextpatch-derived-data build",
    );
    assert_text(&responses[0], "repo_validation: true");
    assert_text(&responses[1], "command: ./gradlew assembleDebug");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "repository-relative path");
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "native_build_run dry-runs must not mutate the repository"
    );
}

#[test]
fn stage2_native_device_run_plans_device_actions_and_requires_confirmation() {
    let root = git_repo("stage2_native_device_run_plans_device_actions_and_requires_confirmation");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_launch_app","params":{"device":"booted","app_id":"com.example.app"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"android_read_logcat","params":{"serial":"emulator-5554","lines":50},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_boot_simulator","params":{"device":"booted"},"dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"android_install_app","params":{"apk_path":"../app.apk"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_create_simulator","params":{"name":"ContextPatch iPhone","device_type":"iPhone 16","runtime":"iOS 26.4"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_cap_run","params":{"target":"00000000-0000-0000-0000-000000000000"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_read_logs","params":{"device":"booted","duration":"3"},"timeout_secs":30}}}"#,
        ],
    );

    assert_text(
        &responses[0],
        "command: xcrun simctl launch booted com.example.app",
    );
    assert_text(&responses[0], "changes_device_state: true");
    assert_text(
        &responses[1],
        "command: adb -s emulator-5554 logcat -d -t 50",
    );
    assert_text(&responses[1], "changes_device_state: false");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "requires confirm");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "repository-relative path");
    assert_text(
        &responses[4],
        "command: xcrun simctl create \"ContextPatch iPhone\" \"iPhone 16\" \"iOS 26.4\"",
    );
    assert_text(
        &responses[5],
        "command: npm exec -- cap run ios --target 00000000-0000-0000-0000-000000000000 --no-sync",
    );
    assert_text(
        &responses[6],
        "command: xcrun simctl spawn booted log stream --style compact --timeout 3",
    );
    assert_text(&responses[6], "changes_device_state: false");
}

#[test]
fn stage2_validation_profile_writes_readable_command_logs() {
    let root = git_repo("stage2_validation_profile_writes_readable_command_logs");

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"validation_profile_run","arguments":{"profile":"repo-basic","timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "profile: repo-basic");
    assert_text(&responses[0], "failed: false");
    assert_text(&responses[0], "git status --branch --short");
    assert_text(&responses[0], "git diff --check");

    let log_id = response_text(&responses[0])
        .split("log_id: ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap()
        .to_string();

    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"read_command_log","arguments":{{"log_id":"{log_id}"}}}}}}"#
    );
    let log_responses = run_server(&root, &[&request]);
    assert_text(&log_responses[0], &format!("log_id: {log_id}"));
    assert_text(&log_responses[0], "allowlist: git/status");
    assert_text(&log_responses[0], "timed_out: false");
}

#[test]
fn stage2_guarded_command_returns_while_mcp_stdin_stays_open() {
    let root = git_repo("stage2_guarded_command_returns_while_mcp_stdin_stays_open");
    let mut child = Command::new(contextpatch_server())
        .arg("--repo-root")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["status","--branch","--short"],"timeout_secs":30}}}
"#,
        )
        .unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    let mut reader = BufReader::new(stdout);
    reader.read_line(&mut line).unwrap();

    let response: Value = serde_json::from_str(&line).unwrap();
    assert_text(&response, "allowlist: git/status");
    assert_text(&response, "exit_code: 0");

    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "server failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stage2_rg_files_returns_while_mcp_stdin_stays_open() {
    let root = git_repo("stage2_rg_files_returns_while_mcp_stdin_stays_open");
    fs::create_dir_all(root.join("clients/vscode/test/suite")).unwrap();
    fs::write(
        root.join("clients/vscode/test/suite/live-runtime.test.ts"),
        "",
    )
    .unwrap();

    let mut child = Command::new(contextpatch_server())
        .arg("--repo-root")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"rg","args":["--files","clients/vscode/test"],"timeout_secs":30}}}
"#,
        )
        .unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    let mut reader = BufReader::new(stdout);
    reader.read_line(&mut line).unwrap();

    let response: Value = serde_json::from_str(&line).unwrap();
    assert_text(&response, "allowlist: rg/--files");
    assert_text(&response, "clients/vscode/test/suite/live-runtime.test.ts");

    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "server failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_server(root: &Path, requests: &[&str]) -> Vec<Value> {
    let mut child = Command::new(contextpatch_server())
        .arg("--repo-root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "server failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_text(response: &Value, expected: &str) {
    let text = response_text(response);
    assert!(
        text.contains(expected),
        "expected response text to contain {expected:?}, got {text:?}"
    );
}

fn response_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"].as_str().unwrap()
}

fn git_repo(name: &str) -> PathBuf {
    let root = temp_root(name);
    git(&root, &["init", "--quiet"]);
    git(&root, &["config", "user.name", "Contextpatch Test"]);
    git(
        &root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
    root
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn bare_repo(name: &str) -> PathBuf {
    let root = temp_root(name);
    let status = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(&root)
        .status()
        .unwrap();
    assert!(status.success());
    root
}

fn temp_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn sha256_hex_for_test(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
