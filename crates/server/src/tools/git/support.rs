use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::tools;
pub(crate) fn validate_commit_subject(subject: &str) -> Result<String, String> {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        return Err("git_commit_exact refused: subject must not be empty".to_string());
    }
    if trimmed.len() > 200 {
        return Err("git_commit_exact refused: subject must be at most 200 bytes".to_string());
    }
    if subject.contains('\n') || subject.contains('\r') || subject.contains('\0') {
        return Err(
            "git_commit_exact refused: subject must be a single line without NUL bytes".to_string(),
        );
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_commit_body(body: &str) -> Result<String, String> {
    if body.contains('\0') {
        return Err("git_commit_exact refused: body must not contain NUL bytes".to_string());
    }
    if body.len() > 20_000 {
        return Err("git_commit_exact refused: body must be at most 20000 bytes".to_string());
    }
    Ok(body.trim().to_string())
}

pub(crate) fn normalize_git_paths(
    tool_name: &str,
    root: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| normalize_git_path(tool_name, root, path))
        .collect()
}

pub(crate) fn normalize_git_prefixes(
    tool_name: &str,
    root: &Path,
    prefixes: &[String],
) -> Result<Vec<String>, String> {
    prefixes
        .iter()
        .map(|prefix| {
            let trimmed = prefix.trim_end_matches('/');
            if trimmed.is_empty() {
                return Err(format!("{tool_name} refused: prefixes must not be empty"));
            }
            normalize_git_path(tool_name, root, trimmed)
        })
        .collect()
}

pub(crate) fn paths_under_prefixes(
    paths: &BTreeSet<String>,
    prefixes: &[String],
) -> BTreeSet<String> {
    paths
        .iter()
        .filter(|path| {
            prefixes
                .iter()
                .any(|prefix| *path == prefix || path.starts_with(&format!("{prefix}/")))
        })
        .cloned()
        .collect()
}

pub(crate) fn normalize_git_path(
    tool_name: &str,
    root: &Path,
    raw: &str,
) -> Result<String, String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: path must not be empty or contain NUL"
        ));
    }
    if raw.starts_with(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "{tool_name} refused: path `{raw}` contains Git pathspec metacharacters"
        ));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "{tool_name} refused: path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }

    let candidate = root.join(path);
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|error| {
            format!("{tool_name} refused: failed to resolve path `{raw}`: {error}")
        })?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("{tool_name} refused: path `{raw}` has no parent"))?;
        let parent = parent.canonicalize().map_err(|error| {
            format!("{tool_name} refused: failed to resolve parent for `{raw}`: {error}")
        })?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| format!("{tool_name} refused: path `{raw}` has no file name"))?;
        parent.join(file_name)
    };

    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: path `{raw}` resolves outside repository root"
        ));
    }

    Ok(raw.to_string())
}

pub(crate) fn git_status_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_status_paths_for_tool(tools::git_commit_exact::NAME, root)
}

pub(crate) fn git_status_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_paths_for_tool(tool_name, &output.stdout, "git status")
}

pub(crate) fn git_untracked_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_untracked_porcelain_paths_for_tool(tool_name, &output.stdout, "git status")
}

pub(crate) fn git_untracked_and_ignored_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    parse_untracked_and_ignored_porcelain_paths_for_tool(tool_name, &output.stdout, "git status")
}

pub(crate) fn git_cached_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_cached_paths_for_tool(tools::git_commit_exact::NAME, root)
}

pub(crate) fn git_cached_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(tool_name, root, &["diff", "--cached", "--name-only", "-z"])?;
    parse_nul_paths_for_tool(tool_name, &output.stdout, "git diff --cached")
}

pub(crate) fn parse_porcelain_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!("{tool_name} refused: unexpected {label} entry"));
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(format!(
                "{tool_name} refused: rename/copy entries require a future dedicated tool"
            ));
        }
        let path = std::str::from_utf8(&entry[3..])
            .map_err(|error| format!("{tool_name} refused: {label} path is not UTF-8: {error}"))?;
        paths.insert(path.to_string());
    }
    Ok(paths)
}

pub(crate) fn parse_untracked_porcelain_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!("{tool_name} refused: unexpected {label} entry"));
        }
        if entry[0] == b'?' && entry[1] == b'?' {
            let path = std::str::from_utf8(&entry[3..]).map_err(|error| {
                format!("{tool_name} refused: {label} path is not UTF-8: {error}")
            })?;
            paths.insert(path.to_string());
        }
    }
    Ok(paths)
}

pub(crate) fn parse_untracked_and_ignored_porcelain_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!("{tool_name} refused: unexpected {label} entry"));
        }
        if (entry[0] == b'?' && entry[1] == b'?') || (entry[0] == b'!' && entry[1] == b'!') {
            let path = std::str::from_utf8(&entry[3..]).map_err(|error| {
                format!("{tool_name} refused: {label} path is not UTF-8: {error}")
            })?;
            paths.insert(path.trim_end_matches('/').to_string());
        }
    }
    Ok(paths)
}

pub(crate) fn parse_nul_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            std::str::from_utf8(entry)
                .map(|path| path.to_string())
                .map_err(|error| format!("{tool_name} refused: {label} path is not UTF-8: {error}"))
        })
        .collect()
}

pub(crate) fn git_args(subcommand: &str, paths: &[String]) -> Vec<String> {
    std::iter::once(subcommand.to_string())
        .chain(std::iter::once("--".to_string()))
        .chain(paths.iter().cloned())
        .collect()
}

pub(crate) fn git_restore_args(paths: &[String]) -> Vec<String> {
    ["restore", "--staged", "--worktree", "--"]
        .into_iter()
        .map(ToString::to_string)
        .chain(paths.iter().cloned())
        .collect()
}

pub(crate) fn ensure_restore_paths_are_tracked(
    root: &Path,
    paths: &[String],
) -> Result<(), String> {
    let untracked = paths
        .iter()
        .filter_map(|path| {
            let args = ["ls-files", "--error-unmatch", "--", path.as_str()];
            match git_output_for_tool(tools::git_restore_exact::NAME, root, &args) {
                Ok(_) => None,
                Err(_) => Some(path.clone()),
            }
        })
        .collect::<BTreeSet<_>>();

    if !untracked.is_empty() {
        return Err(format!(
            "git_restore_exact refused: untracked paths cannot be restored from HEAD\n{}",
            format_set(&untracked)
        ));
    }
    Ok(())
}

pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output(root, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git_commit_exact refused: git output was not UTF-8: {error}"))
}

pub(crate) fn git_success(root: &Path, args: Vec<String>) -> Result<(), String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = git_output(root, &arg_refs)?;
    if !output.status.success() {
        return Err(format!(
            "git_commit_exact refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git_commit_exact refused: failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git_commit_exact refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

pub(crate) fn canonical_repo_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    repo_root
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve repo root: {error}"))
}

pub(crate) fn git_status_short(root: &Path) -> Result<String, String> {
    git_stdout_for_tool(
        "git_status_short",
        root,
        &["status", "--short", "--untracked-files=all"],
    )
}

pub(crate) fn git_stdout_for_tool(
    tool_name: &str,
    root: &Path,
    args: &[&str],
) -> Result<String, String> {
    let output = git_output_for_tool(tool_name, root, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{tool_name} refused: git output was not UTF-8: {error}"))
}

pub(crate) fn git_success_for_tool(
    tool_name: &str,
    root: &Path,
    args: Vec<String>,
) -> Result<(), String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let _ = git_output_for_tool(tool_name, root, &arg_refs)?;
    Ok(())
}

pub(crate) fn git_output_for_tool(
    tool_name: &str,
    root: &Path,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("{tool_name} refused: failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{tool_name} refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

pub(crate) fn git_status_code(root: &Path, args: &[&str]) -> Result<i32, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git operation refused: failed to run git: {error}"))?;
    Ok(output.status.code().unwrap_or(-1))
}

pub(crate) fn rev_count_for_tool(tool_name: &str, root: &Path, range: &str) -> Result<u64, String> {
    let output = git_stdout_for_tool(tool_name, root, &["rev-list", "--count", range])?;
    output.trim().parse::<u64>().map_err(|error| {
        format!(
            "{tool_name} refused: failed to parse revision count `{}`: {error}",
            output.trim()
        )
    })
}

pub(crate) fn ensure_remote_exists(
    tool_name: &str,
    root: &Path,
    remote: &str,
) -> Result<(), String> {
    let remotes = git_stdout_for_tool(tool_name, root, &["remote"])?;
    if remotes.lines().any(|line| line == remote) {
        Ok(())
    } else {
        Err(format!(
            "{tool_name} refused: remote `{remote}` is not configured"
        ))
    }
}

pub(crate) fn validate_git_remote(remote: &str) -> Result<String, String> {
    validate_git_ref_component("remote", remote)?;
    Ok(remote.to_string())
}

pub(crate) fn current_branch(tool_name: &str, root: &Path) -> Result<String, String> {
    let branch = git_stdout_for_tool(tool_name, root, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        Err(format!(
            "{tool_name} refused: repository is in detached HEAD state"
        ))
    } else {
        Ok(branch.to_string())
    }
}

pub(crate) fn local_branch_exists(root: &Path, branch: &str) -> Result<bool, String> {
    let branch_ref = format!("refs/heads/{branch}");
    let status = git_status_code(root, &["show-ref", "--verify", "--quiet", &branch_ref])?;
    match status {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(format!(
            "git workflow refused: failed to check local branch `{branch}` (exit code {code})"
        )),
    }
}

pub(crate) fn validate_git_branch(branch: &str) -> Result<String, String> {
    validate_git_ref_component("branch", branch)?;
    if branch.contains("..")
        || branch.contains("@{")
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    let status = Command::new("git")
        .args(["check-ref-format", "--branch", branch])
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git workflow refused: failed to validate branch: {error}"))?;
    if !status.status.success() {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    Ok(branch.to_string())
}

pub(crate) fn validate_git_ref_expression(value: &str) -> Result<String, String> {
    if value == "HEAD" {
        return Ok(value.to_string());
    }
    if (7..=40).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }

    validate_git_ref_component("ref", value)?;
    if value.contains("..")
        || value.contains('@')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }

    let status = Command::new("git")
        .args(["check-ref-format", "--allow-onelevel", value])
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git workflow refused: failed to validate ref: {error}"))?;
    if !status.status.success() {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }
    Ok(value.to_string())
}

pub(crate) fn validate_git_ref_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains('\0')
        || value.chars().any(char::is_whitespace)
        || value.starts_with('-')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('^')
        || value.contains('~')
        || value.contains('?')
        || value.contains('*')
        || value.contains('[')
    {
        return Err(format!("git workflow refused: invalid {label} `{value}`"));
    }
    Ok(())
}

pub(crate) fn validate_expected_head(expected_head: &str) -> Result<String, String> {
    if expected_head.len() < 7 || expected_head.len() > 40 {
        return Err(
            "git_push_exact refused: expected_head must be a 7 to 40 character commit hash"
                .to_string(),
        );
    }
    if !expected_head.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("git_push_exact refused: expected_head must be hexadecimal".to_string());
    }
    Ok(expected_head.to_ascii_lowercase())
}

pub(crate) fn remote_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

pub(crate) fn infer_fetch_branch(remote: &str, target_ref: &str) -> Result<String, String> {
    let refs_remotes_prefix = format!("refs/remotes/{remote}/");
    let remote_prefix = format!("{remote}/");
    let branch = if let Some(branch) = target_ref.strip_prefix(&refs_remotes_prefix) {
        branch
    } else if let Some(branch) = target_ref.strip_prefix(&remote_prefix) {
        branch
    } else {
        return Err(format!(
            "git_merge_readiness refused: fetch=true requires target_branch unless target_ref starts with `{remote}/` or `refs/remotes/{remote}/`"
        ));
    };
    validate_git_branch(branch)
}

pub(crate) fn resolve_commit(
    tool_name: &str,
    root: &Path,
    ref_name: &str,
) -> Result<String, String> {
    let commit_ref = format!("{ref_name}^{{commit}}");
    let commit = git_stdout_for_tool(tool_name, root, &["rev-parse", "--verify", &commit_ref])?;
    Ok(commit.trim().to_string())
}

pub(crate) fn changed_files_between(
    tool_name: &str,
    root: &Path,
    from_ref: &str,
    to_ref: &str,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["diff", "--name-only", "-z", from_ref, to_ref, "--"],
    )?;
    parse_nul_paths_for_tool(tool_name, &output.stdout, "git diff --name-only")
}

pub(crate) fn validate_required_file_path(
    tool_name: &str,
    root: &Path,
    raw: &str,
) -> Result<String, String> {
    validate_required_file_path_syntax(tool_name, raw)?;
    let path = Path::new(raw);
    let candidate = root.join(path);
    if !candidate.is_file() {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` is missing after branch preparation"
        ));
    }
    let resolved = candidate.canonicalize().map_err(|error| {
        format!("{tool_name} refused: failed to resolve required file `{raw}`: {error}")
    })?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` resolves outside repository root"
        ));
    }
    Ok(raw.to_string())
}

pub(crate) fn validate_required_files_in_ref(
    tool_name: &str,
    root: &Path,
    ref_name: &str,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| {
            validate_required_file_path_syntax(tool_name, path)?;
            let object = format!("{ref_name}:{path}");
            let object_type = git_stdout_for_tool(tool_name, root, &["cat-file", "-t", &object])
                .map_err(|_| {
                    format!(
                        "{tool_name} refused: required file `{path}` is missing from `{ref_name}`"
                    )
                })?;
            if object_type.trim() != "blob" {
                return Err(format!(
                    "{tool_name} refused: required path `{path}` in `{ref_name}` is not a file"
                ));
            }
            Ok(path.to_string())
        })
        .collect()
}

pub(crate) fn validate_required_file_path_syntax(tool_name: &str, raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: required file path must not be empty or contain NUL"
        ));
    }
    if raw.contains(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` contains Git pathspec metacharacters"
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: required file path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }
    Ok(())
}

pub(crate) fn empty_label(text: &str) -> &str {
    if text.trim().is_empty() {
        "(empty)"
    } else {
        text
    }
}

pub(crate) fn format_set(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths.iter().cloned().collect::<Vec<_>>().join("\n")
}
