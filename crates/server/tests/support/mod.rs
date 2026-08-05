use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn contextpatch_server() -> &'static str {
    env!("CARGO_BIN_EXE_contextpatch-server")
}

pub(crate) fn run_server(root: &Path, requests: &[&str]) -> Vec<Value> {
    run_server_pipelined_with_options(root, &[], &[], requests)
}

pub(crate) fn run_server_sequential(root: &Path, requests: &[&str]) -> Vec<Value> {
    run_server_sequential_with_options(root, &[], &[], requests)
}

pub(crate) fn run_server_project(root: &Path, requests: &[&str]) -> Vec<Value> {
    run_server_pipelined_with_options(root, &["--tool-surface", "project"], &[], requests)
}

pub(crate) fn run_server_project_sequential(root: &Path, requests: &[&str]) -> Vec<Value> {
    run_server_sequential_with_options(root, &["--tool-surface", "project"], &[], requests)
}

pub(crate) fn run_server_with_env(
    root: &Path,
    envs: &[(&str, String)],
    requests: &[&str],
) -> Vec<Value> {
    run_server_pipelined_with_options(root, &[], envs, requests)
}

fn run_server_pipelined_with_options(
    root: &Path,
    options: &[&str],
    envs: &[(&str, String)],
    requests: &[&str],
) -> Vec<Value> {
    let mut server = ServerExchange::spawn(root, options, envs);
    for request in requests {
        server.send(request);
    }
    let mut responses = (0..requests.len())
        .map(|_| server.read())
        .collect::<Vec<_>>();
    responses.sort_by_key(|response| {
        response["id"]
            .as_i64()
            .expect("test response must carry a numeric JSON-RPC id")
    });
    server.finish();
    responses
}

fn run_server_sequential_with_options(
    root: &Path,
    options: &[&str],
    envs: &[(&str, String)],
    requests: &[&str],
) -> Vec<Value> {
    let mut server = ServerExchange::spawn(root, options, envs);
    let responses = requests
        .iter()
        .map(|request| server.exchange(request))
        .collect();
    server.finish();
    responses
}

pub(crate) struct ServerExchange {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

impl ServerExchange {
    pub(crate) fn spawn(root: &Path, options: &[&str], envs: &[(&str, String)]) -> Self {
        let mut child = Command::new(contextpatch_server())
            .arg("--repo-root")
            .arg(root)
            .args(options)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(envs.iter().map(|(key, value)| (*key, value)))
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            reader,
        }
    }

    pub(crate) fn send(&mut self, request: &str) {
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
    }

    pub(crate) fn read(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "server exited before replying");
        serde_json::from_str(&line).unwrap()
    }

    pub(crate) fn exchange(&mut self, request: &str) -> Value {
        self.send(request);
        self.read()
    }

    pub(crate) fn finish(self) {
        let Self {
            child,
            stdin,
            reader,
        } = self;
        drop(stdin);
        drop(reader);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "server failed\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

pub(crate) fn started_log_id(response: &Value) -> String {
    let document: Value = serde_json::from_str(response_text(response)).unwrap();
    document["log_id"].as_str().unwrap().to_string()
}

pub(crate) fn poll_command_log(
    server: &mut ServerExchange,
    log_id: &str,
    request_id: i64,
) -> Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/call",
        "params": {
            "name": "read_command_log",
            "arguments": {"log_id": log_id}
        }
    })
    .to_string();
    for _ in 0..250 {
        let response = server.exchange(&request);
        if !response_text(&response).contains("status: running") {
            return response;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("background job {log_id} did not finish");
}

pub(crate) fn poll_project_command_log(
    server: &mut ServerExchange,
    repository: &str,
    log_id: &str,
    request_id: i64,
) -> Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/call",
        "params": {
            "name": "project_execute",
            "arguments": {
                "repository": repository,
                "action": "read_command_log",
                "arguments": {"log_id": log_id}
            }
        }
    })
    .to_string();
    for _ in 0..250 {
        let response = server.exchange(&request);
        if !response_text(&response).contains("status: running") {
            return response;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("background job {log_id} did not finish");
}

pub(crate) fn assert_text(response: &Value, expected: &str) {
    let text = response_text(response);
    assert!(
        text.contains(expected),
        "expected response text to contain {expected:?}, got {text:?}"
    );
}

pub(crate) fn response_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"].as_str().unwrap()
}

pub(crate) fn git_repo(name: &str) -> PathBuf {
    let root = temp_root(name);
    init_git_repo(&root);
    root
}

pub(crate) fn init_git_repo(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Contextpatch Test"]);
    git(
        root,
        &["config", "user.email", "contextpatch@example.invalid"],
    );
}

pub(crate) fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> String {
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

pub(crate) fn bare_repo(name: &str) -> PathBuf {
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

pub(crate) fn temp_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

pub(crate) fn sha256_hex_for_test(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
