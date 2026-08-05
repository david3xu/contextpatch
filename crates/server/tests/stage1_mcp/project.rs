use std::fs;

use serde_json::Value;

use crate::support::*;

#[test]
fn project_surface_wraps_existing_actions_without_changing_their_policy_identity() {
    let root = git_repo("project_surface_wraps_existing_actions");
    fs::write(root.join("sample.txt"), "alpha\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);
    let direct_read = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}"#,
        ],
    );

    let responses = run_server_project_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"describe"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"describe","arguments":{"name":"read_range"}}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"capability_manifest","arguments":{"names_only":true}}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"missing_action","arguments":{}}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"project_execute","arguments":{}}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"replace_exact","arguments":{"path":"sample.txt","old":"alpha","new":"beta"}}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"git_commit_exact","arguments":{"paths":["sample.txt"],"subject":"test: wrapped dry run"}}}}"#,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"read_write_receipts","arguments":{"limit":10}}}}"#,
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"run_guarded_command","arguments":{"program":"git","args":["status","--short"],"timeout_secs":30}}}}"#,
            r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"describe","arguments":{"action":"file_info"}}}}"#,
            r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"describe","arguments":{"name":"file_info","action":"read_range"}}}}"#,
        ],
    );

    assert!(responses[0]["result"]["instructions"]
        .as_str()
        .unwrap()
        .starts_with("Call project_execute first"));

    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "project_execute");
    let description = tools[0]["description"].as_str().unwrap();
    assert!(
        description.starts_with(
            "Describe or execute one guarded ContextPatch action for this configured project."
        ),
        "{description}"
    );
    // The wrapper schema is all a project-surface client sees before its first call, so the cheap
    // projections have to be advertised there. Exact wording is pinned by the schema unit test.
    for advertised in ["arguments.name", "names_only", "response_mode"] {
        assert!(
            description.contains(advertised),
            "the wrapper description must advertise `{advertised}`: {description}"
        );
    }
    assert_eq!(
        tools[0]["inputSchema"]["properties"]["repository"]["type"],
        "string"
    );
    assert_text(
        &responses[2],
        "\"scope\": \"optional normalized workspace-relative path",
    );
    assert_eq!(
        tools[0]["annotations"],
        serde_json::json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false
        })
    );

    let discovery: Value = serde_json::from_str(response_text(&responses[2])).unwrap();
    assert_eq!(discovery["tool_surface"], "project");
    // One more than the registered tool count, because the meta action is dispatchable too and a client
    // that enumerates actions must be able to find it.
    assert_eq!(discovery["action_count"], 53);
    assert_eq!(
        discovery["action_definitions"].as_array().unwrap().len(),
        52
    );
    let discovered: Vec<&str> = discovery["action_names"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(discovered.contains(&"replace_exact"));
    assert!(
        discovered.contains(&"describe"),
        "reported action names must include the meta action: {discovered:?}"
    );

    let read_definition: Value = serde_json::from_str(response_text(&responses[3])).unwrap();
    assert_eq!(read_definition["definition"]["name"], "read_range");
    assert_eq!(
        read_definition["definition"]["annotations"]["readOnlyHint"],
        true
    );
    assert_eq!(response_text(&responses[4]), response_text(&direct_read[0]));

    let capabilities: Value = serde_json::from_str(response_text(&responses[5])).unwrap();
    assert_eq!(capabilities["tool_surface"], "project");
    assert_eq!(
        capabilities["tool_names"],
        serde_json::json!(["project_execute"])
    );
    assert_eq!(capabilities["action_names"].as_array().unwrap().len(), 53);
    assert!(
        capabilities["action_names"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "describe"),
        "the cheap names_only projection must also advertise the meta action: {capabilities}"
    );
    assert!(
        capabilities.get("action_definitions").is_none(),
        "names_only must omit full action schemas"
    );

    for (response, expected) in [
        (&responses[6], "unknown tool for project surface"),
        (&responses[7], "unknown action `missing_action`"),
        (&responses[8], "recursive wrapper dispatch"),
    ] {
        assert_eq!(response["result"]["isError"], true);
        assert_text(response, expected);
    }

    assert_text(&responses[9], "replaced bytes");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta\n"
    );
    assert_text(&responses[10], "\"dry_run\": true");
    let receipts: Value = serde_json::from_str(response_text(&responses[11])).unwrap();
    assert!(receipts["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|receipt| receipt["tool"] == "replace_exact"));
    assert_text(&responses[12], "allowlist: git/status");
    let file_info_definition: Value = serde_json::from_str(response_text(&responses[13])).unwrap();
    assert_eq!(file_info_definition["definition"]["name"], "file_info");
    assert_eq!(
        file_info_definition["definition"]["inputSchema"]["properties"]["paths"]["maxItems"],
        64
    );
    assert_eq!(responses[14]["result"]["isError"], true);
    assert_text(&responses[14], "either `name` or `action`, not both");
}

#[cfg(unix)]
#[test]
fn project_dispatch_queries_the_anchored_repository_not_a_replacement() {
    // The vertical proof for descriptor-anchored dispatch. `git_remote_list` is migrated to the typed
    // repository target, so a selected repository is queried through the descriptor that validated it
    // rather than by re-resolving the selector. Two repositories with distinguishable remotes make it
    // visible which one actually answered.
    let workspace = temp_root("project_surface_anchored_remote");

    let target = workspace.join("target");
    fs::create_dir_all(&target).unwrap();
    init_git_repo(&target);
    git(
        &target,
        &["remote", "add", "origin", "https://example.invalid/target.git"],
    );

    let decoy = workspace.join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    init_git_repo(&decoy);
    git(
        &decoy,
        &["remote", "add", "origin", "https://example.invalid/decoy.git"],
    );

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    // The selector answers from the repository it names, never from the workspace root or the sibling.
    let selected = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_remote_list","arguments":{}}}}"#,
    );
    assert_text(&selected, "target.git");
    assert!(
        !response_text(&selected).contains("decoy.git"),
        "{}",
        response_text(&selected)
    );

    // And the reverse direction, so the first assertion cannot pass by accident of ordering.
    let sibling = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"decoy","action":"git_remote_list","arguments":{}}}}"#,
    );
    assert_text(&sibling, "decoy.git");
    assert!(
        !response_text(&sibling).contains("target.git"),
        "{}",
        response_text(&sibling)
    );

    // With the selected directory renamed away, dispatch refuses rather than falling back to the
    // workspace root or answering from whatever else happens to be present.
    fs::rename(&target, workspace.join("moved-aside")).unwrap();
    let refused = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_remote_list","arguments":{}}}}"#,
    );
    assert_eq!(refused["result"]["isError"], true);
    assert_text(&refused, "project_execute refused");
    assert!(
        !response_text(&refused).contains("decoy.git"),
        "{}",
        response_text(&refused)
    );

    server.finish();
}

#[test]
fn project_surface_selects_exact_child_repositories_within_a_workspace() {
    let workspace = temp_root("project_surface_workspace");
    fs::write(workspace.join("workspace.txt"), "workspace\n").unwrap();

    let alpha = workspace.join("alpha");
    fs::create_dir_all(alpha.join("references")).unwrap();
    init_git_repo(&alpha);
    fs::write(alpha.join("sample.txt"), "alpha\n").unwrap();
    fs::write(
        alpha.join("references/check-base-image.sh"),
        "#!/bin/sh\nexit 0\n",
    )
    .unwrap();
    git(&alpha, &["add", "."]);
    git(&alpha, &["commit", "--quiet", "-m", "initial alpha"]);

    let beta = workspace.join("beta");
    fs::create_dir_all(&beta).unwrap();
    init_git_repo(&beta);
    fs::write(beta.join("sample.txt"), "beta\n").unwrap();
    git(&beta, &["add", "."]);
    git(&beta, &["commit", "--quiet", "-m", "initial beta"]);

    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"read_range","arguments":{"path":"workspace.txt","start_line":1,"end_line":1}}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"beta","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"status_guard","arguments":{}}}}"#,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"validation_profile_run","arguments":{"profile":"repo-basic","timeout_secs":30}}}}"#,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"base_image_check_run","arguments":{"dry_run":true,"timeout_secs":30}}}}"#,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"github_pr_run","arguments":{"action":"pr_create","base":"main","head":"feature/task","title":"Child repository test","body":"Dry run.","dry_run":true}}}}"#,
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"replace_exact","arguments":{"path":"sample.txt","old":"alpha","new":"updated"}}}}"#,
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"read_write_receipts","arguments":{"limit":10}}}}"#,
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"beta","action":"read_write_receipts","arguments":{"limit":10}}}}"#,
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"alpha","action":"git_commit_exact","arguments":{"paths":["sample.txt"],"subject":"test: selected child repository"}}}}"#,
    ];
    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);
    let mut responses = Vec::with_capacity(requests.len());
    for request in requests {
        let response = server.exchange(request);
        if response["id"] == 5 {
            let log_id = started_log_id(&response);
            let completed = poll_project_command_log(&mut server, "alpha", &log_id, 50);
            assert_text(&completed, "status: completed");
            assert_text(&completed, "profile: repo-basic");
        }
        responses.push(response);
    }
    server.finish();

    assert_text(&responses[0], "workspace");
    assert_text(&responses[1], "alpha");
    assert_text(&responses[2], "beta");
    assert_text(&responses[3], "clean: no Git changes");
    assert_text(&responses[4], "\"profile\": \"repo-basic\"");
    assert_text(&responses[4], "\"status\": \"running\"");
    assert_text(&responses[4], "\"log_id\"");
    assert_text(&responses[5], "\"tool\": \"base_image_check_run\"");
    assert_text(&responses[5], "\"dry_run\": true");
    assert_text(&responses[6], "\"tool\": \"github_pr_run\"");
    assert_text(
        &responses[6],
        &alpha.canonicalize().unwrap().display().to_string(),
    );
    assert_text(&responses[7], "replaced bytes");
    assert_eq!(
        fs::read_to_string(alpha.join("sample.txt")).unwrap(),
        "updated\n"
    );
    assert_eq!(
        fs::read_to_string(beta.join("sample.txt")).unwrap(),
        "beta\n"
    );

    let alpha_receipts: Value = serde_json::from_str(response_text(&responses[8])).unwrap();
    assert!(alpha_receipts["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|receipt| receipt["tool"] == "replace_exact"));
    let beta_receipts: Value = serde_json::from_str(response_text(&responses[9])).unwrap();
    assert!(beta_receipts["receipts"].as_array().unwrap().is_empty());
    assert_text(&responses[10], "\"dry_run\": true");
    assert_text(&responses[10], "sample.txt");
}

/// Whether a ref resolves in a repository, without asserting success.
///
/// Test-local rather than reusing the asserting `git` helper, because absence is the expected answer half
/// the time here.
fn has_ref(root: &std::path::Path, reference: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "--quiet", reference])
        .output()
        .unwrap()
        .status
        .success()
}

/// Whether an object actually exists in a repository.
///
/// `rev-parse --verify` is not enough: given a full hash it reports success on format alone, without
/// requiring the object to be present.
fn has_object(root: &std::path::Path, object: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(root)
        .args(["cat-file", "-e", object])
        .output()
        .unwrap()
        .status
        .success()
}

/// Resolve a ref to its commit, for comparing repositories against each other.
fn commit_at(root: &std::path::Path, reference: &str) -> String {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(["rev-parse", reference])
        .output()
        .unwrap();
    assert!(output.status.success(), "rev-parse {reference} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// An empty bare repository standing in for a remote.
///
/// Bare on purpose: pushing to a non-bare repository's checked-out branch is refused by Git itself, which
/// would mask the guard under test.
fn bare_remote(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bare = parent.join(name);
    fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--quiet", "--bare", "--initial-branch=main"]);
    bare
}

/// A repository with its own distinct remote and its own distinct commit.
fn repo_with_remote(
    workspace: &std::path::Path,
    remotes: &std::path::Path,
    name: &str,
    marker: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let bare = bare_remote(remotes, &format!("{name}.git"));
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    fs::write(repo.join("marker.txt"), marker).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", marker.trim()]);
    git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&repo, &["push", "--quiet", "origin", "main"]);
    (repo, bare)
}

#[cfg(unix)]
#[test]
fn project_dispatch_fetches_only_the_selected_repository_refs() {
    // Deterministic rather than timing based: no swap is needed to show which repository a fetch reached,
    // because neither repository has a remote-tracking ref until one is fetched. After the call exactly
    // one of them does.
    let workspace = temp_root("project_surface_fetch_isolation");
    let remotes = temp_root("project_surface_fetch_isolation_remotes");
    let (target, _target_remote) = repo_with_remote(&workspace, &remotes, "target", "target\n");
    let (decoy, _decoy_remote) = repo_with_remote(&workspace, &remotes, "decoy", "decoy\n");

    let tracking = "refs/remotes/origin/main";
    // The initial push already created tracking refs, so both are cleared to make the fetch's effect
    // observable rather than pre-satisfied.
    git(&target, &["update-ref", "-d", tracking]);
    git(&decoy, &["update-ref", "-d", tracking]);
    assert!(!has_ref(&target, tracking), "no tracking ref before fetching");
    assert!(!has_ref(&decoy, tracking), "no tracking ref before fetching");
    let decoy_head_before = commit_at(&decoy, "HEAD");

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);
    let checked = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_remote_check","arguments":{"branch":"main"}}}}"#,
    );

    assert_ne!(
        checked["result"]["isError"],
        true,
        "{}",
        response_text(&checked)
    );
    let report: Value = serde_json::from_str(response_text(&checked)).unwrap();
    assert_eq!(report["head"], commit_at(&target, "HEAD"));

    // Only the selected repository advanced.
    assert!(has_ref(&target, tracking), "the selected repository fetched");
    assert!(
        !has_ref(&decoy, tracking),
        "the sibling repository must not have been fetched into"
    );
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);

    // Replacing the logical path does not let a later call reach through a stale selection: the swapped
    // directory answers as itself, and the repository moved aside is left alone.
    let moved_aside = workspace.join("moved-aside");
    fs::rename(&target, &moved_aside).unwrap();
    fs::rename(&decoy, &target).unwrap();
    let after_swap = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_remote_check","arguments":{"branch":"main"}}}}"#,
    );
    let swapped: Value = serde_json::from_str(response_text(&after_swap)).unwrap();
    assert_eq!(swapped["head"], decoy_head_before, "the swapped directory answers as itself");
    assert_eq!(commit_at(&moved_aside, "HEAD"), report["head"].as_str().unwrap());

    server.finish();
}

#[cfg(unix)]
#[test]
fn project_dispatch_pushes_only_to_the_selected_repository_remote() {
    // Two repositories, two bare remotes, two distinct commits. A push through the selected repository
    // must reach that repository's remote and nothing else.
    let workspace = temp_root("project_surface_push_isolation");
    let remotes = temp_root("project_surface_push_isolation_remotes");
    let (target, target_remote) = repo_with_remote(&workspace, &remotes, "target", "target\n");
    let (decoy, decoy_remote) = repo_with_remote(&workspace, &remotes, "decoy", "decoy\n");

    // A commit the target's remote has not seen yet.
    fs::write(target.join("marker.txt"), "target advanced\n").unwrap();
    git(&target, &["add", "."]);
    git(&target, &["commit", "--quiet", "-m", "advance target"]);
    let pushed_commit = commit_at(&target, "HEAD");
    let target_remote_before = commit_at(&target_remote, "refs/heads/main");
    let decoy_remote_before = commit_at(&decoy_remote, "refs/heads/main");
    let decoy_head_before = commit_at(&decoy, "HEAD");
    assert_ne!(pushed_commit, target_remote_before);

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "project_execute",
            "arguments": {
                "repository": "target",
                "action": "git_push_exact",
                "arguments": {
                    "remote": "origin",
                    "branch": "main",
                    "expected_head": pushed_commit,
                    "confirm": "push exact commit"
                }
            }
        }
    })
    .to_string();

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);
    let pushed = server.exchange(&request);
    server.finish();

    assert_ne!(
        pushed["result"]["isError"],
        true,
        "{}",
        response_text(&pushed)
    );
    let report: Value = serde_json::from_str(response_text(&pushed)).unwrap();
    assert_eq!(report["pushed"], true);
    assert_eq!(report["commit"], pushed_commit);

    // The selected repository's remote received exactly that commit.
    assert_eq!(
        commit_at(&target_remote, "refs/heads/main"),
        pushed_commit,
        "the selected repository's remote received the push"
    );

    // The sibling remote and the sibling repository are untouched.
    assert_eq!(
        commit_at(&decoy_remote, "refs/heads/main"),
        decoy_remote_before,
        "the sibling remote must not have received the push"
    );
    assert!(
        !has_object(&decoy_remote, &pushed_commit),
        "the pushed commit must not exist in the sibling remote at all"
    );
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);
}

/// A repository whose tracked file is committed and then modified, so it has exactly one dirty path.
fn dirty_repo(workspace: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    fs::write(repo.join("marker.txt"), format!("{name} committed\n")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);
    fs::write(repo.join("marker.txt"), format!("{name} dirty\n")).unwrap();
    repo
}

#[cfg(unix)]
#[test]
fn project_dispatch_stages_commits_and_restores_only_the_selected_repository() {
    // The mutating counterpart to the query isolation tests. Staging, committing, and restoring through a
    // selected repository must land in that repository, and the sibling must be bit-for-bit untouched
    // across all three.
    let workspace = temp_root("project_surface_mutation_isolation");
    let target = dirty_repo(&workspace, "target");
    let decoy = dirty_repo(&workspace, "decoy");

    let target_head_before = commit_at(&target, "HEAD");
    let decoy_head_before = commit_at(&decoy, "HEAD");
    let decoy_dirty_before = fs::read_to_string(decoy.join("marker.txt")).unwrap();

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    let staged = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_stage_exact","arguments":{"paths":["marker.txt"],"dry_run":false,"confirm":"stage exact paths"}}}}"#,
    );
    assert_ne!(
        staged["result"]["isError"],
        true,
        "{}",
        response_text(&staged)
    );

    let committed = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_commit_exact","arguments":{"paths":["marker.txt"],"subject":"commit in the selected repository","dry_run":false,"confirm":"commit exact paths"}}}}"#,
    );
    assert_ne!(
        committed["result"]["isError"],
        true,
        "{}",
        response_text(&committed)
    );

    // The selected repository advanced; the sibling did not, and is still dirty with its own content.
    let target_head_after_commit = commit_at(&target, "HEAD");
    assert_ne!(target_head_after_commit, target_head_before);
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);
    assert_eq!(
        fs::read_to_string(decoy.join("marker.txt")).unwrap(),
        decoy_dirty_before
    );

    // Restoring is the same question in the other direction: it must discard work in the selected
    // repository only.
    fs::write(target.join("marker.txt"), "target changed again\n").unwrap();
    let restored = server.exchange(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_restore_exact","arguments":{"paths":["marker.txt"],"dry_run":false,"confirm":"restore exact paths"}}}}"#,
    );
    server.finish();

    assert_ne!(
        restored["result"]["isError"],
        true,
        "{}",
        response_text(&restored)
    );
    // Back to what was committed a moment ago, not to the original content.
    assert_eq!(
        fs::read_to_string(target.join("marker.txt")).unwrap(),
        "target dirty\n"
    );
    assert_eq!(commit_at(&target, "HEAD"), target_head_after_commit);

    // The sibling never moved and its uncommitted work was never discarded.
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);
    assert_eq!(
        fs::read_to_string(decoy.join("marker.txt")).unwrap(),
        decoy_dirty_before
    );
}

#[test]
fn project_surface_refuses_unsafe_or_inexact_repository_selectors() {
    use std::os::unix::fs::symlink;

    let workspace = temp_root("project_surface_invalid_repository");
    let repo = workspace.join("repo");
    fs::create_dir_all(repo.join("subdir")).unwrap();
    init_git_repo(&repo);
    fs::write(repo.join("sample.txt"), "inside\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);

    fs::create_dir(workspace.join("plain")).unwrap();
    let outside = git_repo("project_surface_outside_repository");
    fs::write(outside.join("sample.txt"), "outside\n").unwrap();
    symlink(&outside, workspace.join("linked")).unwrap();

    let responses = run_server_project(
        &workspace,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"../outside","action":"replace_exact","arguments":{"path":"sample.txt","old":"outside","new":"changed"}}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"/absolute","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"repo\\subdir","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"repo/.git","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"repo/subdir","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"plain","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"linked","action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":7,"action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1}}}}"#,
        ],
    );

    for response in &responses {
        assert_eq!(response["result"]["isError"], true);
    }
    for response in responses.iter().take(3) {
        assert_text(
            response,
            "repository must be a normalized workspace-relative path",
        );
    }
    assert_text(&responses[3], "Git administrative directory");
    assert_text(&responses[4], "not the Git worktree root");
    assert_text(&responses[5], "resolve Git root failed");
    assert_text(&responses[6], "symlink component");
    assert_text(&responses[7], "repository must be a string");
    assert_eq!(
        fs::read_to_string(outside.join("sample.txt")).unwrap(),
        "outside\n"
    );
}

/// A repository holding one committed tracked file, plus a nested directory to move into.
fn tracked_repo(workspace: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    fs::write(repo.join("tracked.txt"), format!("{name} content\n")).unwrap();
    fs::create_dir_all(repo.join("nested")).unwrap();
    fs::write(repo.join("nested").join("keep.txt"), "keep\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);
    repo
}

#[cfg(unix)]
#[test]
fn project_dispatch_moves_and_deletes_tracked_files_only_in_the_selected_repository() {
    // `move_tracked` and `delete_guarded` now carry typed root authority end to end: the Git subprocess and
    // every filesystem check around it derive from one root. Two repositories with identical layouts and
    // distinguishable content make it visible which one was actually mutated, and an outside repository
    // proves nothing escaped the workspace.
    let workspace = temp_root("project_surface_tracked_mutation_isolation");
    let target = tracked_repo(&workspace, "target");
    let decoy = tracked_repo(&workspace, "decoy");
    let outside = tracked_repo(&temp_root("project_surface_tracked_outside"), "outside");

    let decoy_head_before = commit_at(&decoy, "HEAD");
    let outside_head_before = commit_at(&outside, "HEAD");

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    let moved = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"move_tracked","arguments":{"from":"tracked.txt","to":"nested/moved.txt","dry_run":false,"confirm":"move tracked file"}}}}"#,
    );
    assert_ne!(
        moved["result"]["isError"],
        true,
        "{}",
        response_text(&moved)
    );

    // The move landed in the selected repository only.
    assert!(!target.join("tracked.txt").exists());
    assert_eq!(
        fs::read_to_string(target.join("nested").join("moved.txt")).unwrap(),
        "target content\n"
    );
    assert!(decoy.join("tracked.txt").exists());
    assert!(!decoy.join("nested").join("moved.txt").exists());
    assert!(outside.join("tracked.txt").exists());
    assert!(!outside.join("nested").join("moved.txt").exists());

    // Deleting a tracked file is the same question with a hash gate in front of it. The digest is read
    // through the same surface rather than recomputed here, so the guard is exercised as callers meet it.
    let inspected = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"decoy","action":"file_info","arguments":{"path":"tracked.txt"}}}}"#,
    );
    let info: Value = serde_json::from_str(response_text(&inspected)).unwrap();
    let decoy_sha256 = info["sha256"].as_str().unwrap().to_string();
    assert_eq!(decoy_sha256, sha256_hex_for_test(b"decoy content\n"));

    let deleted = server.exchange(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"project_execute","arguments":{{"repository":"decoy","action":"delete_guarded","arguments":{{"path":"tracked.txt","expected_sha256":"{decoy_sha256}","dry_run":false,"confirm":"delete tracked file"}}}}}}}}"#,
    ));
    server.finish();

    assert_ne!(
        deleted["result"]["isError"],
        true,
        "{}",
        response_text(&deleted)
    );

    // The deletion reached the repository that was selected for it, in the other direction from the move,
    // so neither assertion can pass by accident of ordering.
    assert!(!decoy.join("tracked.txt").exists());
    assert!(decoy.join("nested").join("keep.txt").exists());
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);

    // The outside repository was never a candidate and is bit-for-bit unchanged.
    assert_eq!(
        fs::read_to_string(outside.join("tracked.txt")).unwrap(),
        "outside content\n"
    );
    assert_eq!(commit_at(&outside, "HEAD"), outside_head_before);
}

#[cfg(unix)]
#[test]
fn project_dispatch_cleans_untracked_and_generated_paths_only_in_the_selected_repository() {
    // `delete_untracked_exact` and `delete_generated_prefix` plan, mutate, and verify through one root
    // authority. Both repositories carry identically named untracked files and identically named ignored
    // build trees, so only the authority can distinguish which one is cleaned.
    let workspace = temp_root("project_surface_cleanup_isolation");

    let prepare = |name: &str| -> std::path::PathBuf {
        let repo = workspace.join(name);
        fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        fs::write(repo.join(".gitignore"), "build/\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        fs::write(repo.join("scratch.txt"), format!("{name} scratch\n")).unwrap();
        fs::create_dir_all(repo.join("build").join("deep")).unwrap();
        fs::write(repo.join("build").join("out.bin"), "binary").unwrap();
        fs::write(repo.join("build").join("deep").join("more.bin"), "more").unwrap();
        repo
    };
    let target = prepare("target");
    let decoy = prepare("decoy");

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    let cleaned = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"delete_untracked_exact","arguments":{"paths":["scratch.txt"],"dry_run":false,"confirm":"delete untracked files"}}}}"#,
    );
    assert_ne!(
        cleaned["result"]["isError"],
        true,
        "{}",
        response_text(&cleaned)
    );
    assert!(!target.join("scratch.txt").exists());
    assert_eq!(
        fs::read_to_string(decoy.join("scratch.txt")).unwrap(),
        "decoy scratch\n",
        "the sibling's identically named untracked file must survive"
    );

    // A generated-prefix cleanup descends an ignored tree and removes it whole. Run against the sibling so
    // the two cleanups point in opposite directions.
    let pruned = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"decoy","action":"delete_generated_prefix","arguments":{"prefixes":["build"],"dry_run":false,"confirm":"delete generated paths"}}}}"#,
    );
    server.finish();

    assert_ne!(
        pruned["result"]["isError"],
        true,
        "{}",
        response_text(&pruned)
    );
    assert!(!decoy.join("build").exists(), "the named ignored tree goes whole");
    assert!(
        target.join("build").join("deep").join("more.bin").exists(),
        "the sibling's identically named ignored tree must survive"
    );
    // Tracked history in both repositories is untouched by either cleanup.
    assert!(target.join(".gitignore").exists());
    assert!(decoy.join(".gitignore").exists());
}

#[cfg(unix)]
#[test]
fn project_dispatch_verifies_branch_required_files_in_the_selected_repository() {
    // `git_branch_prepare` verifies required files against the ref it will land on and again in the
    // worktree afterwards, both through the selected repository's own authority. The sibling holds the
    // required file and the selected repository does not, so a check that reached the wrong repository
    // would pass and this refusal would disappear.
    let workspace = temp_root("project_surface_required_file_isolation");
    let remotes = temp_root("project_surface_required_file_remotes");
    let (target, _target_remote) = repo_with_remote(&workspace, &remotes, "target", "target\n");
    let (decoy, _decoy_remote) = repo_with_remote(&workspace, &remotes, "decoy", "decoy\n");

    // Only the sibling has it, and it is committed so it exists in the ref rather than only on disk.
    fs::write(decoy.join("required.txt"), "present\n").unwrap();
    git(&decoy, &["add", "."]);
    git(&decoy, &["commit", "--quiet", "-m", "add required file"]);
    git(&decoy, &["push", "--quiet", "origin", "main"]);

    let target_head_before = commit_at(&target, "HEAD");
    let decoy_head_before = commit_at(&decoy, "HEAD");

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);
    let refused = server.exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"target","action":"git_branch_prepare","arguments":{"branch":"feature","base_branch":"main","required_files":["required.txt"],"dry_run":false}}}}"#,
    );

    assert_eq!(
        refused["result"]["isError"],
        true,
        "{}",
        response_text(&refused)
    );
    assert_text(&refused, "required file `required.txt` is missing");

    // The refusal happened before the switch, so no branch was created and neither repository moved.
    assert!(!has_ref(&target, "refs/heads/feature"));
    assert!(!has_ref(&decoy, "refs/heads/feature"));
    assert_eq!(commit_at(&target, "HEAD"), target_head_before);
    assert_eq!(commit_at(&decoy, "HEAD"), decoy_head_before);

    // The same request against the repository that actually holds the file succeeds, which proves the
    // refusal above was about the selected repository rather than about the file name.
    let prepared = server.exchange(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_execute","arguments":{"repository":"decoy","action":"git_branch_prepare","arguments":{"branch":"feature","base_branch":"main","required_files":["required.txt"],"dry_run":false}}}}"#,
    );
    server.finish();

    assert_ne!(
        prepared["result"]["isError"],
        true,
        "{}",
        response_text(&prepared)
    );
    assert!(has_ref(&decoy, "refs/heads/feature"));
    // And the branch was created in that repository only.
    assert!(!has_ref(&target, "refs/heads/feature"));
    assert_eq!(commit_at(&target, "HEAD"), target_head_before);
}

// ---------------------------------------------------------------------------
// Repository-file surface isolation.
//
// Core reaches files through root authority and dispatch now hands the file handlers that authority, so
// these exercise the whole path: a selector arrives at `read_range` the same way it already arrived at
// `git_stage_exact`. Every group runs in both directions, because a test that only ever names one
// repository cannot distinguish "reached the right one" from "reached the first one".
// ---------------------------------------------------------------------------

/// A selectable repository holding one file and one nested directory.
///
/// Initialized as a Git worktree because the workspace selector requires an exact worktree root for *any*
/// selection, whatever the action does afterwards. The file surface itself is bounded by path confinement
/// rather than by that requirement, but a selector cannot reach it without satisfying the selector first.
fn file_repo(workspace: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = workspace.join(name);
    fs::create_dir_all(repo.join("nested")).unwrap();
    init_git_repo(&repo);
    fs::write(repo.join("sample.txt"), format!("{name} line one\n")).unwrap();
    fs::write(repo.join("nested").join("inner.txt"), format!("{name} inner\n")).unwrap();
    repo
}

fn file_action(repository: &str, action: &str, arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"project_execute","arguments":{{"repository":"{repository}","action":"{action}","arguments":{arguments}}}}}}}"#
    )
}

#[cfg(unix)]
#[test]
fn project_dispatch_reads_and_lists_only_the_selected_repository() {
    let workspace = temp_root("project_file_reads");
    let target = file_repo(&workspace, "target");
    let decoy = file_repo(&workspace, "decoy");
    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    // Each read family answers from the repository it names, and the sibling's identically named file is
    // never the one that answered.
    for (selected, other) in [("target", "decoy"), ("decoy", "target")] {
        let read = server.exchange(&file_action(
            selected,
            "read_range",
            r#"{"path":"sample.txt","start_line":1,"end_line":1}"#,
        ));
        let text = response_text(&read).to_string();
        assert!(text.contains(&format!("{selected} line one")), "{text}");
        assert!(!text.contains(&format!("{other} line one")), "{text}");

        let info = server.exchange(&file_action(
            selected,
            "file_info",
            r#"{"path":"sample.txt"}"#,
        ));
        let parsed: Value = serde_json::from_str(response_text(&info)).unwrap();
        assert_eq!(
            parsed["sha256"],
            sha256_hex_for_test(format!("{selected} line one\n").as_bytes()),
            "file_info answered from the wrong repository"
        );

        let bytes = server.exchange(&file_action(
            selected,
            "read_file_bytes",
            r#"{"path":"nested/inner.txt","encoding":"base64"}"#,
        ));
        let parsed: Value = serde_json::from_str(response_text(&bytes)).unwrap();
        assert_eq!(
            parsed["sha256"],
            sha256_hex_for_test(format!("{selected} inner\n").as_bytes())
        );

        let listing = server.exchange(&file_action(
            selected,
            "list_directory",
            r#"{"path":".","recursive":true}"#,
        ));
        let text = response_text(&listing);
        assert!(text.contains("sample.txt"), "{text}");
        assert!(text.contains("inner.txt"), "{text}");

        let diff = server.exchange(&file_action(
            selected,
            "diff_preview",
            &format!(
                r#"{{"path":"sample.txt","old":"{selected} line one","new":"changed"}}"#
            ),
        ));
        assert_ne!(
            diff["result"]["isError"],
            true,
            "diff_preview must read the selected repository: {}",
            response_text(&diff)
        );
    }

    server.finish();

    // Reads change nothing.
    assert_eq!(
        fs::read_to_string(target.join("sample.txt")).unwrap(),
        "target line one\n"
    );
    assert_eq!(
        fs::read_to_string(decoy.join("sample.txt")).unwrap(),
        "decoy line one\n"
    );
}

#[cfg(unix)]
#[test]
fn project_dispatch_writes_replacements_and_modes_only_in_the_selected_repository() {
    let workspace = temp_root("project_file_writes");
    let outside = file_repo(&temp_root("project_file_writes_outside"), "outside");
    let target = file_repo(&workspace, "target");
    let decoy = file_repo(&workspace, "decoy");
    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    for (selected, other_name) in [("target", "decoy"), ("decoy", "target")] {
        let other = workspace.join(other_name);

        let created = server.exchange(&file_action(
            selected,
            "write_new_file",
            &format!(r#"{{"path":"created-{selected}.txt","content":"fresh\n"}}"#),
        ));
        assert_ne!(
            created["result"]["isError"],
            true,
            "{}",
            response_text(&created)
        );
        assert!(workspace
            .join(selected)
            .join(format!("created-{selected}.txt"))
            .exists());
        assert!(!other.join(format!("created-{selected}.txt")).exists());

        let replaced = server.exchange(&file_action(
            selected,
            "replace_exact",
            &format!(
                r#"{{"path":"sample.txt","old":"{selected} line one","new":"{selected} replaced"}}"#
            ),
        ));
        assert_ne!(
            replaced["result"]["isError"],
            true,
            "{}",
            response_text(&replaced)
        );
        assert_eq!(
            fs::read_to_string(workspace.join(selected).join("sample.txt")).unwrap(),
            format!("{selected} replaced\n")
        );
        // The sibling's identically named file was not touched by this replacement. Compared against the
        // marker rather than against original content, because the loop's other iteration edits it too.
        let sibling = fs::read_to_string(other.join("sample.txt")).unwrap();
        assert!(
            !sibling.contains(&format!("{selected} replaced")),
            "the sibling must not carry the selected repository's edit: {sibling}"
        );
        assert!(
            sibling.contains(other_name),
            "the sibling still holds its own content: {sibling}"
        );

        // The directory name carries the selector, so each iteration asserts about a name the other
        // iteration never created.
        let directory = server.exchange(&file_action(
            selected,
            "create_directory",
            &format!(r#"{{"path":"generated-{selected}/deep","parents":true}}"#),
        ));
        assert_ne!(
            directory["result"]["isError"],
            true,
            "{}",
            response_text(&directory)
        );
        assert!(workspace
            .join(selected)
            .join(format!("generated-{selected}"))
            .join("deep")
            .is_dir());
        assert!(!other.join(format!("generated-{selected}")).exists());
    }

    // The exact-hash overwrite is gated on a digest read through the same surface, so a stale digest from
    // the sibling must not satisfy it.
    let info = server.exchange(&file_action(
        "target",
        "file_info",
        r#"{"path":"nested/inner.txt"}"#,
    ));
    let parsed: Value = serde_json::from_str(response_text(&info)).unwrap();
    let target_digest = parsed["sha256"].as_str().unwrap().to_string();

    let wrong = server.exchange(&file_action(
        "decoy",
        "write_existing_file_exact_hash",
        &format!(
            r#"{{"path":"nested/inner.txt","content":"overwritten\n","expected_sha256":"{target_digest}","dry_run":false,"confirm":"write exact hash"}}"#
        ),
    ));
    assert_eq!(wrong["result"]["isError"], true);
    assert_text(&wrong, "hash mismatch");
    assert_eq!(
        fs::read_to_string(decoy.join("nested").join("inner.txt")).unwrap(),
        "decoy inner\n",
        "a refused overwrite must leave the file alone"
    );

    let right = server.exchange(&file_action(
        "target",
        "write_existing_file_exact_hash",
        &format!(
            r#"{{"path":"nested/inner.txt","content":"overwritten\n","expected_sha256":"{target_digest}","dry_run":false,"confirm":"write exact hash"}}"#
        ),
    ));
    assert_ne!(
        right["result"]["isError"],
        true,
        "{}",
        response_text(&right)
    );

    let mode = server.exchange(&file_action(
        "target",
        "set_file_executable",
        r#"{"path":"sample.txt","executable":true,"dry_run":true}"#,
    ));
    server.finish();
    assert_ne!(mode["result"]["isError"], true, "{}", response_text(&mode));

    assert_eq!(
        fs::read_to_string(target.join("nested").join("inner.txt")).unwrap(),
        "overwritten\n"
    );
    assert_eq!(
        fs::read_to_string(decoy.join("nested").join("inner.txt")).unwrap(),
        "decoy inner\n"
    );
    // Nothing reached the repository outside the workspace.
    assert_eq!(
        fs::read_to_string(outside.join("sample.txt")).unwrap(),
        "outside line one\n"
    );
    assert!(!outside.join("generated-target").exists());
    assert!(!outside.join("generated-decoy").exists());
}

#[cfg(unix)]
#[test]
fn project_dispatch_bulk_actions_stay_atomic_within_the_selected_repository() {
    let workspace = temp_root("project_file_bulk");
    let target = file_repo(&workspace, "target");
    let decoy = file_repo(&workspace, "decoy");
    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    // A bulk write lands entirely in the selected repository. Paths are root-level because this action
    // does not create parent directories, which is existing behavior rather than something to work around.
    let written = server.exchange(&file_action(
        "target",
        "bulk_write_new_files_base64",
        r#"{"entries":[{"path":"bulk-one.txt","content_base64":"b25l"},{"path":"bulk-two.txt","content_base64":"dHdv"}]}"#,
    ));
    assert_ne!(
        written["result"]["isError"],
        true,
        "{}",
        response_text(&written)
    );
    assert!(target.join("bulk-one.txt").exists());
    assert!(target.join("bulk-two.txt").exists());
    assert!(!decoy.join("bulk-one.txt").exists());
    assert!(!decoy.join("bulk-two.txt").exists());

    // A bulk replacement that fails validation writes nothing. The second hunk names a file that does not
    // exist, so the first must not land either.
    let refused = server.exchange(&file_action(
        "decoy",
        "bulk_replace_exact",
        r#"{"entries":[{"path":"sample.txt","old":"decoy line one","new":"changed"},{"path":"absent.txt","old":"x","new":"y"}]}"#,
    ));
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(
        fs::read_to_string(decoy.join("sample.txt")).unwrap(),
        "decoy line one\n",
        "validation refusals leave every target unchanged"
    );

    // And the successful direction, against the other repository.
    let applied = server.exchange(&file_action(
        "decoy",
        "bulk_replace_exact",
        r#"{"entries":[{"path":"sample.txt","old":"decoy line one","new":"decoy changed"},{"path":"nested/inner.txt","old":"decoy inner","new":"decoy inner changed"}]}"#,
    ));
    server.finish();

    assert_ne!(
        applied["result"]["isError"],
        true,
        "{}",
        response_text(&applied)
    );
    assert_eq!(
        fs::read_to_string(decoy.join("sample.txt")).unwrap(),
        "decoy changed\n"
    );
    // The sibling was never a target of either batch.
    assert_eq!(
        fs::read_to_string(target.join("sample.txt")).unwrap(),
        "target line one\n"
    );
}

#[cfg(unix)]
#[test]
fn project_dispatch_refuses_symlinked_paths_across_the_file_surface() {
    use std::os::unix::fs::symlink;

    let workspace = temp_root("project_file_symlinks");
    let outside = temp_root("project_file_symlinks_outside");
    fs::write(outside.join("secret.txt"), "outside secret\n").unwrap();
    let target = file_repo(&workspace, "target");
    // One link at an intermediate component and one at the leaf.
    symlink(&outside, target.join("escape")).unwrap();
    symlink("sample.txt", target.join("alias.txt")).unwrap();

    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    // An intermediate symlink is refused by every family that resolves a path.
    for (action, arguments) in [
        ("read_range", r#"{"path":"escape/secret.txt","start_line":1,"end_line":1}"#),
        ("read_file_bytes", r#"{"path":"escape/secret.txt"}"#),
        (
            "replace_exact",
            r#"{"path":"escape/secret.txt","old":"outside","new":"x"}"#,
        ),
        (
            "write_new_file",
            r#"{"path":"escape/created.txt","content":"x\n"}"#,
        ),
        (
            "create_directory",
            r#"{"path":"escape/generated","parents":true}"#,
        ),
    ] {
        let response = server.exchange(&file_action("target", action, arguments));
        assert_eq!(
            response["result"]["isError"], true,
            "{action} must refuse an intermediate symlink: {}",
            response_text(&response)
        );
    }

    // A symlink at the leaf is reported as a symlink by inspection, and refused by reading.
    let info = server.exchange(&file_action(
        "target",
        "file_info",
        r#"{"path":"alias.txt"}"#,
    ));
    let parsed: Value = serde_json::from_str(response_text(&info)).unwrap();
    assert_eq!(
        parsed["kind"], "symlink",
        "inspection describes what is there rather than following it"
    );
    assert_eq!(parsed["is_symlink"], true);

    let read = server.exchange(&file_action(
        "target",
        "read_range",
        r#"{"path":"alias.txt","start_line":1,"end_line":1}"#,
    ));
    server.finish();
    assert_eq!(
        read["result"]["isError"], true,
        "reading a symlink leaf is refused rather than followed: {}",
        response_text(&read)
    );

    // Nothing outside the workspace was read, written, or created.
    assert_eq!(
        fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "outside secret\n"
    );
    assert!(!outside.join("created.txt").exists());
    assert!(!outside.join("generated").exists());
}

#[cfg(unix)]
#[test]
fn project_dispatch_file_actions_do_not_follow_a_replaced_root() {
    use std::os::unix::fs::symlink;

    // The case the retained descriptor exists for, now proven through the file surface rather than only
    // through Git. A selection is validated, then its name is taken over by a different directory.
    let workspace = temp_root("project_file_replaced_root");
    let target = file_repo(&workspace, "target");
    let mut server = ServerExchange::spawn(&workspace, &["--tool-surface", "project"], &[]);

    // Establish that the selector works before the swap.
    let before = server.exchange(&file_action(
        "target",
        "read_range",
        r#"{"path":"sample.txt","start_line":1,"end_line":1}"#,
    ));
    assert!(response_text(&before).contains("target line one"));

    // Move the selected directory aside and leave a different one under its name.
    let holder = temp_root("project_file_replaced_root_holder");
    let moved = holder.join("moved");
    fs::rename(&target, &moved).unwrap();
    let replacement = holder.join("replacement");
    fs::create_dir_all(replacement.join("nested")).unwrap();
    fs::write(replacement.join("sample.txt"), "replacement line one\n").unwrap();
    symlink(&replacement, &target).unwrap();

    // Reads and writes refuse rather than answering from, or writing into, the replacement.
    for (action, arguments) in [
        ("read_range", r#"{"path":"sample.txt","start_line":1,"end_line":1}"#),
        ("write_new_file", r#"{"path":"leaked.txt","content":"x\n"}"#),
        ("list_directory", r#"{"path":"."}"#),
    ] {
        let response = server.exchange(&file_action("target", action, arguments));
        let text = response_text(&response).to_string();
        assert_eq!(
            response["result"]["isError"], true,
            "{action} must refuse a replaced root: {text}"
        );
        assert!(
            !text.contains("replacement line one"),
            "{action} must not answer from the replacement: {text}"
        );
    }

    server.finish();

    // The replacement was never written into, and the directory that was selected is intact.
    assert!(!replacement.join("leaked.txt").exists());
    assert_eq!(
        fs::read_to_string(replacement.join("sample.txt")).unwrap(),
        "replacement line one\n"
    );
    assert_eq!(
        fs::read_to_string(moved.join("sample.txt")).unwrap(),
        "target line one\n"
    );
}
