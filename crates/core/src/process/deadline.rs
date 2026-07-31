//! Bound how long a tool call can withhold a reply.
//!
//! Process execution is already time-bounded, but filesystem and Git work is not, and that asymmetry
//! produces the worst failure this server can have. When a read-modify-write stalls, the client does
//! not get a slow answer, it gets no answer: the request never completes, the transport is presumed
//! dead, and every later call to the same server fails too. Worse, the caller cannot tell whether the
//! mutation applied, so the only honest thing it can report is that the file is in an indeterminate
//! state, which is useless to a human and unrecoverable by the caller.
//!
//! A deadline converts that into an ordinary error. The distinction that makes this worth doing is
//! that a structured timeout is *actionable*: the caller learns which operation stalled, is told how
//! to establish the current state, and can carry on. An unanswered request teaches nothing.
//!
//! What a deadline does not do is cancel the work. Rust cannot safely interrupt a thread mid-syscall,
//! and pretending otherwise would risk a half-written file, which is precisely what the atomic write
//! path exists to prevent. So the work is moved to a worker thread and the *reply* is bounded: on
//! expiry the worker is abandoned to finish or block on its own, and the caller is told plainly that
//! the outcome is unknown and how to check. A leaked thread is a bounded, local cost; a wedged
//! transport takes down every subsequent call.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::error::ContextPatchError;

/// Read-only inspection. Generous enough for a large tree, short enough that a stall is visible
/// while the caller is still paying attention.
pub const READ_DEADLINE: Duration = Duration::from_secs(30);

/// Filesystem mutation, including the read-then-write pairs that have historically stalled.
pub const WRITE_DEADLINE: Duration = Duration::from_secs(60);

/// Git operations, which may fetch, and index work on a large repository is legitimately slow.
pub const GIT_DEADLINE: Duration = Duration::from_secs(120);

/// Prevent repeated expired calls from creating an unbounded number of detached workers.
pub const MAX_ACTIVE_WORKERS: usize = 16;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerPermit;

impl WorkerPermit {
    fn try_acquire() -> Result<Self, usize> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_WORKERS).then_some(active + 1)
            })
            .map(|_| Self)
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The outcome of a deadline-bounded call.
#[derive(Debug)]
pub enum Deadline<T> {
    /// The work finished inside the limit.
    Completed(T),
    /// The limit expired first. The work may still be running, so the caller must not assume the
    /// operation did or did not take effect.
    Expired {
        /// Which operation stalled, so the message names something the caller recognises.
        label: String,
        /// The limit that expired, so the caller can distinguish "slow" from "wedged".
        limit: Duration,
    },
    /// No worker was started because earlier calls are still active.
    Saturated { label: String, active: usize },
    /// The runtime could not create a worker, so the operation did not start.
    Unavailable { label: String, reason: String },
}

impl<T> Deadline<T> {
    /// Convert expiry into a refusal that says what is unknown and how to resolve it.
    ///
    /// The recovery instruction is part of the error rather than left to the caller to infer, because
    /// the caller's natural inference after a timeout is that nothing happened, and that inference is
    /// wrong exactly half the time.
    pub fn into_result(self) -> Result<T, ContextPatchError> {
        match self {
            Deadline::Completed(value) => Ok(value),
            Deadline::Expired { label, limit } => Err(ContextPatchError::invalid(format!(
                "`{label}` did not finish within {} seconds, so the server stopped waiting. The \
                 operation may or may not have taken effect: inspect `read_write_receipts` after a \
                 mutation, then establish the current state with `file_info` for a single path, \
                 `status_guard` for the worktree, or `fixture_manifest_verify` for a pinned set \
                 before retrying. The worker was not cancelled and nothing was rolled back.",
                limit.as_secs()
            ))),
            Deadline::Saturated { label, active } => Err(ContextPatchError::new(format!(
                "`{label}` was not started because {active} deadline workers are still active \
                 (maximum {MAX_ACTIVE_WORKERS}). Wait for earlier calls to finish; any previously \
                 expired mutation still has an unknown outcome."
            ))),
            Deadline::Unavailable { label, reason } => Err(ContextPatchError::new(format!(
                "`{label}` was not started because a deadline worker could not be created: {reason}"
            ))),
        }
    }
}

/// Run `work` on a worker thread and bound how long we wait for its reply.
///
/// `work` must be `Send + 'static` because it outlives this call on expiry. That bound is a feature:
/// it forces the work to own everything it touches, so an abandoned worker cannot hold a borrow into
/// a caller frame that has already returned.
pub fn with_deadline<T, F>(label: &str, limit: Duration, work: F) -> Deadline<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = match WorkerPermit::try_acquire() {
        Ok(permit) => permit,
        Err(active) => {
            return Deadline::Saturated {
                label: label.to_string(),
                active,
            }
        }
    };
    let (sender, receiver) = mpsc::channel();
    let owned_label = label.to_string();

    // The worker is deliberately not joined. Holding a handle would tempt a later change into joining
    // it, which reintroduces the unbounded wait this module exists to remove.
    let spawn_result = thread::Builder::new()
        .name(format!("contextpatch-{owned_label}"))
        .spawn(move || {
            let _permit = permit;
            // A send failure means the receiver timed out and went away. That is expected, not an
            // error, so the result is dropped rather than reported.
            let _ = sender.send(work());
        });
    if let Err(error) = spawn_result {
        return Deadline::Unavailable {
            label: owned_label,
            reason: error.to_string(),
        };
    }

    match receiver.recv_timeout(limit) {
        Ok(value) => Deadline::Completed(value),
        Err(mpsc::RecvTimeoutError::Timeout) => Deadline::Expired {
            label: owned_label,
            limit,
        },
        // The worker panicked or could not be spawned, so the channel closed without a value.
        // Reported as expiry with a zero limit so the caller still receives the recovery guidance
        // rather than a bare panic message.
        Err(mpsc::RecvTimeoutError::Disconnected) => Deadline::Expired {
            label: owned_label,
            limit: Duration::ZERO,
        },
    }
}

/// `with_deadline` for work that already returns a `Result`, flattening the two failure modes.
pub fn guard<T, F>(label: &str, limit: Duration, work: F) -> Result<T, ContextPatchError>
where
    F: FnOnce() -> Result<T, ContextPatchError> + Send + 'static,
    T: Send + 'static,
{
    with_deadline(label, limit, work).into_result()?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn serial_test() -> MutexGuard<'static, ()> {
        TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn wait_for_workers_to_finish() {
        for _ in 0..1_000 {
            if ACTIVE_WORKERS.load(Ordering::Acquire) == 0 {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("deadline worker did not finish after release");
    }

    #[test]
    fn completed_work_passes_its_value_through() {
        let _serial = serial_test();
        let outcome = with_deadline("fast", Duration::from_secs(5), || 7);
        match outcome {
            Deadline::Completed(value) => assert_eq!(value, 7),
            other => panic!("fast work should complete, got {other:?}"),
        }
        wait_for_workers_to_finish();
    }

    #[test]
    fn slow_work_expires_rather_than_blocking() {
        let _serial = serial_test();
        let (release, released) = mpsc::channel();
        let outcome = with_deadline("slow", Duration::from_millis(50), move || {
            released.recv().unwrap();
            "never observed"
        });
        match outcome {
            Deadline::Completed(_) => panic!("slow work should have expired"),
            Deadline::Expired { label, limit } => {
                assert_eq!(label, "slow");
                assert_eq!(limit, Duration::from_millis(50));
            }
            other => panic!("slow work should expire, got {other:?}"),
        }
        release.send(()).unwrap();
        wait_for_workers_to_finish();
    }

    #[test]
    fn expiry_names_the_operation_and_how_to_recover() {
        let _serial = serial_test();
        let (release, released) = mpsc::channel();
        let message = with_deadline(
            "write_existing_file_exact_hash",
            Duration::from_millis(20),
            move || {
                released.recv().unwrap();
            },
        )
        .into_result()
        .unwrap_err()
        .to_string();

        assert!(message.contains("write_existing_file_exact_hash"));
        assert!(
            message.contains("may or"),
            "must state the outcome is unknown: {message}"
        );
        assert!(
            message.contains("file_info"),
            "must name a recovery route: {message}"
        );
        release.send(()).unwrap();
        wait_for_workers_to_finish();
    }

    #[test]
    fn guard_flattens_an_inner_error() {
        let _serial = serial_test();
        let error = guard("failing", Duration::from_secs(5), || {
            Err::<(), _>(ContextPatchError::invalid("inner failure"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("inner failure"));
        wait_for_workers_to_finish();
    }

    #[test]
    fn a_panicking_worker_becomes_an_error_not_a_hang() {
        let _serial = serial_test();
        let outcome = with_deadline("panicking", Duration::from_secs(5), || {
            panic!("deliberate");
        });
        match outcome {
            Deadline::Completed(()) => panic!("a panicking worker cannot complete"),
            Deadline::Expired { label, .. } => assert_eq!(label, "panicking"),
            other => panic!("panicking work should disconnect, got {other:?}"),
        }
        wait_for_workers_to_finish();
    }

    #[test]
    fn saturation_refuses_without_starting_another_worker() {
        let _serial = serial_test();
        let permits = (0..MAX_ACTIVE_WORKERS)
            .map(|_| WorkerPermit::try_acquire().unwrap())
            .collect::<Vec<_>>();
        let outcome = with_deadline("one-too-many", Duration::from_secs(5), || 1);
        match outcome {
            Deadline::Saturated { label, active } => {
                assert_eq!(label, "one-too-many");
                assert_eq!(active, MAX_ACTIVE_WORKERS);
            }
            other => panic!("expected saturation, got {other:?}"),
        }
        drop(permits);
    }
}
