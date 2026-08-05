use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::support::*;

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

#[cfg(unix)]
#[test]
fn stage2_task_image_python_run_executes_only_the_typed_fake_docker_plan() {
    let root = git_repo("stage2_task_image_python_run");
    fs::create_dir_all(root.join("task/environment")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("task/environment/Dockerfile"),
        "FROM python:3.13-slim\n",
    )
    .unwrap();
    fs::write(root.join("scripts/calibrate.py"), "print('calibration')\n").unwrap();

    let bin = temp_root("stage2_task_image_python_run_bin");
    let docker_log = bin.join("docker-args.log");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$FAKE_DOCKER_LOG"
case "$1" in
  build) printf 'fake-build-ok\n' ;;
  run) printf 'task-image-python-ok\n' ;;
  image) printf 'fake-image-cleanup-ok\n' ;;
  *) exit 9 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("docker")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("docker"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_DOCKER_LOG", docker_log.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let dry_run = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","args":["--cell","3"]}}}"#,
    );
    let missing_confirm = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","dry_run":false}}}"#,
    );
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","args":["--cell","3"],"timeout_secs":30,"build_timeout_secs":30,"dry_run":false,"confirm":"run task image python"}}}"#,
    );
    let log_id = started_log_id(&started);
    let completed = poll_command_log(&mut server, &log_id, 5);
    let bad_program = server.exchange(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","program":"ruby"}}}"#,
    );
    server.finish();

    assert_text(&dry_run, "\"dry_run\": true");
    assert_text(&dry_run, "\"repository_mount\": \"read-only\"");
    assert_text(&dry_run, "\"network\": \"none\"");
    assert_eq!(missing_confirm["result"]["isError"], true);
    assert_text(&missing_confirm, "requires confirm");
    assert_text(&started, "\"status\": \"running\"");
    assert_text(&started, "\"log_id\"");
    assert_text(&completed, "status: completed");
    assert_text(&completed, "\"success\": true");
    assert_text(&completed, "fake-build-ok");
    assert_text(&completed, "task-image-python-ok");
    assert_text(&completed, "fake-image-cleanup-ok");
    assert_eq!(bad_program["result"]["isError"], true);
    assert_text(&bad_program, "program must be `python3` or `python`");

    let invoked = fs::read_to_string(docker_log).unwrap();
    assert!(invoked.contains("build\n"), "{invoked}");
    assert!(invoked.contains("--name\ncontextpatch-task-"), "{invoked}");
    assert!(invoked.contains("--network\nnone\n"), "{invoked}");
    assert!(invoked.contains("--cap-drop\nALL\n"), "{invoked}");
    assert!(invoked.contains("--read-only\n"), "{invoked}");
    assert!(
        invoked.contains("/workspace/scripts/calibrate.py\n"),
        "{invoked}"
    );
    assert!(invoked.contains("--cell\n3\n"), "{invoked}");
    assert!(invoked.contains("image\nrm\n--force\n"), "{invoked}");
}

#[cfg(unix)]
#[test]
fn stage2_task_image_timeout_force_removes_the_named_container() {
    let root = git_repo("stage2_task_image_timeout_cleanup");
    fs::create_dir_all(root.join("task/environment")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("task/environment/Dockerfile"),
        "FROM python:3.13-slim\n",
    )
    .unwrap();
    fs::write(root.join("scripts/calibrate.py"), "print('calibration')\n").unwrap();

    let bin = temp_root("stage2_task_image_timeout_cleanup_bin");
    let docker_log = bin.join("docker-args.log");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$FAKE_DOCKER_LOG"
case "$1" in
  build) printf 'fake-build-ok\n' ;;
  run)
    while :; do
      sleep 1
    done
    ;;
  rm) printf 'fake-container-cleanup-ok\n' ;;
  image) printf 'fake-image-cleanup-ok\n' ;;
  *) exit 9 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("docker")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("docker"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_DOCKER_LOG", docker_log.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","timeout_secs":1,"build_timeout_secs":30,"dry_run":false,"confirm":"run task image python"}}}"#,
    );
    let completed = poll_command_log(&mut server, &started_log_id(&started), 5);
    server.finish();

    assert_text(&completed, "status: timed_out");
    assert_text(&completed, "\"status\": \"timed_out\"");
    assert_text(&completed, "\"timed_out\": true");
    assert_text(&completed, "fake-container-cleanup-ok");
    assert_text(&completed, "fake-image-cleanup-ok");

    let invoked = fs::read_to_string(docker_log).unwrap();
    assert!(invoked.contains("run\n--rm\n--name\n"), "{invoked}");
    assert!(
        invoked.contains("rm\n--force\ncontextpatch-task-"),
        "{invoked}"
    );
    assert!(invoked.contains("image\nrm\n--force\n"), "{invoked}");
}

#[cfg(unix)]
#[test]
fn stage2_task_image_signal_termination_force_removes_the_named_container() {
    let root = git_repo("stage2_task_image_signal_cleanup");
    fs::create_dir_all(root.join("task/environment")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("task/environment/Dockerfile"),
        "FROM python:3.13-slim\n",
    )
    .unwrap();
    fs::write(root.join("scripts/calibrate.py"), "print('calibration')\n").unwrap();

    let bin = temp_root("stage2_task_image_signal_cleanup_bin");
    let docker_log = bin.join("docker-args.log");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$FAKE_DOCKER_LOG"
case "$1" in
  build) printf 'fake-build-ok\n' ;;
  run) kill -TERM "$$" ;;
  rm) printf 'fake-container-cleanup-ok\n' ;;
  image) printf 'fake-image-cleanup-ok\n' ;;
  *) exit 9 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("docker")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("docker"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_DOCKER_LOG", docker_log.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","timeout_secs":30,"build_timeout_secs":30,"dry_run":false,"confirm":"run task image python"}}}"#,
    );
    let completed = poll_command_log(&mut server, &started_log_id(&started), 5);
    server.finish();

    assert_text(&completed, "status: failed");
    assert_text(&completed, "\"exit_code\": -1");
    assert_text(&completed, "fake-container-cleanup-ok");
    assert_text(&completed, "fake-image-cleanup-ok");

    let invoked = fs::read_to_string(docker_log).unwrap();
    assert!(
        invoked.contains("rm\n--force\ncontextpatch-task-"),
        "{invoked}"
    );
    assert!(invoked.contains("image\nrm\n--force\n"), "{invoked}");
}

#[cfg(unix)]
#[test]
fn stage2_task_image_docker_exit_125_force_removes_the_named_container() {
    let root = git_repo("stage2_task_image_exit_125_cleanup");
    fs::create_dir_all(root.join("task/environment")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("task/environment/Dockerfile"),
        "FROM python:3.13-slim\n",
    )
    .unwrap();
    fs::write(root.join("scripts/calibrate.py"), "print('calibration')\n").unwrap();

    let bin = temp_root("stage2_task_image_exit_125_cleanup_bin");
    let docker_log = bin.join("docker-args.log");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$FAKE_DOCKER_LOG"
case "$1" in
  build) printf 'fake-build-ok\n' ;;
  run) exit 125 ;;
  rm) printf 'fake-container-cleanup-ok\n' ;;
  image) printf 'fake-image-cleanup-ok\n' ;;
  *) exit 9 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("docker")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("docker"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("FAKE_DOCKER_LOG", docker_log.display().to_string()),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","timeout_secs":30,"build_timeout_secs":30,"dry_run":false,"confirm":"run task image python"}}}"#,
    );
    let completed = poll_command_log(&mut server, &started_log_id(&started), 5);
    server.finish();

    assert_text(&completed, "status: failed");
    assert_text(&completed, "\"exit_code\": 125");
    assert_text(&completed, "fake-container-cleanup-ok");
    assert_text(&completed, "fake-image-cleanup-ok");

    let invoked = fs::read_to_string(docker_log).unwrap();
    assert!(
        invoked.contains("rm\n--force\ncontextpatch-task-"),
        "{invoked}"
    );
    assert!(invoked.contains("image\nrm\n--force\n"), "{invoked}");
}

#[cfg(unix)]
#[test]
fn stage2_task_image_run_setup_failure_still_attempts_cleanup() {
    let root = git_repo("stage2_task_image_run_setup_failure_cleanup");
    fs::create_dir_all(root.join("task/environment")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(
        root.join("task/environment/Dockerfile"),
        "FROM python:3.13-slim\n",
    )
    .unwrap();
    fs::write(root.join("scripts/calibrate.py"), "print('calibration')\n").unwrap();

    let bin = temp_root("stage2_task_image_run_setup_failure_bin");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
set -eu
case "$1" in
  build)
    printf 'fake-build-ok\n'
    /bin/rm "$0"
    ;;
  *) exit 9 ;;
esac
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("docker")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("docker"), permissions).unwrap();

    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("PATH", bin.display().to_string()),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_image_python_run","arguments":{"script":"scripts/calibrate.py","timeout_secs":30,"build_timeout_secs":30,"dry_run":false,"confirm":"run task image python"}}}"#,
    );
    let completed = poll_command_log(&mut server, &started_log_id(&started), 5);
    server.finish();

    assert_text(&completed, "status: failed");
    assert_text(&completed, "failed to run task image Python");
    assert_text(&completed, "failed to run task image container");
    assert_text(&completed, "failed to run task image execution tag");
    assert_text(&completed, "\"container_cleanup\": {");
    assert_text(&completed, "\"image_cleanup\": {");
}

#[cfg(unix)]
#[test]
fn stage2_harbor_run_start_polls_and_returns_structured_evidence() {
    let root = git_repo("stage2_harbor_run_start");
    fs::create_dir(root.join("task")).unwrap();
    fs::create_dir(root.join(r"task\inner")).unwrap();
    let bin = temp_root("stage2_harbor_run_start_bin");
    fs::write(
        bin.join("harbor"),
        r#"#!/bin/sh
set -eu
: > harbor-started
while [ ! -f harbor-release ]; do
  sleep 0.05
done
job="jobs/fake-job"
trial="$job/task__trial"
mkdir -p "$trial/verifier"
printf '%s\n' '{"stats":{"evals":{"task":{"reward_stats":{"reward":{"1.0":["task__trial"]}}}}}}' > "$job/result.json"
printf '%s\n' '{"agent_info":{"name":"oracle"},"started_at":"2026-01-01T00:00:00Z","finished_at":"2026-01-01T00:00:01Z","exception_info":null}' > "$trial/result.json"
printf '%s\n' '1.0' > "$trial/verifier/reward.txt"
printf '%s\n' 'calibration-ok' 'Authorization: Bearer harbor-secret-value' > "$trial/verifier/test-stdout.txt"
printf '%s\n' 'Results written to jobs/fake-job/result.json'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("harbor")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("harbor"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);

    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task","agent":"oracle","timeout_secs":30}}}"#,
    );
    let start_document: Value = serde_json::from_str(response_text(&started)).unwrap();
    assert_eq!(start_document["status"], "running");
    let log_id = start_document["log_id"].as_str().unwrap().to_string();

    for _ in 0..100 {
        if root.join("harbor-started").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(root.join("harbor-started").exists());

    let poll_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "read_command_log",
            "arguments": {"log_id": log_id}
        }
    })
    .to_string();
    let running = server.exchange(&poll_request);
    assert_text(&running, "status: running");

    let second_server_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "read_command_log",
            "arguments": {"log_id": log_id}
        }
    })
    .to_string();
    let second_server = run_server(&root, &[&second_server_request]);
    assert_text(&second_server[0], "status: unknown");

    fs::write(root.join("harbor-release"), "").unwrap();
    let completed = poll_command_log(&mut server, &log_id, 2);
    let text = response_text(&completed);
    assert!(text.contains("\"available\": true"), "{text}");
    assert!(text.contains("\"result_path\": \"jobs/fake-job/result.json\""));
    assert!(text.contains("\"job_path\": \"jobs/fake-job\""));
    assert!(text.contains("\"trial_path\": \"jobs/fake-job/task__trial\""));
    assert!(text.contains("\"rewards\": [\n      1.0"));
    assert!(text.contains("\"agent\": \"oracle\""));
    assert!(text.contains("calibration-ok"));
    assert!(text.contains("[redacted potential secret line]"));
    assert!(!text.contains("harbor-secret-value"));

    let bad_agent = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task","agent":"-oracle"}}}"#,
    );
    assert_eq!(bad_agent["result"]["isError"], true);
    assert_text(&bad_agent, "agent");
    let backslash_project = server.exchange(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"harbor_run_start","arguments":{"project":"task\\inner","agent":"oracle"}}}"#,
    );
    assert_eq!(backslash_project["result"]["isError"], true);
    assert_text(&backslash_project, "must use `/` separators");
    server.finish();
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
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"preflight_health","arguments":{"response_mode":"compact"}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"preflight_health","arguments":{"response_mode":"minimal"}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"preflight_health","arguments":{"response_mode":"verbose"}}}"#,
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
    assert_text(&responses[0], "\"pr_comments\"");
    assert_text(&responses[0], "\"workflow_job_log\"");
    assert_text(&responses[0], "\"workflow_runs_for_commit\"");
    assert_text(&responses[0], "\"repository_targeting\"");
    assert_text(&responses[0], "\"github_fork_prepare\"");
    assert_text(&responses[0], "\"fixture_generator_run\"");
    assert_text(&responses[0], "\"base_image_check_run\"");
    assert_text(&responses[0], "\"bulk_write_new_files_base64\"");
    assert_text(&responses[0], "\"write_existing_file_exact_hash\"");
    assert_text(&responses[0], "\"fixture_manifest_verify\"");
    assert_text(&responses[0], "\"fixture_manifest_refresh\"");
    assert_text(&responses[0], "\"dynamo-harbor-task\"");
    assert_text(&responses[0], "\"move_tracked\": true");
    assert_text(&responses[0], "\"delete_guarded\": true");
    assert_text(&responses[0], "\"artifact_delete_exact\": true");
    assert_text(&responses[0], "\"git_subprocess_timeout\": 90");
    assert_text(&responses[0], "\"concurrent_tool_calls\": true");
    assert_text(&responses[0], "\"max_active_tool_calls\": 16");
    assert_text(&responses[0], "\"responses_may_arrive_out_of_order\": true");
    assert_text(&responses[0], "\"correlate_by\": \"JSON-RPC id\"");
    assert_text(&responses[0], "\"max_active\": 2");
    assert_text(&responses[0], "\"harbor_run_max_timeout_secs\": 3600");
    assert_text(&responses[0], "\"action\": \"ios_build\"");
    assert_text(&responses[1], "\"harbor_run_max_timeout_secs\": 3600");
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
    assert_text(&responses[1], "\"response_mode\": \"full\"");
    assert_text(&responses[1], "\"sample_truncated\"");
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

    let compact: Value = serde_json::from_str(response_text(&responses[6])).unwrap();
    assert_eq!(compact["response_mode"], "compact");
    assert!(compact["validation_tools"]["git"].is_boolean());
    assert!(compact["native_build"]["required_tools"]["xcodebuild"].is_boolean());

    let minimal: Value = serde_json::from_str(response_text(&responses[7])).unwrap();
    assert_eq!(minimal["response_mode"], "minimal");
    assert!(minimal["validation_tools"]["total"].is_number());
    assert_eq!(
        minimal["repository"]["sampled_changes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    assert_eq!(responses[8]["result"]["isError"], true);
    assert_text(
        &responses[8],
        "response_mode must be one of minimal, compact, full",
    );
}

#[test]
fn stage2_validation_profile_writes_readable_command_logs() {
    let root = git_repo("stage2_validation_profile_writes_readable_command_logs");
    let mut server = ServerExchange::spawn(&root, &[], &[]);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"validation_profile_run","arguments":{"profile":"repo-basic","timeout_secs":30}}}"#,
    );
    assert_text(&started, "\"profile\": \"repo-basic\"");
    assert_text(&started, "\"status\": \"running\"");
    let log_id = started_log_id(&started);
    let completed = poll_command_log(&mut server, &log_id, 2);
    let command_log_id = response_text(&completed)
        .lines()
        .find(|line| line.starts_with("1. "))
        .and_then(|line| line.rsplit_once("log_id: "))
        .map(|(_, log_id)| log_id)
        .unwrap();
    let command_log_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "read_command_log",
            "arguments": {"log_id": command_log_id}
        }
    })
    .to_string();
    let command_log = server.exchange(&command_log_request);
    server.finish();

    assert_text(&completed, "profile: repo-basic");
    assert_text(&completed, "failed: false");
    assert_text(&completed, "git status --branch --short");
    assert_text(&completed, "git diff --check");
    assert_text(&completed, &format!("log_id: {log_id}"));
    assert_text(&completed, "timed_out: false");
    assert_text(&command_log, &format!("log_id: {command_log_id}"));
    assert_text(&command_log, "allowlist: git/status");
    assert_text(&command_log, "timed_out: false");
}

#[test]
fn stage2_dynamo_harbor_profile_reports_structured_rewards() {
    let root = git_repo("stage2_dynamo_harbor_profile_reports_structured_rewards");
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(
        root.join("references/check-base-image.sh"),
        "#!/bin/sh\nexit 0\n",
    )
    .unwrap();

    let bin = temp_root("stage2_dynamo_harbor_profile_bin");
    fs::write(
        bin.join("harbor"),
        "#!/bin/sh\nset -eu\ncase \"$*\" in\n  *\"--agent oracle\"*) agent=oracle; reward=1.0 ;;\n  *\"--agent nop\"*) agent=nop; reward=0.0 ;;\n  *) agent=unknown; reward=0.5 ;;\nesac\ndir=\"jobs/$agent\"\nmkdir -p \"$dir\"\nprintf '{\"stats\":{\"evals\":{\"%s__adhoc\":{\"reward_stats\":{\"reward\":{\"%s\":[\"task__trial\"]}}}}}}\\n' \"$agent\" \"$reward\" > \"$dir/result.json\"\nprintf '| Reward | Count |\\n| 0.5 | 1 |\\n'\nprintf 'Results written to %s/result.json\\n' \"$dir\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("harbor")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("harbor"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{original_path}", bin.display());
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("PATH", test_path),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"validation_profile_run","arguments":{"profile":"dynamo-harbor-task"}}}"#,
    );
    let log_id = started_log_id(&started);
    let completed = poll_command_log(&mut server, &log_id, 5);
    let direct_harbor = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"harbor","args":["run"],"timeout_secs":3600}}}"#,
    );
    let excessive_timeout = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"git","args":["status"],"timeout_secs":601}}}"#,
    );
    let direct_harbor_excessive_timeout = server.exchange(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"harbor","args":["run"],"timeout_secs":3601}}}"#,
    );
    server.finish();

    assert_text(&started, "\"profile\": \"dynamo-harbor-task\"");
    assert_text(&started, "\"status\": \"running\"");
    assert_text(&completed, "profile: dynamo-harbor-task");
    assert_text(&completed, "failed: false");
    assert_text(&completed, "harbor_summary:");
    assert_text(&completed, "\"oracle_rewards\":[1.0,1.0]");
    assert_text(&completed, "\"nop_rewards\":[0.0,0.0]");
    assert_text(&completed, "\"oracle_all_one\":true");
    assert_text(&completed, "\"nop_all_below_one\":true");
    assert_text(&completed, "\"oracle_deterministic\":true");
    assert_text(&completed, "\"nop_deterministic\":true");
    assert_text(&completed, "\"passed\":true");
    assert_text(
        &completed,
        "bash \"references/check-base-image.sh\" task | timeout_secs: 600",
    );
    assert_text(
        &completed,
        "harbor run -p task --agent oracle | timeout_secs: 3600",
    );
    assert_eq!(direct_harbor["result"]["isError"], true);
    assert_text(&direct_harbor, "use harbor_run_start");
    assert_eq!(excessive_timeout["result"]["isError"], true);
    assert_text(&excessive_timeout, "between 1 and 600");
    assert_eq!(direct_harbor_excessive_timeout["result"]["isError"], true);
    assert_text(&direct_harbor_excessive_timeout, "use harbor_run_start");
}

#[test]
fn stage2_dynamo_harbor_profile_semantic_failure_sets_failed_status() {
    let root = git_repo("stage2_dynamo_harbor_profile_semantic_failure");
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(
        root.join("references/check-base-image.sh"),
        "#!/bin/sh\nexit 0\n",
    )
    .unwrap();

    let bin = temp_root("stage2_dynamo_harbor_profile_semantic_failure_bin");
    fs::write(
        bin.join("harbor"),
        "#!/bin/sh\nset -eu\ncase \"$*\" in\n  *\"--agent oracle\"*) agent=oracle; reward=0.5 ;;\n  *\"--agent nop\"*) agent=nop; reward=0.0 ;;\n  *) agent=unknown; reward=0.5 ;;\nesac\ndir=\"jobs/$agent\"\nmkdir -p \"$dir\"\nprintf '{\"stats\":{\"evals\":{\"%s__adhoc\":{\"reward_stats\":{\"reward\":{\"%s\":[\"task__trial\"]}}}}}}\\n' \"$agent\" \"$reward\" > \"$dir/result.json\"\nprintf 'Results written to %s/result.json\\n' \"$dir\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(bin.join("harbor")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(bin.join("harbor"), permissions).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    let envs = [
        (
            "CONTEXTPATCH_VALIDATION_PATHS",
            bin.to_str().unwrap().to_string(),
        ),
        ("PATH", format!("{}:{original_path}", bin.display())),
    ];
    let mut server = ServerExchange::spawn(&root, &[], &envs);
    let started = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"validation_profile_run","arguments":{"profile":"dynamo-harbor-task"}}}"#,
    );
    let completed = poll_command_log(&mut server, &started_log_id(&started), 5);
    server.finish();

    assert_text(&completed, "status: failed");
    assert_text(&completed, "failed: true");
    assert_text(&completed, "\"oracle_rewards\":[0.5,0.5]");
    assert_text(&completed, "\"oracle_all_one\":false");
    assert_text(&completed, "\"passed\":false");
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
