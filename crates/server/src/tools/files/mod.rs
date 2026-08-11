pub mod artifact_delete_exact {
    pub const NAME: &str = "artifact_delete_exact";
    pub const CONFIRMATION: &str = "delete artifact exact";
}

pub mod artifact_write_base64 {
    pub const NAME: &str = "artifact_write_base64";
}

pub mod artifact_write_text {
    pub const NAME: &str = "artifact_write_text";
}

pub mod bulk_replace_exact {
    pub const NAME: &str = "bulk_replace_exact";
}

pub mod bulk_write_new_files_base64 {
    pub const NAME: &str = "bulk_write_new_files_base64";
}

pub mod create_directory {
    pub const NAME: &str = "create_directory";
}

pub mod file_info {
    pub const NAME: &str = "file_info";
}

pub mod read_write_receipts {
    pub const NAME: &str = "read_write_receipts";
}

pub mod list_directory {
    pub const NAME: &str = "list_directory";
}

pub mod diff_preview {
    pub const NAME: &str = "diff_preview";
}

pub mod read_file_bytes {
    pub const NAME: &str = "read_file_bytes";
}

pub mod read_range {
    pub const NAME: &str = "read_range";
}

pub mod replace_exact {
    pub const NAME: &str = "replace_exact";
}

pub mod set_file_executable {
    pub const NAME: &str = "set_file_executable";
}

pub mod status_guard {
    pub const NAME: &str = "status_guard";
}

pub mod write_existing_file_exact_hash {
    pub const NAME: &str = "write_existing_file_exact_hash";
}

pub mod write_new_file {
    pub const NAME: &str = "write_new_file";
}

pub mod write_new_file_base64 {
    pub const NAME: &str = "write_new_file_base64";
}

mod artifacts;
mod read;
mod write;

pub(crate) use artifacts::{
    call_artifact_delete_exact, call_artifact_write_base64, call_artifact_write_text,
};
pub(crate) use read::{
    call_diff_preview, call_file_info, call_list_directory, call_read_file_bytes, call_read_range,
    call_read_write_receipts, call_status_guard,
};
pub(crate) use write::{
    call_bulk_replace_exact, call_bulk_write_new_files_base64, call_create_directory,
    call_replace_exact, call_set_file_executable, call_write_existing_file_exact_hash,
    call_write_new_file, call_write_new_file_base64,
};

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use contextpatch_core::fs::create_directory::create_directory_in_root;
use contextpatch_core::fs::directory::list_directory_in_root;
use contextpatch_core::fs::guarded_file::{
    inspect_path_in_root, open_regular_file_in_root, validate_new_file_path_in_root,
    GuardedPathKind, GuardedRegularFile,
};
use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::{
    write_new_file_bytes_in_root, write_new_file_bytes_with_parents_in_root, write_new_file_in_root,
};
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::replace::exact::replace_exact_in_root_with_sha256;

use crate::tools;
use crate::tools::common::*;
use contextpatch_core::fs::rooted::canonical_label;
use contextpatch_core::git::RepositoryRoot;

/// Address a core refusal to the calling tool.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

/// The canonical path label for the boundaries that are still keyed by name.
///
/// Locks, receipts, and scratch identity are derived from a path and remain so. This resolves a configured
/// root once and returns an anchored selection's logical path untouched, so a selection is never reopened.
fn label(root: RepositoryRoot<'_>, tool_name: &str) -> Result<PathBuf, String> {
    canonical_label(root).map_err(|error| refused(tool_name, error))
}

/// The sidecar directory for one repository, created if absent.
///
/// Derived from repository identity rather than from the repository's path spelling, so it follows a rename
/// and is never inherited by a different repository that acquires the old name. The returned path is the
/// location of a directory outside the worktree; it is not authority over the repository.
pub(crate) fn artifact_root<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    tool_name: &str,
) -> Result<PathBuf, String> {
    contextpatch_core::fs::artifact::ensure_artifact_root(repository_root)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}
