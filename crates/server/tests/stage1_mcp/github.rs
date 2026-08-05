use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::support::*;

#[test]
fn stage2_github_workflows_target_upstream_and_read_execution_evidence() {
    let root = git_repo("stage2_github_workflows_target_upstream");
    let bin = temp_root("stage2_github_workflows_target_upstream_bin");
    fs::write(
        bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf '%s\\n' 'DATACORE_TOKEN=super-secret-value'\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("gh")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("gh"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{original_path}", bin.display());
    let responses = run_server_with_env(
        &root,
        &[("PATH", test_path)],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_view","number":22,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_checks","number":22,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_run_view","run_id":12345,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_job_log","job_id":67890,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_view","number":22,"repository":"upstream/project/extra"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"auth_status","repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_run_rerun_failed","run_id":12345,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_run_rerun_failed","run_id":12345,"repository":"upstream/project","dry_run":false,"confirm":"wrong"}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_run_rerun_failed","run_id":12345,"repository":"upstream/project","dry_run":false,"confirm":"rerun failed workflow jobs"}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_runs_for_commit","head_sha":"0123456789abcdef0123456789abcdef01234567","limit":10,"repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_runs_for_commit","head_sha":"efacda3","repository":"upstream/project"}}}"#,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_job_log","job_id":67890,"repository":"upstream/project","log_view":"head"}}}"#,
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"workflow_job_log","job_id":67890,"repository":"upstream/project","log_view":"middle"}}}"#,
            r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_view","number":22,"repository":"upstream/project","log_view":"tail"}}}"#,
        ],
    );

    assert_text(&responses[0], "headRefOid");
    assert_text(&responses[0], "comments,reviews,statusCheckRollup");
    assert_text(&responses[0], "--repo\\nupstream/project");
    assert_text(
        &responses[1],
        "bucket,completedAt,description,event,link,name",
    );
    assert_text(&responses[1], "--repo\\nupstream/project");
    assert_text(&responses[2], "attempt,conclusion,createdAt,databaseId");
    assert_text(&responses[2], "12345");
    assert_text(&responses[3], "--job\\n67890\\n--log");
    assert_text(&responses[3], "\"stdout_truncated\": false");
    assert_text(&responses[3], "\"log_view\": \"tail\"");
    assert_text(&responses[3], "[redacted potential secret line]");
    assert!(!response_text(&responses[3]).contains("super-secret-value"));
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "repository must use OWNER/REPO");
    assert_eq!(responses[5]["result"]["isError"], true);
    assert_text(&responses[5], "repository is not accepted for auth_status");
    assert_text(&responses[6], "\"dry_run\": true");
    assert_text(&responses[6], "run");
    assert_text(&responses[6], "rerun");
    assert_text(&responses[6], "--failed");
    assert_eq!(responses[7]["result"]["isError"], true);
    assert_text(&responses[7], "confirm must be");
    assert_text(&responses[8], "rerun\\n12345\\n--failed");
    assert_text(&responses[8], "[redacted potential secret line]");
    assert_text(
        &responses[9],
        "list\\n--commit\\n0123456789abcdef0123456789abcdef01234567",
    );
    assert_text(&responses[9], "--limit\\n10");
    assert_text(&responses[9], "--repo\\nupstream/project");
    assert_eq!(responses[10]["result"]["isError"], true);
    assert_text(&responses[10], "full 40-character hexadecimal commit SHA");
    assert_text(&responses[11], "\"log_view\": \"head\"");
    assert_eq!(responses[12]["result"]["isError"], true);
    assert_text(&responses[12], "log_view must be one of head, tail");
    assert_eq!(responses[13]["result"]["isError"], true);
    assert_text(
        &responses[13],
        "log_view is accepted only for workflow_job_log",
    );
}

#[test]
fn stage2_github_workflows_filter_sticky_pr_comments() {
    let root = git_repo("stage2_github_workflows_filter_sticky_comments");
    let bin = temp_root("stage2_github_workflows_filter_sticky_comments_bin");
    fs::write(
        bin.join("gh"),
        r#"#!/bin/sh
cat <<'EOF'
{"comments":[{"author":{"login":"bot"},"body":"pass@5 old difficulty finding","createdAt":"2026-07-01T00:00:00Z","url":"https://example.test/old"},{"author":{"login":"other"},"body":"unrelated note","createdAt":"2026-07-30T00:00:00Z","url":"https://example.test/unrelated"},{"author":{"login":"bot"},"body":"pass@2 infrastructure error; Rerun Recommended: YES","createdAt":"2026-07-31T00:00:00Z","url":"https://example.test/new"}]}
EOF
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("gh")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("gh"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{original_path}", bin.display());
    let responses = run_server_with_env(
        &root,
        &[("PATH", test_path)],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_comments","number":22,"repository":"upstream/project","comment_contains":"pass@","limit":1}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_comments","number":22,"comment_contains":"  "}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"github_pr_run","arguments":{"action":"pr_comments","number":22,"limit":101}}}"#,
        ],
    );

    assert_text(&responses[0], "\\\"matched_count\\\": 2");
    assert_text(&responses[0], "\\\"returned_count\\\": 1");
    assert_text(&responses[0], "pass@2 infrastructure error");
    assert!(!response_text(&responses[0]).contains("pass@5 old"));
    assert!(!response_text(&responses[0]).contains("unrelated note"));
    assert_text(&responses[0], "--repo");
    assert_text(&responses[0], "upstream/project");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "comment_contains must contain 1 to 256");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(
        &responses[2],
        "limit for pr_comments must be between 1 and 100",
    );
}
