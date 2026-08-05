use std::fs;

use crate::support::*;

#[test]
fn stage2_fixture_workflow_tools_cover_generator_base_image_and_bulk_import() {
    let root = git_repo("stage2_fixture_workflow_tools_cover_generator_base_image_and_bulk_import");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("references")).unwrap();
    fs::write(root.join("README.md"), "readme\n").unwrap();
    fs::write(
        root.join("scripts/gen.py"),
        r#"import pathlib
import sys

target = pathlib.Path(sys.argv[1])
target.parent.mkdir(parents=True, exist_ok=True)
target.write_bytes(b"\x00fixture")
if len(sys.argv) > 2:
    pathlib.Path(sys.argv[2]).write_text("rogue\n")
"#,
    )
    .unwrap();
    fs::write(
        root.join("references/check-base-image.sh"),
        "#!/usr/bin/env bash\nif [ \"${1:-}\" = task ]; then exit 0; fi\nexit 1\n",
    )
    .unwrap();
    git(
        &root,
        &["add", "README.md", "references/check-base-image.sh"],
    );
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"parents":true,"entries":[{"path":"task/environment/data/a.bin","content_base64":"AQI=","expected_bytes":2},{"path":"task/environment/data/b.txt","content_base64":"aGkK","expected_bytes":3}]}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"entries":[{"path":"task/environment/data/a.bin","content_base64":"AA=="}]}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"entries":[{"path":"../escape.bin","content_base64":"AA=="}]}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/dry.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out.bin"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"confirm":"run fixture generator","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fixture_generator_run","arguments":{"script_path":"scripts/gen.py","args":["generated/out2.bin","rogue.txt"],"expected_output_prefixes":["generated"],"allowed_existing_dirty_paths":["scripts/gen.py","generated/out.bin","task/environment/data/a.bin","task/environment/data/b.txt"],"dry_run":false,"confirm":"run fixture generator","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"base_image_check_run","arguments":{"project_path":"task","dry_run":false,"confirm":"run base image check","timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"bash","args":["scripts/gen.py"],"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "\"file_count\": 2");
    assert_eq!(
        fs::read(root.join("task/environment/data/a.bin")).unwrap(),
        [1, 2]
    );
    assert_eq!(
        fs::read_to_string(root.join("task/environment/data/b.txt")).unwrap(),
        "hi\n"
    );
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_text(&responses[1], "already exists");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "normalized relative path");
    assert_text(&responses[3], "\"dry_run\": true");
    assert!(!root.join("generated/dry.bin").exists());
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "confirm must be");
    assert_text(&responses[5], "\"ran\": true");
    assert_eq!(
        fs::read(root.join("generated/out.bin")).unwrap(),
        b"\0fixture"
    );
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "outside declared outputs");
    assert_text(&responses[6], "rogue.txt");
    assert_text(&responses[7], "\"dry_run\": true");
    assert_text(&responses[7], "references/check-base-image.sh");
    assert_text(&responses[7], "task");
    assert_eq!(responses[8]["result"]["isError"], true);
    assert_text(&responses[8], "confirm must be");
    assert_text(&responses[9], "\"ran\": true");
    assert_eq!(responses[10]["result"]["isError"], true);
    assert_text(&responses[10], "not allowlisted");
}

#[cfg(unix)]
#[test]
fn bulk_write_refuses_symlinked_parent_without_creating_an_outside_tree() {
    let root = git_repo("bulk_write_refuses_symlinked_parent");
    let outside = temp_root("bulk_write_symlinked_parent_outside");
    std::os::unix::fs::symlink(&outside, root.join("fixture-link")).unwrap();

    let responses = run_server(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bulk_write_new_files_base64","arguments":{"parents":true,"entries":[{"path":"fixture-link/nested/data.bin","content_base64":"AQI=","expected_bytes":2}]}}}"#,
        ],
    );

    assert_eq!(responses[0]["result"]["isError"], true);
    assert_text(&responses[0], "symlink or non-directory component");
    assert!(!outside.join("nested").exists());
}

#[test]
fn stage2_setup_profile_run_plans_capacitor_shell_without_mutating() {
    let root = git_repo("stage2_setup_profile_run_plans_capacitor_shell_without_mutating");
    fs::write(root.join("package.json"), "{}\n").unwrap();
    fs::create_dir_all(root.join("ios/App")).unwrap();
    fs::write(root.join("ios/App/Podfile"), "target 'App' do\nend\n").unwrap();
    git(&root, &["add", "package.json", "ios/App/Podfile"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_init","params":{"app_id":"com.example.app","app_name":"Example","web_dir":"dist"},"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"ios"}}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","dry_run":false}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"windows"}}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"npm","args":["install","@capacitor/core"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"ios_pod_install","cwd":"ios/App"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"pnpm","args":["add","@capacitor/core"],"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_dependencies","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"ios_pod_install"}}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_filesystem","dry_run":true,"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "profile: node-capacitor-shell");
    assert_text(&responses[0], "action: cap_init");
    assert_text(
        &responses[0],
        "command: npm exec -- cap init Example com.example.app --web-dir dist",
    );
    assert_text(&responses[0], "external_mutator: true");
    assert_text(
        &responses[0],
        "required_confirm_for_mutation: \"run setup profile\"",
    );
    assert_text(&responses[1], "command: npm exec -- cap sync ios");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "requires confirm");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "unsupported cap_sync platform");
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_text(&responses[4], "not allowlisted");
    assert_text(&responses[5], "command: pod install");
    assert_eq!(responses[6]["result"]["isError"], true);
    assert_text(&responses[6], "not allowlisted");
    assert_text(
        &responses[7],
        "commands: npm install \"@capacitor/core\" \"@capacitor/ios\" \"@capacitor/android\"",
    );
    assert_text(&responses[7], "npm install --save-dev \"@capacitor/cli\"");
    assert_eq!(responses[8]["result"]["isError"], true);
    assert_text(&responses[8], "requires a Podfile");
    assert_text(
        &responses[9],
        "command: npm install \"@capacitor/filesystem\"",
    );
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "setup_profile_run dry-runs must not mutate the repository"
    );
}

#[test]
fn stage2_setup_profile_run_uses_pnpm_for_pnpm_projects_without_raw_allowlist() {
    let root =
        git_repo("stage2_setup_profile_run_uses_pnpm_for_pnpm_projects_without_raw_allowlist");
    fs::write(root.join("package.json"), "{}\n").unwrap();
    fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
    git(&root, &["add", "package.json", "pnpm-lock.yaml"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"install_capacitor_dependencies","dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_profile_run","arguments":{"profile":"node-capacitor-shell","action":"cap_sync","params":{"platform":"ios"},"dry_run":true,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_guarded_command","arguments":{"program":"pnpm","args":["add","@capacitor/core"],"timeout_secs":30}}}"#,
        ],
    );

    assert_text(
        &responses[0],
        "commands: pnpm add \"@capacitor/core\" \"@capacitor/ios\" \"@capacitor/android\"",
    );
    assert_text(&responses[0], "pnpm add --save-dev \"@capacitor/cli\"");
    assert_text(&responses[1], "command: pnpm exec cap sync ios");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "not allowlisted");
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "setup_profile_run dry-runs must not mutate pnpm projects"
    );
}

#[test]
fn stage2_native_build_run_plans_builds_without_raw_commands() {
    let root = git_repo("stage2_native_build_run_plans_builds_without_raw_commands");
    fs::write(root.join("gradlew"), "#!/bin/sh\nexit 0\n").unwrap();
    git(&root, &["add", "gradlew"]);
    git(&root, &["commit", "--quiet", "-m", "initial"]);

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"ios/App/App.xcworkspace","scheme":"App","derived_data_path":".contextpatch-derived-data"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"android_assemble_debug","params":{},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"native_build_run","arguments":{"action":"ios_build","params":{"workspace":"../App.xcworkspace","scheme":"App"},"timeout_secs":30}}}"#,
        ],
    );

    assert_text(&responses[0], "action: ios_build");
    assert_text(
        &responses[0],
        "command: xcodebuild -workspace \"ios/App/App.xcworkspace\" -scheme App -configuration Debug -sdk iphonesimulator -derivedDataPath .contextpatch-derived-data build",
    );
    assert_text(&responses[0], "repo_validation: true");
    assert_text(&responses[1], "command: ./gradlew assembleDebug");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "repository-relative path");
    assert_eq!(
        git_stdout(&root, &["status", "--short"]),
        "",
        "native_build_run dry-runs must not mutate the repository"
    );
}

#[test]
fn stage2_native_device_run_plans_device_actions_and_requires_confirmation() {
    let root = git_repo("stage2_native_device_run_plans_device_actions_and_requires_confirmation");

    let responses = run_server_sequential(
        &root,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_launch_app","params":{"device":"booted","app_id":"com.example.app"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"android_read_logcat","params":{"serial":"emulator-5554","lines":50},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_boot_simulator","params":{"device":"booted"},"dry_run":false,"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"android_install_app","params":{"apk_path":"../app.apk"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_create_simulator","params":{"name":"ContextPatch iPhone","device_type":"iPhone 16","runtime":"iOS 26.4"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_cap_run","params":{"target":"00000000-0000-0000-0000-000000000000"},"timeout_secs":30}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"native_device_run","arguments":{"action":"ios_read_logs","params":{"device":"booted","duration":"3"},"timeout_secs":30}}}"#,
        ],
    );

    assert_text(
        &responses[0],
        "command: xcrun simctl launch booted com.example.app",
    );
    assert_text(&responses[0], "changes_device_state: true");
    assert_text(
        &responses[1],
        "command: adb -s emulator-5554 logcat -d -t 50",
    );
    assert_text(&responses[1], "changes_device_state: false");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert_text(&responses[2], "requires confirm");
    assert_eq!(responses[3]["result"]["isError"], true);
    assert_text(&responses[3], "repository-relative path");
    assert_text(
        &responses[4],
        "command: xcrun simctl create \"ContextPatch iPhone\" \"iPhone 16\" \"iOS 26.4\"",
    );
    assert_text(
        &responses[5],
        "command: npm exec -- cap run ios --target 00000000-0000-0000-0000-000000000000 --no-sync",
    );
    assert_text(
        &responses[6],
        "command: xcrun simctl spawn booted log stream --style compact --timeout 3",
    );
    assert_text(&responses[6], "changes_device_state: false");
}
