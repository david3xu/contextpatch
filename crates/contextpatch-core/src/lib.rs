pub mod error;
pub mod fs;
pub mod git;
pub mod patch;
pub mod policy;
pub mod replace;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
