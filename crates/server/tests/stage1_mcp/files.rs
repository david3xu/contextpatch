use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::support::*;

#[test]
fn stage2_bulk_replace_exact_validates_before_writing_and_applies_per_file() {
    // Validation is batch-wide, but application is deliberately per-file. A validation refusal must
    // leave every target unchanged; successful writes are individually atomic and journalled.
    let root = git_repo("stage2_bulk_replace_exact_validates_then_applies");
    fs::write(root.join("one.txt"), "alpha beta gamma\n").unwrap();
    fs::write(root.join("two.txt"), "delta delta\n").unwrap();
    fs::write(root.join("three.txt"), "epsilon\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    // Separate sessions so file state can be inspected between batches: a single run_server call
    // executes every request before returning, which would let a later success mask an earlier refusal.
    let ambiguous = run_server(
        &root,
        &[
            // two.txt has "delta" twice, so validation must refuse before one.txt is written.
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_replace_exact","arguments":{"entries":[{"path":"one.txt","old":"beta","new":"BETA"},{"path":"two.txt","old":"delta","new":"DELTA"}]}}}"#,
        ],
    );
    let refusal = response_text(&ambiguous[0]);
    assert_eq!(ambiguous[0]["result"]["isError"], true);
    assert!(refusal.contains("matched 2 times"), "{refusal}");
    assert!(refusal.contains("no file was changed"), "{refusal}");
    assert_eq!(
        fs::read_to_string(root.join("one.txt")).unwrap(),
        "alpha beta gamma\n",
        "an earlier entry must not survive a later refusal"
    );
    assert_eq!(
        fs::read_to_string(root.join("two.txt")).unwrap(),
        "delta delta\n"
    );

    let duplicated = run_server(
        &root,
        &[
            // A duplicate path is refused rather than applied twice against shifting content.
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_replace_exact","arguments":{"entries":[{"path":"one.txt","old":"beta","new":"BETA"},{"path":"one.txt","old":"gamma","new":"GAMMA"}]}}}"#,
        ],
    );
    let duplicate = response_text(&duplicated[0]);
    assert_eq!(duplicated[0]["result"]["isError"], true);
    assert!(duplicate.contains("duplicate target path"), "{duplicate}");
    assert_eq!(
        fs::read_to_string(root.join("one.txt")).unwrap(),
        "alpha beta gamma\n"
    );

    let succeeded = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_replace_exact","arguments":{"entries":[{"path":"one.txt","old":"beta","new":"BETA"},{"path":"three.txt","old":"epsilon","new":"EPSILON"}]}}}"#,
        ],
    );
    let applied: Value = serde_json::from_str(response_text(&succeeded[0])).unwrap();
    assert_eq!(applied["applied"], 2);
    assert_eq!(applied["atomicity"], "per_file");
    assert_eq!(
        fs::read_to_string(root.join("one.txt")).unwrap(),
        "alpha BETA gamma\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("three.txt")).unwrap(),
        "EPSILON\n"
    );

    // Each write is journalled individually, so an interrupted apply phase stays recoverable.
    let receipts = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_write_receipts","arguments":{"limit":10}}}"#,
        ],
    );
    let journal: Value = serde_json::from_str(response_text(&receipts[0])).unwrap();
    let listed = journal["receipts"].as_array().unwrap();
    assert!(
        listed
            .iter()
            .filter(|entry| entry["tool"] == "bulk_replace_exact")
            .count()
            >= 2,
        "each file in a batch needs its own receipt: {journal}"
    );
}

#[test]
fn stage1_mcp_tools_work_together() {
    let root = git_repo("stage1_mcp_tools_work_together");
    fs::write(root.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();
    fs::write(root.join("scratch.log"), "temporary\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_range","arguments":{"path":"sample.txt","start_line":2,"end_line":3}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"diff_preview","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"create_directory","arguments":{"path":"native-plugins/background-audio","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"native-plugins/background-audio/plugin.ts","content":"new file\n"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"write_new_file_base64","arguments":{"path":"fixture.bin","content_base64":"AAEC/w==","expected_bytes":4}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"artifact_write_text","arguments":{"path":"stage1/tool.txt","content":"sidecar\n","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"delete_untracked_exact","arguments":{"paths":["scratch.log"],"dry_run":false,"confirm":"delete untracked files"}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"git_remote_list","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"replace_exact","arguments":{"path":"sample.txt","old":"beta","new":"delta"}}}"#,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"status_guard","arguments":{"path":"sample.txt"}}}"#,
        ],
    );

    let list = &responses[0]["result"]["tools"];
    assert_eq!(list.as_array().unwrap().len(), 52, "{list}");
    for name in [
        "capability_manifest",
        "preflight_health",
        "read_range",
        "read_write_receipts",
        "diff_preview",
        "replace_exact",
        "bulk_replace_exact",
        "status_guard",
        "write_new_file",
        "write_new_file_base64",
        "write_existing_file_exact_hash",
        "file_info",
        "set_file_executable",
        "list_directory",
        "read_file_bytes",
        "artifact_write_text",
        "artifact_write_base64",
        "artifact_delete_exact",
        "bulk_write_new_files_base64",
        "create_directory",
        "run_guarded_command",
        "artifact_python_run",
        "task_image_python_run",
        "harbor_run_start",
        "image_cleanliness_check_run",
        "docker_image_inspect",
        "fixture_generator_run",
        "base_image_check_run",
        "fixture_manifest_verify",
        "fixture_manifest_refresh",
        "read_command_log",
        "validation_profile_run",
        "setup_profile_run",
        "native_build_run",
        "native_device_run",
        "git_commit_exact",
        "git_commit_scoped",
        "git_commit_prefix",
        "git_stage_exact",
        "git_staged_scope_check",
        "git_restore_exact",
        "move_tracked",
        "delete_guarded",
        "delete_untracked_exact",
        "delete_generated_prefix",
        "git_remote_list",
        "git_remote_check",
        "git_branch_prepare",
        "git_merge_readiness",
        "git_push_exact",
        "github_pr_run",
        "github_fork_prepare",
    ] {
        assert!(
            list.as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name),
            "tools/list did not include {name}: {list}"
        );
    }
    for unavailable in ["apply_patch", "insert_at_anchor"] {
        assert!(
            list.as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["name"] != unavailable),
            "tools/list advertised unsupported capability {unavailable}: {list}"
        );
    }
    for tool in list.as_array().unwrap() {
        let annotations = &tool["annotations"];
        assert!(
            annotations.is_object(),
            "tools/list did not include annotations for {}: {tool}",
            tool["name"]
        );
        assert_eq!(annotations["destructiveHint"], false, "{tool}");
        assert_eq!(
            annotations["idempotentHint"], annotations["readOnlyHint"],
            "{tool}"
        );
        assert_eq!(annotations["openWorldHint"], false, "{tool}");
    }
    for read_only_tool in [
        "capability_manifest",
        "preflight_health",
        "read_range",
        "read_write_receipts",
        "diff_preview",
        "status_guard",
        "file_info",
        "list_directory",
        "read_file_bytes",
        "read_command_log",
        "fixture_manifest_verify",
        "git_remote_list",
        "git_merge_readiness",
        "git_staged_scope_check",
    ] {
        let tool = list
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == read_only_tool)
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
    let guarded_command = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "run_guarded_command")
        .unwrap();
    assert_eq!(
        guarded_command["inputSchema"]["properties"]["timeout_secs"]["maximum"],
        600
    );
    let validation_profile = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "validation_profile_run")
        .unwrap();
    assert_eq!(
        validation_profile["inputSchema"]["properties"]["timeout_secs"]["maximum"],
        600
    );
    let receipts = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "read_write_receipts")
        .unwrap();
    assert_eq!(
        receipts["inputSchema"]["properties"]["limit"]["maximum"],
        100
    );
    let replace_exact = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "replace_exact")
        .unwrap();
    assert_eq!(
        replace_exact["inputSchema"]["properties"]["expected_sha256"]["pattern"],
        "^[0-9a-f]{64}$"
    );
    let bulk_replace_exact = list
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bulk_replace_exact")
        .unwrap();
    assert_eq!(
        bulk_replace_exact["inputSchema"]["properties"]["entries"]["maxItems"],
        64
    );
    assert_eq!(
        bulk_replace_exact["inputSchema"]["properties"]["entries"]["items"]["properties"]
            ["expected_sha256"]["pattern"],
        "^[0-9a-f]{64}$"
    );

    assert_text(&responses[1], "clean: no Git changes under sample.txt");
    assert_text(&responses[2], "2. beta\n3. gamma\n");
    assert_text(&responses[3], "-beta\n+delta");
    assert_text(&responses[4], "created");
    assert_text(&responses[5], "created");
    assert_text(&responses[6], "created");
    assert_text(&responses[7], "\"repo_mutation\": false");
    assert_text(&responses[8], "\"deleted\": true");
    assert_text(&responses[9], "\"tool\": \"git_remote_list\"");
    assert_text(&responses[10], "replaced bytes");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("native-plugins/background-audio/plugin.ts")).unwrap(),
        "new file\n"
    );
    assert_eq!(
        fs::read(root.join("fixture.bin")).unwrap(),
        vec![0, 1, 2, 255]
    );
    assert!(!root.join("scratch.log").exists());

    assert_eq!(responses[11]["result"]["isError"], true);
    assert_text(&responses[11], "status_guard refused");
    assert_text(&responses[11], "sample.txt");
}

#[test]
fn replace_exact_hash_guard_and_file_receipts_are_end_to_end() {
    let root = git_repo("replace_exact_hash_guard_and_file_receipts_are_end_to_end");
    fs::write(root.join("sample.txt"), "alpha beta gamma\n").unwrap();
    git(&root, &["add", "sample.txt"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let current_sha256 = sha256_hex_for_test(b"alpha beta gamma\n");
    let stale_sha256 = sha256_hex_for_test(b"stale content\n");
    let stale_request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"replace_exact","arguments":{{"path":"sample.txt","old":"beta","new":"delta","expected_sha256":"{stale_sha256}"}}}}}}"#
    );
    let guarded_request = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"replace_exact","arguments":{{"path":"sample.txt","old":"beta","new":"delta","expected_sha256":"{current_sha256}"}}}}}}"#
    );
    let responses = run_server_sequential(
        &root,
        &[
            &stale_request,
            &guarded_request,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_write_receipts","arguments":{"limit":10}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "SHA-256 mismatch");
    assert_text(&responses[1], "replaced bytes");
    assert_eq!(
        fs::read_to_string(root.join("sample.txt")).unwrap(),
        "alpha delta gamma\n"
    );

    let receipts: Value = serde_json::from_str(response_text(&responses[2])).unwrap();
    assert_eq!(receipts["returned"], 2);
    assert_eq!(receipts["receipts"][0]["tool"], "replace_exact");
    assert_eq!(receipts["receipts"][0]["outcome"], "applied");
    assert_eq!(receipts["receipts"][0]["before_sha256"], current_sha256);
    assert_eq!(
        receipts["receipts"][0]["after_sha256"],
        sha256_hex_for_test(b"alpha delta gamma\n")
    );
    assert_eq!(receipts["receipts"][1]["outcome"], "refused");
    assert_eq!(receipts["receipts"][1]["before_sha256"], current_sha256);
}

#[test]
fn stage2_file_inspection_tools_report_digest_listing_and_binary_ranges() {
    let root = git_repo("stage2_file_inspection_tools_report_digest_listing_and_binary_ranges");
    fs::create_dir(root.join("data")).unwrap();
    fs::create_dir(root.join("data/nested")).unwrap();
    fs::write(root.join("data/sample.txt"), "alpha\nbeta\n").unwrap();
    fs::write(root.join("data/blob.bin"), [0, 1, 2, 255, 16, 32]).unwrap();
    fs::write(root.join("data/nested/deep.txt"), "deep\n").unwrap();
    fs::write(root.join("data/.hidden"), "hidden\n").unwrap();
    let digest = Sha256::digest(b"alpha\nbeta\n")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"data/sample.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"data"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file_bytes","arguments":{"path":"data/blob.bin","offset":1,"max_bytes":3,"encoding":"hex"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_directory","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":""}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"file_info","arguments":{"paths":["data/sample.txt","data/blob.bin","data/missing.txt"]}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"file_info","arguments":{"paths":[]}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"data/sample.txt","paths":["data/blob.bin"]}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"data","recursive":true,"max_depth":2,"max_entries":20}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"data","recursive":true,"max_depth":2,"max_entries":2}}}"#,
        ],
    );

    assert_text(&responses[0], "\"sha256\"");
    assert_text(&responses[0], &digest);
    assert_text(&responses[0], "\"line_count\": 2");
    assert_text(&responses[0], "\"is_symlink\": false");
    assert_text(&responses[1], "\"entry_count\": 3");
    assert_text(&responses[1], "\"path\": \"data/blob.bin\"");
    assert_text(&responses[1], "\"size_bytes\": 6");
    assert_text(&responses[2], "\"total_bytes\": 6");
    assert_text(&responses[2], "\"bytes_returned\": 3");
    assert_text(&responses[2], "\"data\": \"0102ff\"");
    assert_text(&responses[3], "\"path\": \".\"");
    assert_text(&responses[3], "\"path\": \"data\"");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(
        &responses[4],
        "path must be `.` or a normalized repository-relative path",
    );
    assert_text(&responses[5], "\"path_count\": 3");
    assert_text(&responses[5], "\"path\": \"data/missing.txt\"");
    assert_text(&responses[5], "\"exists\": false");
    assert_text(&responses[5], "\"mode\":");
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "paths must not be empty");
    assert_eq!(responses[7]["result"]["isError"], true);
    assert_text(&responses[7], "either `path` or `paths`");
    assert_text(&responses[8], "\"path\": \"data/nested/deep.txt\"");
    assert_text(&responses[8], "\"depth\": 2");
    assert_text(&responses[8], "\"recursive\": true");
    assert_text(&responses[8], "\"truncated\": false");
    assert!(!response_text(&responses[8]).contains("data/.hidden"));
    assert_text(&responses[9], "\"entry_count\": 2");
    assert_text(&responses[9], "\"truncated\": true");

    #[cfg(unix)]
    {
        let outside = temp_root("stage2_file_inspection_outside_target");
        fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("data/outside-link"))
            .unwrap();
        let symlink_responses = run_server(
            &root,
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"data/outside-link"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file_bytes","arguments":{"path":"data/outside-link","max_bytes":4}}}"#,
            ],
        );

        assert_text(&symlink_responses[0], "\"is_symlink\": true");
        assert_text(
            &symlink_responses[0],
            "\"symlink_resolves_inside_repo\": false",
        );
        assert_text(&symlink_responses[0], "\"sha256\": null");
        assert_eq!(symlink_responses[1]["result"]["isError"], true);
        assert_text(&symlink_responses[1], "contains a symlink component");
    }
}

#[test]
fn file_inspection_streams_large_file_facts_and_returns_only_the_requested_range() {
    let root = git_repo("file_inspection_streams_large_file_facts");
    let target = root.join("large.bin");
    let mut file = fs::File::create(&target).unwrap();
    let chunk = [b'x'; 64 * 1024];
    let mut hasher = Sha256::new();
    for _ in 0..256 {
        file.write_all(&chunk).unwrap();
        hasher.update(chunk);
    }
    file.write_all(b"tail-data").unwrap();
    hasher.update(b"tail-data");
    file.flush().unwrap();
    let offset = 16_u64 * 1024 * 1024;
    let expected_sha256 = format!("{:x}", hasher.finalize());
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"read_file_bytes","arguments":{{"path":"large.bin","offset":{offset},"max_bytes":4,"encoding":"hex"}}}}}}"#
    );

    let range_responses = run_server(&root, &[&request]);
    let result: Value = serde_json::from_str(response_text(&range_responses[0])).unwrap();

    assert_eq!(result["offset"], offset);
    assert_eq!(result["bytes_returned"], 4);
    assert_eq!(result["total_bytes"], offset + 9);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["sha256"], expected_sha256);
    assert_eq!(result["data"], "7461696c");

    let info_responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"large.bin"}}}"#,
        ],
    );
    let info: Value = serde_json::from_str(response_text(&info_responses[0])).unwrap();

    assert_eq!(info["size_bytes"], offset + 9);
    assert_eq!(info["sha256"], expected_sha256);
    assert_eq!(info["line_count"], 1);
}

#[cfg(unix)]
#[test]
fn file_tools_refuse_intermediate_symlink_components_without_touching_outside_files() {
    let root = git_repo("file_tools_refuse_intermediate_symlinks");
    let outside = temp_root("file_tools_intermediate_symlink_outside");
    let secret = outside.join("secret.txt");
    fs::write(&secret, "outside\n").unwrap();
    let before_mode = fs::metadata(&secret).unwrap().permissions().mode() & 0o7777;
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

    let digest = sha256_hex_for_test(b"outside\n");
    let overwrite = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"linked/secret.txt","content":"changed\n","expected_sha256":"{digest}","dry_run":false,"confirm":"write exact hash"}}}}}}"#
    );
    let chmod = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"set_file_executable","arguments":{{"path":"linked/secret.txt","executable":true,"expected_sha256":"{digest}","expected_mode":"{before_mode:04o}","dry_run":false,"confirm":"set file executable"}}}}}}"#
    );
    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"linked/secret.txt"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file_bytes","arguments":{"path":"linked/secret.txt","max_bytes":8}}}"#,
            &overwrite,
            &chmod,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_new_file","arguments":{"path":"linked/new.txt","content":"new\n"}}}"#,
        ],
    );

    for response in &responses {
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert_text(response, "symlink or non-directory component");
    }
    assert_eq!(fs::read_to_string(&secret).unwrap(), "outside\n");
    assert_eq!(
        fs::metadata(&secret).unwrap().permissions().mode() & 0o7777,
        before_mode
    );
    assert!(!outside.join("new.txt").exists());
}

#[cfg(unix)]
#[test]
fn stage2_set_file_executable_uses_current_hash_and_mode_without_changing_content() {
    let root = git_repo("stage2_set_file_executable");
    let target = root.join("test.sh");
    fs::write(&target, "#!/bin/sh\nprintf 'ok\\n'\n").unwrap();
    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_mode(0o640);
    fs::set_permissions(&target, permissions).unwrap();

    let plan_responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"set_file_executable","arguments":{"path":"test.sh","executable":true}}}"#,
        ],
    );
    let plan: Value = serde_json::from_str(response_text(&plan_responses[0])).unwrap();
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["before_mode"], "0640");
    assert_eq!(plan["after_mode"], "0751");
    let digest = plan["sha256"].as_str().unwrap();

    let stale_mode = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"set_file_executable","arguments":{{"path":"test.sh","executable":true,"expected_sha256":"{digest}","expected_mode":"0600","dry_run":false,"confirm":"set file executable"}}}}}}"#
    );
    let apply = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"set_file_executable","arguments":{{"path":"test.sh","executable":true,"expected_sha256":"{digest}","expected_mode":"0640","dry_run":false,"confirm":"set file executable"}}}}}}"#
    );
    let responses = run_server_sequential(
        &root,
        &[
            &stale_mode,
            &apply,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"file_info","arguments":{"path":"test.sh"}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "mode mismatch");
    assert_text(&responses[1], "\"changed\": true");
    assert_text(&responses[1], "\"after_mode\": \"0751\"");
    assert_text(&responses[2], "\"executable\": true");
    assert_text(&responses[2], "\"mode\": \"0751\"");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "#!/bin/sh\nprintf 'ok\\n'\n"
    );
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o751
    );
}

#[cfg(unix)]
#[test]
fn stage2_set_file_executable_accepts_three_digit_modes_and_refuses_hard_links() {
    let root = git_repo("stage2_set_file_executable_mode_and_hard_links");
    let target = root.join("test.sh");
    fs::write(&target, "#!/bin/sh\n").unwrap();
    let mut permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_mode(0o640);
    fs::set_permissions(&target, permissions).unwrap();
    let digest = sha256_hex_for_test(b"#!/bin/sh\n");

    let apply = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"set_file_executable","arguments":{{"path":"test.sh","executable":true,"expected_sha256":"{digest}","expected_mode":"640","dry_run":false,"confirm":"set file executable"}}}}}}"#
    );
    let applied = run_server(&root, &[&apply]);
    assert_text(&applied[0], "\"changed\": true");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o751
    );

    permissions = fs::metadata(&target).unwrap().permissions();
    permissions.set_mode(0o640);
    fs::set_permissions(&target, permissions).unwrap();
    let outside = temp_root("stage2_set_file_executable_hard_link_outside");
    let alias = outside.join("alias.sh");
    fs::hard_link(&target, &alias).unwrap();

    let refused = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"set_file_executable","arguments":{"path":"test.sh","executable":true}}}"#,
        ],
    );
    assert_eq!(refused[0]["result"]["isError"], true);
    assert_text(&refused[0], "hard links");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(
        fs::metadata(&alias).unwrap().permissions().mode() & 0o7777,
        0o640
    );
}

#[test]
fn stage2_artifact_python_run_executes_scratch_outside_repo() {
    let root = git_repo("stage2_artifact_python_run_executes_scratch_outside_repo");

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"artifact_write_text","arguments":{"path":"scratch.py","content":"print('scratch-ok')\n","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"artifact_python_run","arguments":{"script":"scratch.py","timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "\"repo_mutation\": false");
    assert_text(&responses[1], "scratch-ok");
    assert!(!root.join("scratch.py").exists());
    assert_eq!(git_stdout(&root, &["status", "--short"]), "");
}

#[test]
fn stage2_artifact_delete_exact_requires_current_hash_and_deletes_only_the_file() {
    let root = git_repo("stage2_artifact_delete_exact_requires_current_hash");
    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"artifact_write_text","arguments":{"path":"cleanup/tool.txt","content":"sidecar\n","parents":true}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"artifact_delete_exact","arguments":{"path":"cleanup/tool.txt"}}}"#,
        ],
    );

    let dry_run: Value = serde_json::from_str(response_text(&responses[1])).unwrap();
    let artifact_root = PathBuf::from(dry_run["artifact_root"].as_str().unwrap());
    let target = artifact_root.join("cleanup/tool.txt");
    let sha256 = dry_run["sha256"].as_str().unwrap();
    assert!(target.is_file());
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["deleted"], false);
    assert_eq!(dry_run["repo_mutation"], false);

    let wrong_hash_request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"artifact_delete_exact","arguments":{{"path":"cleanup/tool.txt","expected_sha256":"{}","dry_run":false,"confirm":"delete artifact exact"}}}}}}"#,
        "0".repeat(64)
    );
    let refused = run_server(&root, &[&wrong_hash_request]);
    assert_eq!(refused[0]["result"]["isError"], true);
    assert_text(&refused[0], "hash mismatch");
    assert!(target.is_file());

    let delete_request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"artifact_delete_exact","arguments":{{"path":"cleanup/tool.txt","expected_sha256":"{sha256}","dry_run":false,"confirm":"delete artifact exact"}}}}}}"#
    );
    let deleted = run_server(&root, &[&delete_request]);
    assert_text(&deleted[0], "\"deleted\": true");
    assert_text(&deleted[0], "\"repo_mutation\": false");
    assert!(!target.exists());
    assert!(artifact_root.join("cleanup").is_dir());
    assert_eq!(git_stdout(&root, &["status", "--short"]), "");

    let missing = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"artifact_delete_exact","arguments":{"path":"cleanup/tool.txt"}}}"#,
        ],
    );
    assert_eq!(missing[0]["result"]["isError"], true);
    assert_text(&missing[0], "does not exist");
}

#[test]
fn stage2_manifest_and_hash_tools_cover_fixture_integrity_workflow() {
    let root = git_repo("stage2_manifest_and_hash_tools_cover_fixture_integrity_workflow");
    fs::create_dir_all(root.join("task/environment/data")).unwrap();
    fs::create_dir_all(root.join("task/tests")).unwrap();
    fs::write(root.join("task/environment/data/a.bin"), [1_u8, 2, 3]).unwrap();
    fs::write(root.join("task/environment/data/b.txt"), "hello\n").unwrap();
    fs::write(root.join("README.md"), "before\n").unwrap();
    git(&root, &["add", "README.md", "task/environment/data"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let readme_sha = sha256_hex_for_test(b"before\n");
    let dry_write = run_server(
        &root,
        &[&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":true}}}}}}"#
        )],
    );
    assert_text(&dry_write[0], "\"dry_run\": true");
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "before\n"
    );

    let refresh_confirm = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":false,"confirm":"refresh fixture manifest"}}}"#;
    let responses = run_server_sequential(
        &root,
        &[
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":false}}}}}}"#
            ),
            &format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"write_existing_file_exact_hash","arguments":{{"path":"README.md","content":"after\n","expected_sha256":"{readme_sha}","dry_run":false,"confirm":"write exact hash"}}}}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":true}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fixture_manifest_refresh","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"],"dry_run":false}}}"#,
            refresh_confirm,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fixture_manifest_verify","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"]}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "requires confirm");
    assert_text(&responses[1], "\"wrote\": true");
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "after\n"
    );
    assert_text(&responses[2], "\"dry_run\": true");
    assert_text(&responses[2], "\"file_count\": 2");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "requires confirm");
    assert_text(&responses[4], "\"refreshed\": true");
    assert_text(&responses[5], "\"verified\": true");

    fs::write(root.join("task/environment/data/b.txt"), "changed\n").unwrap();
    let verify_again = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fixture_manifest_verify","arguments":{"manifest_path":"task/tests/fixture_manifest.json","fixture_prefixes":["task/environment/data"]}}}"#,
        ],
    );
    assert_eq!(verify_again[0]["result"]["isError"], true);
    assert_text(&verify_again[0], "\"verified\": false");
    assert_text(&verify_again[0], "\"modified_files\"");
}
