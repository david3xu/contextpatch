pub mod error;
pub mod fs;
pub mod git;
pub mod native_build;
pub mod native_device;
pub mod patch;
pub mod policy;
pub mod process;
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
pub const BUILD_TIMESTAMP: &str = env!("CONTEXTPATCH_BUILD_TIMESTAMP");
pub const BUILD_PROFILE: &str = env!("CONTEXTPATCH_BUILD_PROFILE");
