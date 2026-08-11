use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::json;

use super::*;

pub(crate) const MAX_ACTIVE_BACKGROUND_JOBS: usize = 2;

pub(super) static ACTIVE_BACKGROUND_JOBS: AtomicUsize = AtomicUsize::new(0);

pub(super) struct BackgroundJobPermit;

impl BackgroundJobPermit {
    fn try_acquire(tool_name: &str) -> Result<Self, String> {
        let mut active = ACTIVE_BACKGROUND_JOBS.load(Ordering::Acquire);
        loop {
            if active >= MAX_ACTIVE_BACKGROUND_JOBS {
                return Err(format!(
                    "{tool_name} refused: at most {MAX_ACTIVE_BACKGROUND_JOBS} background jobs may run at \
                     once, across Harbor, task-image, validation-profile, Compose-stack, and \
                     artifact-build work; poll existing log_ids before starting another job"
                ));
            }
            match ACTIVE_BACKGROUND_JOBS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(observed) => active = observed,
            }
        }
    }
}

impl Drop for BackgroundJobPermit {
    fn drop(&mut self) {
        ACTIVE_BACKGROUND_JOBS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct BackgroundJobOutcome {
    pub(super) status: &'static str,
    pub(super) exit_code: i32,
    pub(super) timed_out: bool,
    pub(super) log: String,
}

impl BackgroundJobOutcome {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            status: "failed",
            exit_code: -1,
            timed_out: false,
            log: json!({
                "status": "failed",
                "error": message.into()
            })
            .to_string(),
        }
    }
}

pub(super) fn start_background_job<F>(
    tool_name: &'static str,
    log_prefix: &str,
    initial_log: &str,
    worker: F,
) -> Result<String, String>
where
    F: FnOnce(&str) -> Result<BackgroundJobOutcome, String> + Send + 'static,
{
    let permit = BackgroundJobPermit::try_acquire(tool_name)?;
    let log_id =
        new_command_log_id(log_prefix).map_err(|error| format!("{tool_name} refused: {error}"))?;
    write_command_log_with_id(&log_id, initial_log)
        .and_then(|_| write_command_status(&log_id, "running", None, Some(false)))
        .map_err(|error| format!("{tool_name} refused: {error}"))?;

    let worker_log_id = log_id.clone();
    let spawn = thread::Builder::new()
        .name(format!("contextpatch-{log_id}"))
        .spawn(move || {
            let _permit = permit;
            let outcome = match catch_unwind(AssertUnwindSafe(|| worker(&worker_log_id))) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => BackgroundJobOutcome::failed(error),
                Err(payload) => BackgroundJobOutcome::failed(format!(
                    "{tool_name} worker panicked: {}",
                    panic_payload(payload)
                )),
            };

            if let Err(error) = write_command_log_with_id(&worker_log_id, &outcome.log) {
                eprintln!("contextpatch: failed to write {worker_log_id} result: {error}");
                let _ = write_command_status(&worker_log_id, "failed", None, None);
                return;
            }
            if let Err(error) = write_command_status(
                &worker_log_id,
                outcome.status,
                Some(outcome.exit_code),
                Some(outcome.timed_out),
            ) {
                eprintln!("contextpatch: failed to finalize {worker_log_id}: {error}");
            }
        });

    if let Err(error) = spawn {
        let failure = format!("{tool_name} failed to spawn background worker: {error}");
        let _ = write_command_log_with_id(&log_id, &failure);
        let _ = write_command_status(&log_id, "failed", None, None);
        return Err(format!("{tool_name} refused: {failure}"));
    }
    Ok(log_id)
}

pub(super) fn panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
