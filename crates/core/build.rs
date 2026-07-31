//! Capture build provenance so a client can tell which binary it is actually talking to.
//!
//! `CARGO_PKG_VERSION` alone is useless for this: it stays `0.1.0` across every rebuild, so a
//! client that finds a tool missing cannot distinguish "this capability does not exist" from "this
//! server predates the capability". That ambiguity is expensive in practice, because the reasonable
//! inference is the wrong one, and an agent will confidently report a capability as unavailable when
//! a reinstall would have provided it.
//!
//! Emits the commit the binary was built from, whether that worktree was dirty, and when. No
//! external crates: `git` and `date` are invoked directly, and every failure degrades to a marker
//! string rather than breaking the build, because provenance is diagnostic and must never be the
//! reason a build fails.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Emitted when a value cannot be determined, so the field is always present and always a string.
const UNKNOWN: &str = "unknown";

fn repository_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    manifest
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git_output(root, &["rev-parse", "--git-path", name])?);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn listed_repository_files(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| String::from_utf8(entry.to_vec()).ok())
        .map(|relative| root.join(relative))
        .collect()
}

fn build_timestamp() -> String {
    // Prefer a readable UTC instant. `date` is present on every platform this server targets, and
    // falling back to epoch seconds keeps the field meaningful where it is not.
    if let Some(output) = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|output| output.status.success())
    {
        return String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| format!("epoch:{}", elapsed.as_secs()))
        .unwrap_or_else(|_| UNKNOWN.to_string())
}

fn main() {
    let root = repository_root();

    // Rebuild the stamp whenever HEAD, the index, or any Git-visible worktree file changes. Watching
    // the repository directory itself would recursively include target/ and cause rebuild loops.
    if let Some(root) = root.as_deref() {
        let mut watched = vec![
            git_path(root, "HEAD"),
            git_path(root, "index"),
            git_path(root, "packed-refs"),
        ];
        if let Some(reference) = git_output(root, &["symbolic-ref", "-q", "HEAD"]) {
            watched.push(git_path(root, &reference));
        }
        watched.extend(listed_repository_files(root).into_iter().map(Some));
        for path in watched.into_iter().flatten() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let sha = root
        .as_deref()
        .and_then(|root| git_output(root, &["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| UNKNOWN.to_string());

    // An empty porcelain listing means the build came from a clean worktree. Anything else means the
    // binary contains changes that exist on no commit, which a reviewer needs to know.
    let dirty = root
        .as_deref()
        .and_then(|root| git_output(root, &["status", "--porcelain"]))
        .map(|listing| if listing.is_empty() { "false" } else { "true" }.to_string())
        .unwrap_or_else(|| UNKNOWN.to_string());

    println!("cargo:rustc-env=CONTEXTPATCH_BUILD_GIT_SHA={sha}");
    println!("cargo:rustc-env=CONTEXTPATCH_BUILD_GIT_DIRTY={dirty}");
    println!(
        "cargo:rustc-env=CONTEXTPATCH_BUILD_TIMESTAMP={}",
        build_timestamp()
    );
    println!(
        "cargo:rustc-env=CONTEXTPATCH_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| UNKNOWN.to_string())
    );
}
