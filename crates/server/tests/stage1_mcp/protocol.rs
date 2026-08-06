use std::fs;

use serde_json::Value;

use crate::support::*;

/// Actions that must advertise `openWorldHint: true`, because they either contact a remote system
/// themselves or start repository-controlled code that inherits the server's network capability.
/// A blanket `false` here was a false public capability claim; see `docs/execution-threat-model.md`.
const EXPECTED_OPEN_WORLD_ACTIONS: &[&str] = &[
    "artifact_python_run",
    "base_image_check_run",
    "fixture_generator_run",
    "git_branch_prepare",
    "git_merge_readiness",
    "git_push_exact",
    "git_remote_check",
    "github_fork_prepare",
    "github_pr_run",
    "harbor_run_start",
    "native_build_run",
    "native_device_run",
    "project_execute",
    "run_guarded_command",
    "setup_profile_run",
    "validation_profile_run",
];

/// Execution paths that stay closed-world because their isolation is documented and enforced:
/// the task image and the cleanliness check both run with networking disabled.
const EXPECTED_ISOLATED_ACTIONS: &[&str] =
    &["task_image_python_run", "image_cleanliness_check_run"];

#[test]
fn stage2_open_world_annotations_match_the_documented_execution_authority() {
    let root = git_repo("stage2_open_world_annotations_match_authority");
    let responses = run_server(
        &root,
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#],
    );

    let tools = responses[0]["result"]["tools"]
        .as_array()
        .expect("tools/list array");
    assert!(!tools.is_empty(), "tools/list must advertise something");

    let mut observed_open_world = Vec::new();
    for tool in tools {
        let name = tool["name"].as_str().expect("tool name");
        let open_world = tool["annotations"]["openWorldHint"]
            .as_bool()
            .unwrap_or_else(|| panic!("{name} must advertise openWorldHint"));
        if open_world {
            observed_open_world.push(name.to_string());
        }

        if EXPECTED_ISOLATED_ACTIONS.contains(&name) {
            assert!(
                !open_world,
                "{name} runs with networking disabled and must stay closed-world"
            );
        }
    }
    observed_open_world.sort();

    let mut expected = EXPECTED_OPEN_WORLD_ACTIONS
        .iter()
        .filter(|name| {
            tools
                .iter()
                .any(|tool| tool["name"].as_str() == Some(**name))
        })
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    expected.sort();

    assert_eq!(
        observed_open_world, expected,
        "open-world classification drifted from the documented execution authority"
    );
}

#[test]
fn stage2_capability_manifest_projects_cheaply_without_losing_the_build_stamp() {
    // The full manifest runs to hundreds of lines, which made the orientation tool expensive enough to
    // avoid calling. Both cheap modes must still carry the build stamp, or a cheap read reintroduces the
    // stale-install false negative the stamp exists to prevent.
    let root = git_repo("stage2_capability_manifest_projects_cheaply");
    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capability_manifest","arguments":{"names_only":true}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"capability_manifest","arguments":{"section":"build"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capability_manifest","arguments":{"section":"not_a_section"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"capability_manifest","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/list","params":{}}"#,
        ],
    );

    let names: Value = serde_json::from_str(response_text(&responses[0])).unwrap();
    let listed = names["tool_names"].as_array().expect("tool_names array");
    let mut schema_names = responses[4]["result"]["tools"]
        .as_array()
        .expect("tools/list array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name").to_string())
        .collect::<Vec<_>>();
    schema_names.sort();
    let projected_names = listed
        .iter()
        .map(|name| name.as_str().expect("projected tool name").to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        projected_names, schema_names,
        "names_only must come from the registered tool schemas"
    );
    assert!(
        names["build"]["git_sha"].is_string(),
        "names_only keeps the build stamp"
    );

    let section: Value = serde_json::from_str(response_text(&responses[1])).unwrap();
    let section_object = section.as_object().expect("section projection object");
    assert_eq!(
        section_object.len(),
        3,
        "build projection must contain only server, section, and build metadata"
    );
    assert_eq!(section["server"], "contextpatch");
    assert_eq!(section["section"], "build");
    assert!(section_object["build"]["git_sha"].is_string());

    // An unknown section names the valid set rather than returning a silently empty document.
    let refusal = response_text(&responses[2]);
    assert_eq!(responses[2]["result"]["isError"], true);
    assert!(refusal.contains("unknown section"), "{refusal}");
    assert!(refusal.contains("valid choices: build"), "{refusal}");

    // The no-argument call keeps its full shape so existing callers are unaffected.
    let full: Value = serde_json::from_str(response_text(&responses[3])).unwrap();
    assert!(full["file_tools"].is_object());
    assert!(full["process_execution"].is_object());
    assert!(full["build"]["git_sha"].is_string());
}

#[test]
fn stage2_mcp_bounds_health_evidence_and_oversized_tool_results() {
    let root = git_repo("stage2_mcp_bounds_tool_results");
    for index in 0..105 {
        fs::write(root.join(format!("dirty-{index:03}.txt")), "dirty\n").unwrap();
    }
    fs::write(root.join("large.txt"), format!("{}\n", "a".repeat(950_000))).unwrap();
    let oversized_name = "x".repeat(950_000);
    let oversized_name_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": oversized_name,
            "arguments": {}
        }
    })
    .to_string();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"preflight_health","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"large.txt","start_line":1,"end_line":1}}}"#,
            &oversized_name_request,
        ],
    );

    let health: Value = serde_json::from_str(response_text(&responses[0])).unwrap();
    assert_eq!(health["repository"]["change_count"], 106);
    assert_eq!(
        health["repository"]["sampled_changes"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    assert_eq!(health["repository"]["sample_truncated"], true);
    assert!(
        serde_json::to_vec(&responses[0]).unwrap().len() < 1_000_000,
        "preflight response must fit below the client limit"
    );

    assert!(responses[1]["result"].get("isError").is_none());
    let omitted: Value = serde_json::from_str(response_text(&responses[1])).unwrap();
    assert_eq!(omitted["tool"], "read_range");
    assert_eq!(omitted["handler_result"], "success");
    assert_eq!(omitted["output_omitted"], true);
    assert_eq!(omitted["tool_name_truncated"], false);
    assert!(
        omitted["measured_response_bytes"].as_u64().unwrap() > 900 * 1024,
        "the fixture must exercise the response-envelope fallback"
    );
    assert_text(&responses[1], "do not retry a mutation");
    assert!(
        serde_json::to_vec(&responses[1]).unwrap().len() <= 900 * 1024,
        "the compact success fallback must fit the response envelope"
    );

    assert_eq!(responses[2]["result"]["isError"], true);
    let diagnostic: Value = serde_json::from_str(response_text(&responses[2])).unwrap();
    assert_eq!(diagnostic["handler_result"], "error");
    assert_eq!(diagnostic["diagnostic_omitted"], true);
    assert_eq!(diagnostic["tool_name_truncated"], true);
    assert_eq!(
        diagnostic["tool"].as_str().unwrap().len(),
        256,
        "fallback request metadata must itself be bounded"
    );
    assert!(
        diagnostic["measured_response_bytes"].as_u64().unwrap() > 900 * 1024,
        "the fixture must exercise the oversized-diagnostic fallback"
    );
    assert!(
        serde_json::to_vec(&responses[2]).unwrap().len() <= 900 * 1024,
        "the compact error fallback must fit the response envelope"
    );
}

#[test]
fn stage1_mcp_initialize_carries_client_instructions() {
    // The instructions field is what a client surfaces to the model before its first tool call, so its
    // presence is a contract rather than a nicety. Asserted through the real handshake rather than
    // against the constant, because the failure mode being guarded is forgetting to wire it up.
    let root = git_repo("stage1_mcp_initialize_carries_client_instructions");
    let responses = run_server(
        &root,
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#],
    );

    let instructions = responses[0]["result"]["instructions"]
        .as_str()
        .expect("initialize result must carry instructions");

    assert!(
        instructions.contains("capability_manifest"),
        "instructions must point at the discovery tool: {instructions}"
    );
    assert!(
        instructions.contains("build.git_sha"),
        "instructions must explain how to detect a stale install: {instructions}"
    );
    assert!(
        instructions.contains("read_write_receipts"),
        "instructions must name the recovery path for an interrupted mutation: {instructions}"
    );
}

#[test]
fn full_and_project_surfaces_refuse_unknown_action_arguments() {
    let root = git_repo("surfaces_refuse_unknown_action_arguments");
    fs::write(root.join("sample.txt"), "alpha\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let direct = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1,"dry_run":true}}}"#,
        ],
    );
    let wrapped = run_server_project(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_execute","arguments":{"action":"read_range","arguments":{"path":"sample.txt","start_line":1,"end_line":1,"dry_run":true}}}}"#,
        ],
    );

    for response in [&direct[0], &wrapped[0]] {
        assert_eq!(response["result"]["isError"], true);
        assert_text(response, "read_range refused: unknown argument `dry_run`");
        assert_text(response, "permitted arguments: end_line, path, start_line");
    }
}

#[test]
fn stage1_mcp_refusals_are_tool_results() {
    let root = git_repo("stage1_mcp_refusals_are_tool_results");
    fs::write(root.join("sample.txt"), "beta beta\n").unwrap();
    fs::create_dir(root.join("existing")).unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"sample.txt","content":"replacement"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_directory","arguments":{"path":"existing"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"bad.bin","content_base64":"not base64!"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"mismatch.bin","content_base64":"AAEC","expected_bytes":4}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"artifact_write_base64","arguments":{"path":"bad.bin","content_base64":"not base64!"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"delete_untracked_exact","arguments":{"paths":["sample.txt"],"dry_run":false,"confirm":"delete untracked files"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "expected exactly one match");
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "already exists");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "already exists");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "invalid characters");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "expected 4");
    assert_eq!(responses[5]["result"]["isError"], true);
    assert_text(&responses[5], "invalid characters");
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "not_untracked_paths");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "beta beta\n"
    );
    assert!(!root.join("bad.bin").exists());
    assert!(!root.join("mismatch.bin").exists());
}
