use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn contextpatch_server() -> &'static str {
    env!("CARGO_BIN_EXE_contextpatch-server")
}

#[test]
fn stage1_mcp_tools_work_together() {
    let root = git_repo("stage1_mcp_tools_work_together");
    fs::write(root.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":2,"end_line":3}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"diff_preview","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"created.txt","content":"new file\n"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
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
        "run_guarded_command",
        "read_command_log",
        "validation_profile_run",
        "setup_profile_run",
        "native_build_run",
        "native_device_run",
        "git_commit_exact",
        "git_remote_check",
        "git_branch_prepare",
        "git_merge_readiness",
        "git_push_exact",
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
    assert_text(&responses[5], "replaced bytes");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("created.txt")).unwrap(),
        "new file\n"
    );

    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "status_guard refused");
    assert_text(&responses[6], "sample.txt");
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
fn stage1_mcp_refusals_are_tool_results() {
    let root = git_repo("stage1_mcp_refusals_are_tool_results");
    fs::write(root.join("sample.txt"), "beta beta\n").unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"sample.txt","content":"replacement"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "expected exactly one match");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "already exists");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta beta\n"
    );
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
    assert_text(&responses[0], "\"action\": \"ios_build\"");
    assert_text(&responses[0], "\"action\": \"android_read_logcat\"");
    assert_text(&responses[1], "\"guarded_process_execution\"");
    assert_text(&responses[1], "\"setup_profiles\"");
    assert_text(&responses[1], "\"native_build\"");
    assert_text(&responses[1], "\"native_device\"");
    assert_text(&responses[2], "allowlist: git/status");
    assert_text(&responses[2], "exit_code: 0");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "not allowlisted");
}

#[test]
fn stage2_setup_profile_run_plans_capacitor_shell_without_mutating() {
    let root = git_repo("stage2_setup_profile_run_plans_capacitor_shell_without_mutating");
    fs::write(root.join("package.json"), "{}\n").unwrap();
    git(&root, &["add", "package.json"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_init","params":{"app_id":"com.example.app","app_name":"Example","web_dir":"dist"},"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"ios"}}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"windows"}}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"npm","args":["install","@capacitor/core"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"ios_pod_install"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"pnpm","args":["add","@capacitor/core"],"timeout_secs":30}}}"#,
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
        "command: pnpm add \"@capacitor/core\" \"@capacitor/cli\" \"@capacitor/ios\" \"@capacitor/android\"",
    );
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
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"ios/App/App.xcworkspace","scheme":"App"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"android_assemble_debug","params":{},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"../App.xcworkspace","scheme":"App"},"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "action: ios_build");
    assert_text(
        &responses[0],
        "command: xcodebuild -workspace \"ios/App/App.xcworkspace\" -scheme App -configuration Debug -sdk iphonesimulator build",
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
