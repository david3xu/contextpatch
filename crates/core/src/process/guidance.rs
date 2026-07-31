//! Turn a refusal into a next step.
//!
//! A refusal that says only "not allowlisted" tells the caller that this attempt failed and nothing
//! about what would succeed. In practice the caller then infers the capability does not exist, which
//! is usually wrong: `gh` is refused here because `github_pr_run` owns it, `git rm` because
//! `delete_guarded` owns it, `git mv` because `move_tracked` owns it. The capability is present and
//! the caller walks away believing otherwise, which is the most expensive outcome this server can
//! produce, because it is silent and confident.
//!
//! So every refusal carries three things: what was wrong, what is permitted for that program if it is
//! allowlisted at all, and which typed tool owns the capability if one does. Discovery of last resort
//! is always named, so no refusal is a dead end.
//!
//! This module is deliberately data, not logic. Adding a program or a tool means adding a row, and
//! the tables are the single source of truth for both the refusal text and the tests.

/// Named so a caller can point at the map rather than guessing twice.
pub const DISCOVERY_TOOL: &str = "capability_manifest";

/// What a refusal should tell the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guidance {
    /// Present when the program is allowlisted but this particular invocation is not, so the caller
    /// can see the gap between what it asked for and what is permitted.
    pub permitted: Option<&'static str>,
    /// Typed tools that own this capability. Empty when nothing does.
    pub alternatives: Vec<&'static str>,
    /// Always populated, so a refusal never terminates without a route forward.
    pub discover: &'static str,
}

impl Guidance {
    /// The guidance as a single trailing clause, appended to the existing refusal text.
    ///
    /// Rendered as a suffix rather than replacing the message so callers and tests that match on
    /// "not allowlisted" keep working. A behavioural improvement should not require anyone to
    /// rewrite their error handling.
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        if let Some(permitted) = self.permitted {
            parts.push(format!("permitted for this program: {permitted}"));
        }
        if !self.alternatives.is_empty() {
            parts.push(format!("use instead: {}", self.alternatives.join(", ")));
        }
        parts.push(format!("discover capabilities with {}", self.discover));
        parts.join("; ")
    }
}

/// Exactly what each allowlisted program permits, phrased for a reader rather than a parser.
///
/// Kept beside the guard it describes. When the two drift, the refusal becomes a lie, which is worse
/// than saying nothing, so a program added to the guard without a row here should be treated as an
/// incomplete change.
pub fn permitted_summary(program: &str) -> Option<&'static str> {
    Some(match program {
        "git" => "status, diff, log, show, rev-parse, ls-tree (read-only inspection only)",
        "cargo" => "check, test, build, clippy",
        "bun" | "npm" | "pnpm" => "run, test",
        "python" | "python3" => "a repository-relative .py script path",
        "pytest" => "validation invocations",
        "harbor" => "run",
        "bash" => "references/check-base-image.sh, optionally with the argument `task`",
        "rg" => "any search invocation with at least one argument",
        _ => return None,
    })
}

/// Typed tools that own a capability the raw program cannot reach.
///
/// The mapping is intentionally generous: it is better to name a tool that turns out not to fit than
/// to leave the caller believing the capability is absent. Ordered most specific first.
pub fn tool_redirects(program: &str, args: &[String]) -> Vec<&'static str> {
    let subcommand = args.first().map(String::as_str);
    match (program, subcommand) {
        ("gh", Some("pr")) => vec!["github_pr_run"],
        ("gh", Some("run")) => vec!["github_pr_run (actions: run_list, run_view)"],
        ("gh", Some("repo")) => vec!["github_fork_prepare"],
        ("gh", _) => vec!["github_pr_run", "github_fork_prepare"],

        // Git mutations are owned by typed, dry-run-first tools rather than the raw subcommand.
        ("git", Some("rm")) => vec!["delete_guarded", "delete_untracked_exact"],
        ("git", Some("mv")) => vec!["move_tracked"],
        ("git", Some("add")) => vec!["git_stage_exact", "git_commit_scoped"],
        ("git", Some("commit")) => vec!["git_commit_scoped", "git_commit_exact"],
        ("git", Some("push")) => vec!["git_push_exact"],
        ("git", Some("fetch")) => vec!["git_remote_check"],
        ("git", Some("remote")) => vec!["git_remote_list"],
        ("git", Some("checkout" | "restore")) => vec!["git_restore_exact"],
        ("git", Some("switch" | "branch")) => vec!["git_branch_prepare"],
        ("git", Some("clean")) => vec!["delete_untracked_exact"],
        // `ls-files` is the reflex for listing tracked paths; `ls-tree` is the permitted equivalent.
        ("git", Some("ls-files")) => vec!["run_guarded_command with `git ls-tree`"],
        ("git", Some("apply")) => vec!["replace_exact", "write_existing_file_exact_hash"],

        ("docker" | "docker-compose" | "podman", _) => {
            vec!["validation_profile_run", "base_image_check_run"]
        }
        ("xcodebuild", _) => vec!["native_build_run"],
        ("gradle" | "./gradlew", _) => vec!["native_build_run"],
        ("xcrun" | "simctl" | "adb", _) => vec!["native_device_run"],
        ("npx" | "pod" | "cap", _) => vec!["setup_profile_run"],
        ("sha256sum" | "shasum" | "md5" | "openssl", _) => {
            vec!["file_info", "fixture_manifest_verify"]
        }
        ("mv" | "cp", _) => vec!["move_tracked", "write_new_file"],
        ("rm" | "rmdir", _) => vec!["delete_untracked_exact", "delete_guarded"],
        ("mkdir", _) => vec!["create_directory"],
        ("cat" | "head" | "tail" | "less" | "sed" | "awk", _) => {
            vec!["read_range", "replace_exact"]
        }
        ("find" | "ls" | "tree", _) => {
            vec!["list_directory", "run_guarded_command with `rg --files`"]
        }
        ("sh" | "bash" | "zsh", _) => vec!["validation_profile_run", "setup_profile_run"],
        _ => Vec::new(),
    }
}

/// Assemble guidance for one refused invocation.
pub fn guidance_for(program: &str, args: &[String]) -> Guidance {
    Guidance {
        permitted: permitted_summary(program),
        alternatives: tool_redirects(program, args),
        discover: DISCOVERY_TOOL,
    }
}

/// The refusal suffix for one invocation, ready to append.
pub fn refusal_suffix(program: &str, args: &[String]) -> String {
    guidance_for(program, args).render()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn names_the_owning_tool_for_a_refused_gh_invocation() {
        // The case that cost the most in practice: `gh` is refused, `github_pr_run` owns it, and the
        // old message mentioned neither.
        let guidance = guidance_for("gh", &args(&["pr", "view", "2"]));
        assert_eq!(guidance.alternatives, vec!["github_pr_run"]);
        assert!(guidance.render().contains("github_pr_run"));
    }

    #[test]
    fn routes_gh_run_to_the_workflow_actions() {
        let guidance = guidance_for("gh", &args(&["run", "view", "--log-failed"]));
        assert!(guidance.render().contains("run_view"));
    }

    #[test]
    fn routes_tracked_deletion_and_movement_to_typed_tools() {
        assert_eq!(
            tool_redirects("git", &args(&["rm", "old.py"])),
            vec!["delete_guarded", "delete_untracked_exact"]
        );
        assert_eq!(
            tool_redirects("git", &args(&["mv", "a", "b"])),
            vec!["move_tracked"]
        );
    }

    #[test]
    fn tells_an_allowlisted_program_what_it_may_actually_run() {
        // `git ls-files` is refused while `git` itself is allowlisted, so the caller needs the
        // permitted set, not just a denial.
        let guidance = guidance_for("git", &args(&["ls-files"]));
        let rendered = guidance.render();
        assert!(rendered.contains("ls-tree"), "{rendered}");
        assert!(
            rendered.contains("permitted for this program"),
            "{rendered}"
        );
    }

    #[test]
    fn always_names_a_discovery_route() {
        // Even a program nobody has thought about must leave the caller somewhere to go.
        let guidance = guidance_for("some-unknown-binary", &args(&["--help"]));
        assert!(guidance.permitted.is_none());
        assert!(guidance.alternatives.is_empty());
        assert!(guidance.render().contains(DISCOVERY_TOOL));
    }

    #[test]
    fn every_allowlisted_program_has_a_permitted_summary() {
        // Drift between the guard and this table turns a refusal into a false statement, so the
        // coupling is asserted rather than trusted.
        for program in [
            "git", "cargo", "bun", "npm", "pnpm", "python", "python3", "pytest", "harbor", "bash",
            "rg",
        ] {
            assert!(
                permitted_summary(program).is_some(),
                "{program} is allowlisted but has no permitted summary"
            );
        }
    }
}
