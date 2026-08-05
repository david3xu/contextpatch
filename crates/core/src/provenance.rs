// Deterministic build provenance for a dirty worktree.
//
// `git_sha` alone cannot pin a build. When the worktree is dirty the sha still matches `HEAD`, so
// comparing the reported sha against the checked-out commit returns a false all-clear and the
// running binary cannot be matched to a known source state. This module produces a fingerprint of
// the uncommitted delta so two builds from different dirty trees at the same commit are
// distinguishable.
//
// `build.rs` includes this file directly with `include!`, because a build script cannot depend on
// the crate it builds. Everything here therefore stays free of crate-internal types and imports and
// confines itself to string transformation, so the logic that produces the shipped stamp is exactly
// the logic the tests exercise. Inner doc comments are deliberately avoided for the same reason:
// `include!` splices this file into the middle of the build script, where they are not legal.
//
// No file content ever enters this module. Callers supply a per-path content digest, so contents
// cannot reach the emitted stamp, a build log, or a failure message.

/// Reported in place of a fingerprint when the worktree carries no changes, so the field is always
/// present and a consumer never has to branch on absence.
pub const CLEAN_WORKTREE_FINGERPRINT: &str = "clean";

/// Recorded for a dirty path with no readable content, such as a deletion. Distinct from the digest
/// of an empty file, so removing a file and truncating it do not collapse to the same stamp.
pub const ABSENT_CONTENT_DIGEST: &str = "absent";

/// Recorded as the status of the previous path of a rename or copy, so both endpoints contribute.
pub const RENAME_ORIGIN_STATUS: &str = "->";

/// Porcelain v1 prefixes two status characters and one separating space before each path.
pub const PORCELAIN_STATUS_FIELD_WIDTH: usize = 3;

/// Width of the rendered fingerprint. Fixed, so the stamp cannot grow with the size of the dirty set
/// and cannot leak how much is uncommitted.
pub const FINGERPRINT_HEX_WIDTH: usize = 16;

/// Field separator inside the canonical manifest. A tab cannot appear in a porcelain status code.
const MANIFEST_FIELD_SEPARATOR: char = '\t';

/// Record separator inside the canonical manifest.
const MANIFEST_RECORD_SEPARATOR: char = '\n';

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One uncommitted path: its porcelain status, its repository-relative path, and a digest of its
/// current content.
///
/// Field order is deliberate: the derived ordering sorts by path first, which is what makes the
/// canonical manifest independent of the order Git happened to report entries in.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DirtyEntry {
    pub path: String,
    pub status: String,
    pub content_digest: String,
}

/// Parse `git status --porcelain=v1 -z --untracked-files=all` records into status and path pairs.
///
/// Records arrive already split on NUL. A rename or copy is reported as the new path followed by a
/// separate record holding the previous path; that record carries no status prefix, so it is
/// consumed here rather than left to the next iteration, which would otherwise read the first three
/// characters of the previous path as a status code.
pub fn parse_porcelain_records(records: &[String]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut index = 0;

    while index < records.len() {
        let record = &records[index];
        index += 1;

        if record.len() < PORCELAIN_STATUS_FIELD_WIDTH {
            continue;
        }

        let (status, path) = record.split_at(PORCELAIN_STATUS_FIELD_WIDTH);
        // Only the trailing separator is padding. A leading space is meaningful: it distinguishes a
        // change staged in the index from one present only in the worktree.
        let status = status.trim_end().to_string();
        let renamed = status.starts_with('R') || status.starts_with('C');
        entries.push((status, path.to_string()));

        if renamed {
            if let Some(origin) = records.get(index) {
                entries.push((RENAME_ORIGIN_STATUS.to_string(), origin.clone()));
                index += 1;
            }
        }
    }

    entries
}

/// Build the canonical text whose digest becomes the fingerprint.
///
/// Entries are sorted so the result does not depend on Git's reporting order, and each record
/// carries only a path, a status, and a content digest.
pub fn canonical_manifest(entries: &[DirtyEntry]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort();

    let mut manifest = String::new();
    for entry in &sorted {
        manifest.push_str(&entry.path);
        manifest.push(MANIFEST_FIELD_SEPARATOR);
        manifest.push_str(&entry.status);
        manifest.push(MANIFEST_FIELD_SEPARATOR);
        manifest.push_str(&entry.content_digest);
        manifest.push(MANIFEST_RECORD_SEPARATOR);
    }
    manifest
}

/// Combine the canonical manifest into a single value with FNV-1a.
///
/// A build script may not depend on an external crate here, and content sensitivity already comes
/// from the per-path digests Git computed, so this step only has to combine them deterministically.
/// A collision would cost a diagnostic distinction rather than a safety property.
fn combine(payload: &str) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Fingerprint the uncommitted worktree state.
///
/// Returns [`CLEAN_WORKTREE_FINGERPRINT`] when nothing is uncommitted, so a clean build stays
/// distinguishable from a build whose provenance could not be determined at all.
pub fn dirty_tree_fingerprint(entries: &[DirtyEntry]) -> String {
    if entries.is_empty() {
        return CLEAN_WORKTREE_FINGERPRINT.to_string();
    }

    let combined = combine(&canonical_manifest(entries));
    format!("{combined:0width$x}", width = FINGERPRINT_HEX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, status: &str, content_digest: &str) -> DirtyEntry {
        DirtyEntry {
            path: path.to_string(),
            status: status.to_string(),
            content_digest: content_digest.to_string(),
        }
    }

    #[test]
    fn clean_worktree_reports_the_clean_marker() {
        assert_eq!(dirty_tree_fingerprint(&[]), CLEAN_WORKTREE_FINGERPRINT);
    }

    #[test]
    fn identical_dirty_trees_report_the_same_stamp() {
        let first = [entry("a.rs", "M", "aaa"), entry("b.rs", "??", "bbb")];
        let second = [entry("a.rs", "M", "aaa"), entry("b.rs", "??", "bbb")];

        assert_eq!(
            dirty_tree_fingerprint(&first),
            dirty_tree_fingerprint(&second)
        );
    }

    #[test]
    fn reporting_order_does_not_change_the_stamp() {
        let ascending = [entry("a.rs", "M", "aaa"), entry("b.rs", "??", "bbb")];
        let descending = [entry("b.rs", "??", "bbb"), entry("a.rs", "M", "aaa")];

        assert_eq!(
            dirty_tree_fingerprint(&ascending),
            dirty_tree_fingerprint(&descending)
        );
    }

    #[test]
    fn differing_content_at_the_same_head_changes_the_stamp() {
        // Same commit, same dirty path set, same statuses. Only the working-tree content differs,
        // which is precisely the case a dirty boolean cannot express.
        let first = [entry("a.rs", "M", "aaa")];
        let second = [entry("a.rs", "M", "zzz")];

        assert_ne!(
            dirty_tree_fingerprint(&first),
            dirty_tree_fingerprint(&second)
        );
    }

    #[test]
    fn differing_dirty_path_sets_change_the_stamp() {
        let fewer = [entry("a.rs", "M", "aaa")];
        let more = [entry("a.rs", "M", "aaa"), entry("b.rs", "??", "bbb")];

        assert_ne!(dirty_tree_fingerprint(&fewer), dirty_tree_fingerprint(&more));
    }

    #[test]
    fn staging_state_changes_the_stamp() {
        let staged = [entry("a.rs", "M", "aaa")];
        let unstaged = [entry("a.rs", " M", "aaa")];

        assert_ne!(
            dirty_tree_fingerprint(&staged),
            dirty_tree_fingerprint(&unstaged)
        );
    }

    #[test]
    fn a_deletion_is_distinguishable_from_an_empty_file() {
        let empty_blob = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let deleted = [entry("a.rs", " D", ABSENT_CONTENT_DIGEST)];
        let truncated = [entry("a.rs", " D", empty_blob)];

        assert_ne!(
            dirty_tree_fingerprint(&deleted),
            dirty_tree_fingerprint(&truncated)
        );
    }

    #[test]
    fn the_stamp_is_fixed_width_hex_regardless_of_dirty_set_size() {
        let small = vec![entry("a.rs", "M", "aaa")];
        let large: Vec<DirtyEntry> = (0..500)
            .map(|index| entry(&format!("file{index}.rs"), "M", "aaa"))
            .collect();

        for entries in [small.as_slice(), large.as_slice()] {
            let stamp = dirty_tree_fingerprint(entries);
            assert_eq!(stamp.len(), FINGERPRINT_HEX_WIDTH);
            assert!(stamp.chars().all(|character| character.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn the_manifest_carries_only_the_supplied_fields() {
        // The manifest is the only input to the digest, so anything absent from it cannot reach the
        // stamp. Content digests are supplied by the caller and file bytes are never read here.
        let manifest = canonical_manifest(&[entry("a.rs", "M", "aaa")]);

        assert_eq!(manifest, "a.rs\tM\taaa\n");
    }

    #[test]
    fn porcelain_records_parse_into_status_and_path() {
        let records = [
            "?? docs/new.md".to_string(),
            " M crates/core/src/lib.rs".to_string(),
        ];

        assert_eq!(
            parse_porcelain_records(&records),
            vec![
                ("??".to_string(), "docs/new.md".to_string()),
                (" M".to_string(), "crates/core/src/lib.rs".to_string()),
            ]
        );
    }

    #[test]
    fn a_rename_contributes_both_endpoints() {
        let records = ["R  new.rs".to_string(), "old.rs".to_string()];

        assert_eq!(
            parse_porcelain_records(&records),
            vec![
                ("R".to_string(), "new.rs".to_string()),
                (RENAME_ORIGIN_STATUS.to_string(), "old.rs".to_string()),
            ]
        );
    }

    #[test]
    fn a_rename_origin_is_not_parsed_as_a_status_code() {
        // Without pairing, the second record would yield status `cra` and a truncated path.
        let records = [
            "R  crates/core/src/new.rs".to_string(),
            "crates/core/src/old.rs".to_string(),
        ];
        let parsed = parse_porcelain_records(&records);

        assert_eq!(parsed[1].0, RENAME_ORIGIN_STATUS);
        assert_eq!(parsed[1].1, "crates/core/src/old.rs");
    }

    #[test]
    fn records_shorter_than_a_status_field_are_ignored() {
        let records = [String::new(), "?".to_string(), "??".to_string()];

        assert!(parse_porcelain_records(&records).is_empty());
    }
}
