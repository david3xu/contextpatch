#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use crate::support::*;

#[cfg(unix)]
#[test]
fn long_command_does_not_block_a_fast_read() {
    let root = git_repo("long_command_does_not_block_a_fast_read");
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("scripts/slow.py"),
        "print('unused by fake python')\n",
    )
    .unwrap();
    fs::write(root.join("sample.txt"), "fast response\n").unwrap();

    let bin = temp_root("long_command_does_not_block_a_fast_read_bin");
    fs::write(
        bin.join("python3"),
        r#"#!/bin/sh
set -eu
: > "$FAKE_SLOW_STARTED"
sleep 2
printf 'slow-command-finished\n'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("python3")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("python3"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let started_marker = root.join("slow-command-started");
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_SLOW_STARTED", started_marker.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    server.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"python3","args":["scripts/slow.py"],"timeout_secs":10}}}"#,
    );

    for _ in 0..100 {
        if started_marker.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(started_marker.exists(), "slow command never started");

    server.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}"#,
    );
    let fast = server.read();
    assert_eq!(
        fast["id"], 2,
        "fast read was blocked behind the slow command"
    );
    assert_text(&fast, "fast response");

    let slow = server.read();
    assert_eq!(slow["id"], 1);
    assert_text(&slow, "slow-command-finished");
    server.finish();
}

#[cfg(unix)]
#[test]
fn slow_git_tool_does_not_block_a_later_read() {
    let root = git_repo("slow_git_tool_does_not_block_a_later_read");
    fs::write(root.join("sample.txt"), "fast response\n").unwrap();

    let bin = temp_root("slow_git_tool_does_not_block_a_later_read_bin");
    fs::write(
        bin.join("git"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--no-pager" ]; then
  shift
fi
if [ "$1" = "status" ]; then
  : > "$FAKE_SLOW_STARTED"
  sleep 2
  exit 0
fi
exit 9
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("git")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("git"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let started_marker = root.join("slow-git-started");
    let envs = [
        ("FAKE_SLOW_STARTED", started_marker.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    server.send(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"status_guard","arguments":{}}}"#,
    );

    for _ in 0..100 {
        if started_marker.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(started_marker.exists(), "slow Git call never started");

    server.send(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}"#,
    );
    let fast = server.read();
    assert_eq!(fast["id"], 2, "read was blocked behind the Git tool call");
    assert_text(&fast, "fast response");

    let slow = server.read();
    assert_eq!(slow["id"], 1);
    assert_text(&slow, "clean: no Git changes");
    server.finish();
}

#[cfg(unix)]
#[test]
fn background_workflows_share_a_two_job_limit() {
    let root = git_repo("background_workflows_share_a_two_job_limit");
    fs::create_dir(root.join("task")).unwrap();

    let bin = temp_root("background_workflows_share_a_two_job_limit_bin");
    fs::write(
        bin.join("harbor"),
        r#"#!/bin/sh
set -eu
while [ ! -f "$FAKE_HARBOR_RELEASE" ]; do
  sleep 0.05
done
printf 'fake harbor complete\n'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("harbor")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("harbor"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let release = root.join("harbor-release");
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_HARBOR_RELEASE", release.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let first = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task","agent":"oracle","timeout_secs":30}}}"#,
    );
    let second = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task","agent":"nop","timeout_secs":30}}}"#,
    );
    let third = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task","agent":"oracle","timeout_secs":30}}}"#,
    );

    let first_log_id = started_log_id(&first);
    let second_log_id = started_log_id(&second);
    assert_eq!(third["result"]["isError"], true);
    assert_text(&third, "at most 2");
    assert_text(&third, "poll existing log_ids");

    fs::write(release, "").unwrap();
    let first_completed = poll_command_log(&mut server, &first_log_id, 4);
    let second_completed = poll_command_log(&mut server, &second_log_id, 5);
    assert_text(&first_completed, "status: completed");
    assert_text(&second_completed, "status: completed");
    server.finish();
}

#[cfg(unix)]
#[test]
fn broken_stdout_stops_server_while_stdin_remains_open() {
    let root = git_repo("broken_stdout_stops_server_while_stdin_remains_open");
    fs::write(root.join("sample.txt"), "response\n").unwrap();

    let mut child = Command::new(contextpatch_server())
        .arg("--repo-root")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    drop(child.stdout.take().unwrap());
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"read_range","arguments":{{"path":"sample.txt","start_line":1,"end_line":1}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            panic!("server remained blocked on open stdin after stdout failed");
        }
        thread::sleep(Duration::from_millis(20));
    };

    drop(stdin);
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!status.success(), "server unexpectedly succeeded");
    assert!(
        stderr.contains("failed to write stdout"),
        "missing broken-stdout diagnostic: {stderr:?}"
    );
}
