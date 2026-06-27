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
        "git_commit_exact",
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
    assert_text(&responses[1], "\"guarded_process_execution\"");
    assert_text(&responses[2], "allowlist: git/status");
    assert_text(&responses[2], "exit_code: 0");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "not allowlisted");
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
    writeln!(
        stdin,
        "{}",
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["status","--branch","--short"],"timeout_secs":30}}}"#
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
    writeln!(
        stdin,
        "{}",
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"rg","args":["--files","clients/vscode/test"],"timeout_secs":30}}}"#
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

fn temp_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}
