//! Checked-in snapshots of the advertised tool surface, as a refactor safety net.
//!
//! The tool registry migration moves every tool's schema, dispatch arm, deadline class, mutation-lock
//! membership, and advertised authority out of five separate files and into one table. Across 55
//! tools that is far more moving parts than review can cover: the realistic failure is not a crash
//! but a silently dropped annotation or a subtly reworded description, which no behavioural test
//! would notice because every test still passes.
//!
//! These fixtures make that failure loud. They record what the surface is *before* the refactor, so
//! every later step has to prove it produced the same thing rather than merely producing something
//! that works. A snapshot captured after a change would ratify whatever drift the change introduced,
//! which is why capturing them first is a correctness requirement and not a convenience.
//!
//! Regenerate deliberately, never reflexively:
//!
//! ```text
//! CONTEXTPATCH_UPDATE_FIXTURES=1 cargo test -p server
//! ```
//!
//! A regeneration that was not the point of the change is the bug this module exists to catch, so the
//! resulting diff belongs in review.

use std::path::PathBuf;

const UPDATE_ENV: &str = "CONTEXTPATCH_UPDATE_FIXTURES";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Compare one generated surface against its recorded snapshot.
///
/// On mismatch the panic names the first differing line rather than dumping both documents, because a
/// 2,000-line schema diff in test output is unreadable and the useful information is which tool moved.
pub(crate) fn assert_matches(name: &str, actual: &str) {
    let path = fixture_path(name);

    if std::env::var_os(UPDATE_ENV).is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("fixture directory must be creatable");
        }
        std::fs::write(&path, actual).expect("fixture must be writable");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing snapshot {}: {error}\nregenerate with {UPDATE_ENV}=1 cargo test -p server",
            path.display()
        )
    });

    if expected == actual {
        return;
    }

    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    let mut line = 0usize;
    loop {
        line += 1;
        match (expected_lines.next(), actual_lines.next()) {
            (None, None) => break,
            (want, got) if want != got => panic!(
                "{name} drifted from its recorded surface at line {line}\n  \
                 recorded: {}\n  current:  {}\n\
                 If this change is intended, regenerate with {UPDATE_ENV}=1 cargo test -p server \
                 and review the diff.",
                want.unwrap_or("<end of file>"),
                got.unwrap_or("<end of file>")
            ),
            _ => {}
        }
    }
}
