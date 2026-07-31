use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn contextpatch() -> &'static str {
    env!("CARGO_BIN_EXE_contextpatch")
}

#[test]
fn stage1_cli_tools_work_together() {
    let root = git_repo("stage1_cli_tools_work_together");
    fs::write(root.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let status = run_ok(&root, &["status-guard"]);
    assert_eq!(status.stdout, "clean: no Git changes\n");

    let range = run_ok(
        &root,
        &["read-range", "sample.txt", "--start", "2", "--end", "3"],
    );
    assert_eq!(range.stdout, "2. beta\n3. gamma\n");

    let diff = run_ok(
        &root,
        &[
            "diff-preview",
            "sample.txt",
            "--old",
            "beta",
            "--new",
            "delta",
        ],
    );
    assert!(diff.stdout.contains("-beta\n"));
    assert!(diff.stdout.contains("+delta\n"));
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha\nbeta\ngamma\n"
    );

    let replace = run_ok(
        &root,
        &[
            "replace-exact",
            "sample.txt",
            "--old",
            "beta",
            "--new",
            "delta",
        ],
    );
    assert!(replace.stdout.contains("replaced bytes"));
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );

    let directory = run_ok(
        &root,
        &[
            "create-directory",
            "native-plugins/background-audio",
            "--parents",
        ],
    );
    assert!(directory.stdout.contains("created directory"));

    let create = run_ok(
        &root,
        &[
            "write-new-file",
            "native-plugins/background-audio/plugin.ts",
            "--content",
            "new file\n",
        ],
    );
    assert!(create.stdout.contains("created"));
    assert_eq!(
        fs::read_to_string(root.join("native-plugins/background-audio/plugin.ts")).unwrap(),
        "new file\n"
    );

    let dirty = run_err(&root, &["status-guard"]);
    assert!(dirty.stderr.contains("status-guard refused"));
    assert!(dirty.stderr.contains("sample.txt"));
    assert!(dirty
        .stderr
        .contains("native-plugins/background-audio/plugin.ts"));
}

#[test]
fn stage1_cli_refusals_are_visible() {
    let root = git_repo("stage1_cli_refusals_are_visible");
    fs::write(root.join("sample.txt"), "beta beta\n").unwrap();

    let ambiguous = run_err(
        &root,
        &[
            "replace-exact",
            "sample.txt",
            "--old",
            "beta",
            "--new",
            "delta",
        ],
    );
    assert!(ambiguous.stderr.contains("expected exactly one match"));
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta beta\n"
    );

    let existing = run_err(
        &root,
        &["write-new-file", "sample.txt", "--content", "replacement"],
    );
    assert!(existing.stderr.contains("already exists"));
    fs::create_dir(root.join("existing")).unwrap();
    let existing_dir = run_err(&root, &["create-directory", "existing"]);
    assert!(existing_dir.stderr.contains("already exists"));
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta beta\n"
    );
}

#[test]
fn configure_claude_desktop_allows_every_contextpatch_tool() {
    let root = temp_root("configure_claude_desktop_allows_every_contextpatch_tool");
    let config = root.join("claude_desktop_config.json");
    let original = r#"{
  "mcpServers": {
    "contextpatch-one": {
      "command": "/opt/contextpatch-server",
      "args": ["--repo-root", "/repo/one"],
      "toolPolicy": {"replace_exact": "ask"}
    },
    "contextpatch-two": {
      "command": "contextpatch-server",
      "args": ["--repo-root", "/repo/two"]
    },
    "contextpatch-windows": {
      "command": "C:\\Program Files\\ContextPatch\\contextpatch-server.exe",
      "args": ["--repo-root", "C:\\repo\\three"]
    },
    "unrelated": {
      "command": "/opt/other-server",
      "toolPolicy": {"*": "blocked"}
    }
  },
  "theme": "dark"
}"#;
    fs::write(&config, original).unwrap();

    let output = run_ok_without_default_config_env(
        &root,
        &[
            "configure-claude-desktop",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert!(output.stdout.contains("allowed all tools for 3"));
    assert!(output.stdout.contains("restart Claude Desktop"));

    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        updated["mcpServers"]["contextpatch-one"]["toolPolicy"],
        serde_json::json!({"*": "allow"})
    );
    assert_eq!(
        updated["mcpServers"]["contextpatch-two"]["toolPolicy"],
        serde_json::json!({"*": "allow"})
    );
    assert_eq!(
        updated["mcpServers"]["contextpatch-windows"]["toolPolicy"],
        serde_json::json!({"*": "allow"})
    );
    assert_eq!(
        updated["mcpServers"]["unrelated"]["toolPolicy"],
        serde_json::json!({"*": "blocked"})
    );
    assert_eq!(updated["theme"], "dark");

    let backups = fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("claude_desktop_config.json.contextpatch-backup-")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original.as_bytes());

    let second = run_ok(
        &root,
        &[
            "configure-claude-desktop",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert!(second.stdout.contains("already allowed"));
    let backups_after_second_run = fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("claude_desktop_config.json.contextpatch-backup-")
        })
        .count();
    assert_eq!(backups_after_second_run, 1);
}

#[test]
fn configure_claude_desktop_refuses_invalid_config_without_changing_it() {
    let root = temp_root("configure_claude_desktop_refuses_invalid_config_without_changing_it");
    let config = root.join("claude_desktop_config.json");
    fs::write(&config, b"{not json}\n").unwrap();

    let output = run_err(
        &root,
        &[
            "configure-claude-desktop",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert!(output.stderr.contains("not valid JSON"));
    assert_eq!(fs::read(&config).unwrap(), b"{not json}\n");
    let backups = fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("claude_desktop_config.json.contextpatch-backup-")
        })
        .count();
    assert_eq!(backups, 0);
}

struct OutputText {
    stdout: String,
    stderr: String,
}

fn run_ok(root: &Path, args: &[&str]) -> OutputText {
    let output = command(root, args);
    assert!(
        output.status.success(),
        "expected success for {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_text(output)
}

fn run_err(root: &Path, args: &[&str]) -> OutputText {
    let output = command(root, args);
    assert!(
        !output.status.success(),
        "expected refusal for {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_text(output)
}

fn run_ok_without_default_config_env(root: &Path, args: &[&str]) -> OutputText {
    let output = Command::new(contextpatch())
        .current_dir(root)
        .env_remove("HOME")
        .env_remove("APPDATA")
        .env_remove("XDG_CONFIG_HOME")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "expected success for {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_text(output)
}

fn command(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(contextpatch())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn output_text(output: std::process::Output) -> OutputText {
    OutputText {
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
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
