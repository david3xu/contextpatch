use std::collections::BTreeMap;
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
fn configure_claude_desktop_cleans_exact_legacy_policy_and_preserves_custom_policy() {
    let root = temp_root("configure_claude_desktop_keeps_ordinary_servers");
    let config = root.join("claude_desktop_config.json");
    let library = deprecated_config_library_path(&root);
    fs::create_dir_all(&library).unwrap();
    fs::write(
        library.join("_meta.json"),
        b"{this deliberately is not valid JSON}\n",
    )
    .unwrap();
    let library_before = snapshot_tree(&library);
    let original = r#"{
  "mcpServers": {
    "contextpatch-one": {
      "command": "/opt/contextpatch-server",
      "args": ["--repo-root", "/repo/one"],
      "toolPolicy": {"*": "allow"},
      "env": {"RUST_LOG": "info"}
    },
    "contextpatch-two": {
      "command": "contextpatch-server",
      "args": ["--repo-root", "/repo/two"],
      "toolPolicy": {"replace_exact": "ask"}
    },
    "contextpatch-windows": {
      "command": "C:\\Program Files\\ContextPatch\\contextpatch-server.exe",
      "args": ["--repo-root", "C:\\repo\\three"]
    },
    "unrelated": {
      "command": "/opt/other-server",
      "toolPolicy": {"*": "blocked"},
      "custom": true
    }
  },
  "theme": "dark",
  "otherData": {"keep": [1, 2, 3]}
}"#;
    let original_json: serde_json::Value = serde_json::from_str(original).unwrap();
    let custom_policy_before =
        serde_json::to_vec(&original_json["mcpServers"]["contextpatch-two"]["toolPolicy"]).unwrap();
    fs::write(&config, original).unwrap();

    let output = run_ok_with_config_env(&root, &configure_args(&config, &[]));
    assert!(output.stdout.contains("normal `mcpServers` map"));
    assert!(output
        .stdout
        .contains("removed the exact legacy ContextPatch wildcard `toolPolicy` from 1"));
    assert!(output
        .stdout
        .contains("local ContextPatch MCP connection requires no authentication"));
    assert!(output.stdout.contains("project_execute"));
    assert!(output
        .stdout
        .contains("restart Claude Desktop to reload the updated configuration"));

    let updated = read_json(&config);
    let servers = updated["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 4);
    assert_eq!(
        servers["contextpatch-one"]["command"],
        "/opt/contextpatch-server"
    );
    assert_eq!(
        servers["contextpatch-one"]["args"],
        serde_json::json!(["--repo-root", "/repo/one", "--tool-surface", "project"])
    );
    assert_eq!(
        servers["contextpatch-one"]["env"],
        serde_json::json!({"RUST_LOG": "info"})
    );
    assert_eq!(
        servers["contextpatch-two"]["args"],
        serde_json::json!(["--repo-root", "/repo/two", "--tool-surface", "project"])
    );
    assert_eq!(
        servers["contextpatch-windows"]["args"],
        serde_json::json!([
            "--repo-root",
            "C:\\repo\\three",
            "--tool-surface",
            "project"
        ])
    );
    assert!(servers["contextpatch-one"].get("toolPolicy").is_none());
    assert_eq!(
        servers["contextpatch-two"]["toolPolicy"],
        serde_json::json!({"replace_exact": "ask"})
    );
    assert_eq!(
        serde_json::to_vec(&servers["contextpatch-two"]["toolPolicy"]).unwrap(),
        custom_policy_before
    );
    assert!(servers["contextpatch-windows"].get("toolPolicy").is_none());
    assert_eq!(
        servers["unrelated"]["toolPolicy"],
        serde_json::json!({"*": "blocked"})
    );
    assert_eq!(servers["unrelated"]["custom"], true);
    assert_eq!(updated["theme"], "dark");
    assert_eq!(updated["otherData"], serde_json::json!({"keep": [1, 2, 3]}));
    assert_eq!(snapshot_tree(&library), library_before);

    let backups = backups_for(&config);
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original.as_bytes());

    let config_after_first = fs::read(&config).unwrap();
    let backups_before_second = backups_for(&config);
    let second = run_ok_with_config_env(&root, &configure_args(&config, &[]));
    assert!(second
        .stdout
        .contains("no exact legacy ContextPatch wildcard `toolPolicy` was present"));
    assert!(!second.stdout.contains("restart Claude Desktop"));
    assert_eq!(fs::read(&config).unwrap(), config_after_first);
    assert_eq!(backups_for(&config), backups_before_second);
    assert_eq!(snapshot_tree(&library), library_before);
}

#[test]
fn configure_claude_desktop_dry_run_writes_no_policy_or_library_files() {
    let root = temp_root("configure_claude_desktop_dry_run");
    let config = root.join("claude_desktop_config.json");
    let library = deprecated_config_library_path(&root);
    let original = br#"{"mcpServers":{"contextpatch":{"command":"/opt/contextpatch-server","args":["--repo-root","/repo"],"toolPolicy":{"*":"allow"}}}}"#;
    fs::write(&config, original).unwrap();

    let output = run_ok_with_config_env(&root, &configure_args(&config, &["--dry-run"]));
    assert!(output
        .stdout
        .contains("would remove the exact legacy ContextPatch wildcard `toolPolicy`"));
    assert!(!output.stdout.contains("restart Claude Desktop"));
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(!library.exists());
    assert!(backups_for(&config).is_empty());
}

#[test]
fn configure_claude_desktop_help_and_parser_have_no_config_library_surface() {
    let root = temp_root("configure_help");
    let help = run_ok(&root, &["--help"]);
    assert!(help.stdout.contains(
        "configure-claude-desktop [--config <path>] [--dry-run] [--tool-surface <project|full>]"
    ));
    assert!(!help.stdout.contains("--config-library"));

    let config = root.join("claude_desktop_config.json");
    fs::write(
        &config,
        br#"{"mcpServers":{"contextpatch":{"command":"contextpatch-server"}}}"#,
    )
    .unwrap();
    let refused = run_err(
        &root,
        &[
            "configure-claude-desktop",
            "--config",
            config.to_str().unwrap(),
            "--config-library",
            root.to_str().unwrap(),
        ],
    );
    assert!(refused
        .stderr
        .contains("unknown argument `--config-library`"));
}

#[test]
fn configure_claude_desktop_can_restore_full_surface_without_reordering_other_args() {
    let root = temp_root("configure_claude_desktop_full_surface");
    let config = root.join("claude_desktop_config.json");
    fs::write(
        &config,
        br#"{"mcpServers":{"contextpatch":{"command":"contextpatch-server","args":["--repo-root","/repo","--custom","value","--tool-surface","project"],"env":{"KEEP":"yes"}}}}"#,
    )
    .unwrap();

    let output = run_ok(&root, &configure_args(&config, &["--tool-surface", "full"]));
    assert!(output.stdout.contains("set the `full` tool surface"));
    let updated = read_json(&config);
    assert_eq!(
        updated["mcpServers"]["contextpatch"]["args"],
        serde_json::json!([
            "--repo-root",
            "/repo",
            "--custom",
            "value",
            "--tool-surface",
            "full"
        ])
    );
    assert_eq!(
        updated["mcpServers"]["contextpatch"]["env"],
        serde_json::json!({"KEEP": "yes"})
    );
}

#[test]
fn configure_claude_desktop_refuses_malformed_surface_without_writing() {
    let root = temp_root("configure_claude_desktop_malformed_surface");
    let config = root.join("claude_desktop_config.json");
    let original = br#"{"mcpServers":{"contextpatch":{"command":"contextpatch-server","args":["--repo-root","/repo","--tool-surface","wide"]}}}"#;
    fs::write(&config, original).unwrap();

    let output = run_err(&root, &configure_args(&config, &[]));
    assert!(output.stderr.contains("invalid tool surface `wide`"));
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(backups_for(&config).is_empty());
}

#[test]
fn configure_claude_desktop_refuses_duplicate_surface_without_writing() {
    let root = temp_root("configure_claude_desktop_duplicate_surface");
    let config = root.join("claude_desktop_config.json");
    let original = br#"{"mcpServers":{"contextpatch":{"command":"contextpatch-server","args":["--repo-root","/repo","--tool-surface","project","--tool-surface","full"]}}}"#;
    fs::write(&config, original).unwrap();

    let output = run_err(&root, &configure_args(&config, &[]));
    assert!(output.stderr.contains("duplicate `--tool-surface`"));
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(backups_for(&config).is_empty());
}

#[test]
fn configure_claude_desktop_refuses_invalid_config_without_changing_it() {
    let root = temp_root("configure_claude_desktop_refuses_invalid_config");
    let config = root.join("claude_desktop_config.json");
    fs::write(&config, b"{not json}\n").unwrap();
    let before = snapshot_tree(&root);

    let output = run_err(&root, &configure_args(&config, &[]));
    assert!(output.stderr.contains("not valid JSON"));
    assert_eq!(fs::read(&config).unwrap(), b"{not json}\n");
    assert!(backups_for(&config).is_empty());
    assert_eq!(
        snapshot_tree(&root)
            .keys()
            .filter(|path| !path.ends_with(".contextpatch.lock"))
            .count(),
        before.len()
    );
}

struct OutputText {
    stdout: String,
    stderr: String,
}

fn configure_args<'a>(config: &'a Path, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "configure-claude-desktop",
        "--config",
        config.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    args
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn backups_for(path: &Path) -> Vec<PathBuf> {
    let prefix = format!(
        "{}.contextpatch-backup-",
        path.file_name().unwrap().to_string_lossy()
    );
    let mut backups = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    backups.sort();
    backups
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, directory: &Path, snapshot: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, snapshot);
            } else {
                snapshot.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    collect(root, root, &mut snapshot);
    snapshot
}

fn deprecated_config_library_path(root: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        root.join("Library/Application Support/Claude-3p/configLibrary")
    } else {
        root.join("Claude-3p/configLibrary")
    }
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

fn run_ok_with_config_env(root: &Path, args: &[&str]) -> OutputText {
    let output = Command::new(contextpatch())
        .current_dir(root)
        .env("HOME", root)
        .env("APPDATA", root)
        .env("XDG_CONFIG_HOME", root)
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
