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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// A build script cannot depend on the crate it builds, so the deterministic fingerprint helpers are
// included directly. The crate compiles the same file as `contextpatch_core::provenance`, where
// their tests live, so the shipped stamp and the tested logic cannot drift apart.
include!("src/provenance.rs");

/// Emitted when a value cannot be determined, so the field is always present and always a string.
const UNKNOWN: &str = "unknown";

/// Paths handed to one `git hash-object` invocation, so a large dirty set neither spawns a process
/// per file nor overruns the platform argument limit.
const DIGEST_BATCH_SIZE: usize = 64;

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

/// Digest the current content of each dirty path with `git hash-object`.
///
/// Git computes the digests, so file bytes never enter this process and cannot reach the emitted
/// stamp, a build log, or a failure message.
fn blob_digests(root: &Path, paths: &[String]) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();

    for batch in paths.chunks(DIGEST_BATCH_SIZE) {
        let mut command = Command::new("git");
        command.current_dir(root).args(["hash-object", "--"]);
        command.args(batch);

        let Ok(output) = command.output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }

        // One digest per input path, in the order the paths were given.
        let listing = String::from_utf8_lossy(&output.stdout);
        for (path, digest) in batch.iter().zip(listing.lines()) {
            digests.insert(path.clone(), digest.trim().to_string());
        }
    }

    digests
}

/// Collect every uncommitted path together with a digest of its current content.
///
/// Returns `None` only when the status listing itself could not be read, which the caller reports as
/// unknown rather than silently as clean.
fn dirty_entries(root: &Path) -> Option<Vec<DirtyEntry>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let records: Vec<String> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8_lossy(record).to_string())
        .collect();

    let parsed = parse_porcelain_records(&records);
    let present: Vec<String> = parsed
        .iter()
        .map(|(_, path)| path.clone())
        .filter(|path| root.join(path).exists())
        .collect();
    let digests = blob_digests(root, &present);

    Some(
        parsed
            .into_iter()
            .map(|(status, path)| DirtyEntry {
                content_digest: digests
                    .get(&path)
                    .cloned()
                    .unwrap_or_else(|| ABSENT_CONTENT_DIGEST.to_string()),
                path,
                status,
            })
            .collect(),
    )
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

    // A dirty boolean cannot pin a build: the sha still matches HEAD, so comparing the reported sha
    // against the checked-out commit returns a false all-clear. The fingerprint distinguishes two
    // builds made from different dirty trees at the same commit.
    let dirty_fingerprint = root
        .as_deref()
        .and_then(dirty_entries)
        .map(|entries| dirty_tree_fingerprint(&entries))
        .unwrap_or_else(|| UNKNOWN.to_string());

    println!("cargo:rustc-env=CONTEXTPATCH_BUILD_GIT_SHA={sha}");
    println!("cargo:rustc-env=CONTEXTPATCH_BUILD_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=CONTEXTPATCH_BUILD_GIT_DIRTY_FINGERPRINT={dirty_fingerprint}");
    println!(
        "cargo:rustc-env=CONTEXTPATCH_BUILD_TIMESTAMP={}",
        build_timestamp()
    );
    println!(
        "cargo:rustc-env=CONTEXTPATCH_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| UNKNOWN.to_string())
    );
}
