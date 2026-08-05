pub mod deadline;
pub mod guarded_command;
pub mod guidance;
pub mod runner;
pub mod task_image;

/// Child-process ceiling used beneath the 120-second Git reply deadline.
///
/// A shorter inner limit leaves time to collect output and return a structured refusal instead of
/// relying solely on the outer worker deadline, which cannot cancel a blocked child process.
pub const GIT_SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
