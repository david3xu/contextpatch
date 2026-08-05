pub mod error;
pub mod fs;
pub mod git;
pub mod native_build;
pub mod native_device;
pub mod patch;
pub mod policy;
pub mod process;
/// Deterministic fingerprinting of an uncommitted worktree, so a build made from a dirty tree can
/// still be pinned to an exact source state. Shared verbatim with `build.rs`, which includes the
/// same file because a build script cannot depend on the crate it builds.
pub mod provenance;
pub mod replace;
pub mod setup;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build provenance, so a client can distinguish a missing capability from a stale binary.
///
/// `VERSION` cannot answer that question: it is the crate version and does not move between
/// rebuilds, so a client that finds a tool absent cannot tell whether the capability does not exist
/// or whether this binary predates it. The reasonable inference is the wrong one, and the cost is a
/// confident, silent false negative.
///
/// Populated by `build.rs`. Every field is always present, degrading to `unknown` rather than
/// vanishing, so a consumer never has to branch on absence.
pub const BUILD_GIT_SHA: &str = env!("CONTEXTPATCH_BUILD_GIT_SHA");
pub const BUILD_GIT_DIRTY: &str = env!("CONTEXTPATCH_BUILD_GIT_DIRTY");

/// Fingerprint of the uncommitted delta this binary was built from.
///
/// `BUILD_GIT_DIRTY` reports only that uncommitted changes existed. Because `BUILD_GIT_SHA` still
/// matches `HEAD` in that case, comparing the reported sha against the checked-out commit returns a
/// false all-clear, and the running binary cannot be matched to a known source state. This value
/// distinguishes two builds made from different dirty trees at the same commit.
///
/// Reads `clean` for a clean worktree and `unknown` when provenance could not be determined, so a
/// consumer never has to branch on absence. It is a fixed-width digest and never contains file
/// content. See [`provenance`] for the derivation.
pub const BUILD_GIT_DIRTY_FINGERPRINT: &str = env!("CONTEXTPATCH_BUILD_GIT_DIRTY_FINGERPRINT");
pub const BUILD_TIMESTAMP: &str = env!("CONTEXTPATCH_BUILD_TIMESTAMP");
pub const BUILD_PROFILE: &str = env!("CONTEXTPATCH_BUILD_PROFILE");
