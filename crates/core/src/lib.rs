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
