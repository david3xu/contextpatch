use std::fs;
use std::process::Command;

use serde_json::Value;

use crate::support::*;

#[test]
fn stage2_git_branch_prepare_defaults_to_plan_then_creates_from_remote_base() {
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
    let branch_before = git_stdout(&root, &["branch", "--show-current"]);
    let remote_ref_before = git_stdout(&root, &["rev-parse", "refs/remotes/origin/Develop"]);

    fs::write(
        seed.join("azure-pipelines.foundry-adapter.yml"),
        "updated pipeline\n",
    )
    .unwrap();
    git(&seed, &["commit", "--quiet", "-am", "remote update"]);
    git(&seed, &["push", "--quiet", "origin", "Develop"]);
    let remote_head = git_stdout(&seed, &["rev-parse", "HEAD"]);
    assert_ne!(remote_ref_before, remote_head);

    let planned = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"chore/personal-fresh-redeploy-20260704-085732","required_files":["azure-pipelines.foundry-adapter.yml"]}}}"#,
        ],
    );

    let plan: Value = serde_json::from_str(response_text(&planned[0])).unwrap();
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["prepared"], false);
    assert_eq!(plan["would_prepare"], true);
    assert_eq!(plan["action"], "create_branch");
    assert_eq!(
        plan["commands"],
        serde_json::json!([
            {
                "program": "git",
                "args": [
                    "fetch",
                    "origin",
                    "refs/heads/Develop:refs/remotes/origin/Develop"
                ]
            },
            {
                "program": "git",
                "args": [
                    "switch",
                    "-c",
                    "chore/personal-fresh-redeploy-20260704-085732",
                    "refs/remotes/origin/Develop"
                ]
            }
        ])
    );
    assert_eq!(
        git_stdout(&root, &["branch", "--show-current"]),
        branch_before
    );
    assert_eq!(
        git_stdout(&root, &["rev-parse", "refs/remotes/origin/Develop"]),
        remote_ref_before,
        "dry-run must not fetch"
    );
    assert_eq!(
        git_stdout(
            &root,
            &[
                "branch",
                "--list",
                "chore/personal-fresh-redeploy-20260704-085732"
            ]
        ),
        "",
        "dry-run must not create the local branch"
    );

    let executed = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"chore/personal-fresh-redeploy-20260704-085732","required_files":["azure-pipelines.foundry-adapter.yml"],"dry_run":false}}}"#,
        ],
    );

    assert_text(&executed[0], "\"dry_run\": false");
    assert_text(&executed[0], "\"prepared\": true");
    assert_text(&executed[0], "\"action\": \"created_branch\"");
    assert_text(
        &executed[0],
        "\"current_branch\": \"chore/personal-fresh-redeploy-20260704-085732\"",
    );
    assert_text(&executed[0], "\"remote_base_is_ancestor\": true");
    assert_eq!(
        git_stdout(&root, &["branch", "--show-current"]).trim(),
        "chore/personal-fresh-redeploy-20260704-085732"
    );
    assert_eq!(
        git_stdout(&root, &["rev-parse", "refs/remotes/origin/Develop"]),
        remote_head
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
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"chore/personal-fresh-redeploy-20260704-085732","required_files":["azure-pipelines.foundry-adapter.yml"],"dry_run":false}}}"#,
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

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature","reset_existing":true,"dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_branch_prepare","arguments":{"remote":"origin","base_branch":"Develop","branch":"feature","reset_existing":true,"confirm":"reset branch from remote base","required_files":["pipeline.yml"],"dry_run":false}}}"#,
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

    let responses = run_server_sequential(
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

    let responses = run_server_sequential(&root, &[remote_check, &push_without_confirm, &push]);

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
    let initial_head = git_stdout(&root, &["rev-parse", "HEAD"]).trim().to_string();
    fs::write(root.join("sample.txt"), "after\n").unwrap();
    fs::write(root.join("created.txt"), "new\n").unwrap();

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["sample.txt","created.txt"],"subject":"test: commit exact paths","body":"Co-authored-by: Contextpatch <contextpatch@example.invalid>","dry_run":false,"confirm":"commit exact paths"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read_write_receipts","arguments":{"limit":10}}}"#,
        ],
    );

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"would_commit\": true");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"committed\": true");
    assert_text(&responses[2], "\"push\": false");
    let committed_head = git_stdout(&root, &["rev-parse", "HEAD"]).trim().to_string();
    let receipts: Value = serde_json::from_str(response_text(&responses[3])).unwrap();
    assert_eq!(receipts["receipts"][0]["tool"], "git_commit_exact");
    assert_eq!(receipts["receipts"][0]["outcome"], "applied");
    assert_eq!(receipts["receipts"][0]["before_git_head"], initial_head);
    assert_eq!(receipts["receipts"][0]["after_git_head"], committed_head);

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
fn stage2_git_commit_exact_receipt_handles_an_unborn_head() {
    let root = git_repo("stage2_git_commit_exact_receipt_handles_an_unborn_head");
    fs::write(root.join("first.txt"), "first\n").unwrap();

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_commit_exact","arguments":{"paths":["first.txt"],"subject":"test: first commit","dry_run":false,"confirm":"commit exact paths"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_write_receipts","arguments":{"limit":10}}}"#,
        ],
    );

    assert_text(&responses[0], "\"committed\": true");
    let committed_head = git_stdout(&root, &["rev-parse", "HEAD"]).trim().to_string();
    let receipts: Value = serde_json::from_str(response_text(&responses[1])).unwrap();
    assert_eq!(receipts["receipts"][0]["tool"], "git_commit_exact");
    assert_eq!(receipts["receipts"][0]["outcome"], "applied");
    assert_eq!(receipts["receipts"][0]["before_git_head"], "unborn");
    assert_eq!(receipts["receipts"][0]["after_git_head"], committed_head);
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

    let responses = run_server_sequential(
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

    let responses = run_server_sequential(
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

    let responses = run_server_sequential(
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
fn stage2_move_and_delete_tracked_files_are_dry_run_hash_and_confirmation_guarded() {
    let root =
        git_repo("stage2_move_and_delete_tracked_files_are_dry_run_hash_and_confirmation_guarded");
    fs::create_dir(root.join("archive")).unwrap();
    fs::write(root.join("move.txt"), "move me\n").unwrap();
    fs::write(root.join("obsolete.txt"), "delete me\n").unwrap();
    fs::write(root.join("dirty.txt"), "base\n").unwrap();
    git(&root, &["add", "move.txt", "obsolete.txt", "dirty.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("dirty.txt"), "changed\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();

    let delete_sha = sha256_hex_for_test(b"delete me\n");
    let untracked_sha = sha256_hex_for_test(b"untracked\n");
    let delete_mismatch = "0".repeat(64);
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"move_tracked","arguments":{"from":"move.txt","to":"archive/moved.txt"}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"move_tracked","arguments":{"from":"move.txt","to":"archive/moved.txt","dry_run":false}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"move_tracked","arguments":{"from":"move.txt","to":"archive/moved.txt","dry_run":false,"confirm":"move tracked file"}}}"#.to_string(),
        format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"delete_guarded","arguments":{{"path":"obsolete.txt","expected_sha256":"{delete_mismatch}"}}}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"delete_guarded","arguments":{{"path":"obsolete.txt","expected_sha256":"{delete_sha}","dry_run":false}}}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"delete_guarded","arguments":{{"path":"obsolete.txt","expected_sha256":"{delete_sha}","dry_run":false,"confirm":"delete tracked file"}}}}}}"#
        ),
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"move_tracked","arguments":{"from":"dirty.txt","to":"archive/dirty.txt"}}}"#.to_string(),
        format!(
            r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"delete_guarded","arguments":{{"path":"untracked.txt","expected_sha256":"{untracked_sha}"}}}}}}"#
        ),
    ];
    let request_refs = requests.iter().map(String::as_str).collect::<Vec<_>>();
    let responses = run_server_sequential(&root, &request_refs);

    assert_text(&responses[0], "\"dry_run\": true");
    assert_text(&responses[0], "\"moved\": false");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "requires confirm");
    assert_text(&responses[2], "\"moved\": true");
    assert!(!root.join("move.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("archive/moved.txt")).unwrap(),
        "move me\n"
    );

    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "hash mismatch");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "requires confirm");
    assert_text(&responses[5], "\"deleted\": true");
    assert!(!root.join("obsolete.txt").exists());
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "must be clean");
    assert_eq!(responses[7]["result"]["isError"], true);
    assert_text(&responses[7], "not tracked");
    assert!(root.join("untracked.txt").exists());

    let status = git_stdout(&root, &["status", "--short"]);
    assert!(status.contains("move.txt -> archive/moved.txt"));
    assert!(status.contains(" D obsolete.txt"));
    assert!(status.contains(" M dirty.txt"));
    assert!(status.contains("?? untracked.txt"));
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

    let responses = run_server_sequential(
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
fn stage2_git_stage_exact_stages_without_committing() {
    let root = git_repo("stage2_git_stage_exact_stages_without_committing");
    fs::write(root.join("tracked.txt"), "before\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.join("tracked.txt"), "after\n").unwrap();
    fs::write(root.join("other.txt"), "other\n").unwrap();

    let head_before = git_stdout(&root, &["rev-parse", "HEAD"]);
    let responses = run_server_sequential(
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
fn stage2_git_staged_scope_check_reports_policy_result() {
    let root = git_repo("stage2_git_staged_scope_check_reports_policy_result");
    fs::create_dir_all(root.join("task")).unwrap();
    fs::write(root.join(".gitignore"), "jobs/\n").unwrap();
    fs::write(root.join("task/task.toml"), "name = \"task\"\n").unwrap();
    fs::write(root.join("README.md"), "readme\n").unwrap();
    git(&root, &["add", ".gitignore", "task/task.toml", "README.md"]);

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"git_staged_scope_check","arguments":{"allowed_paths":[".gitignore"],"allowed_prefixes":["task"],"required_paths":["task/task.toml"]}}}"#,
        ],
    );

    assert_text(&responses[0], "\"tool\": \"git_staged_scope_check\"");
    assert_text(&responses[0], "\"read_only\": true");
    assert_text(&responses[0], "\"passed\": false");
    assert_text(&responses[0], "\"staged_path_count\": 3");
    assert_text(
        &responses[0],
        "\"disallowed_paths\": [\n    \"README.md\"\n  ]",
    );
    assert_text(&responses[0], "\"missing_required_paths\": []");
}
