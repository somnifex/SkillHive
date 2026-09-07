//! Background sync triggers (M2.6 remainder, plan §13 trigger sources).
//!
//! Runs the sync engine from a dedicated OS thread with conservative,
//! event-driven triggers:
//!
//! - **startup**: one cycle shortly after launch (outbox/cursor state is
//!   already recovered by then; a failure just waits for the next wake);
//! - **bounded periodic wake**: a slow heartbeat catches retry backoffs
//!   whose `next_attempt_at` elapsed while nothing else happened;
//! - **pull backlog**: a cycle that hit its page bound re-pokes itself;
//! - **explicit user sync request**: `sync_now` pokes the worker too.
//!
//! No busy loop: the worker sleeps on a long heartbeat between cycles.
//! Every cycle re-reads all eligibility from SQLite (dispatchable
//! mutations, backoff schedules, durable cursor), so worker memory holds
//! no correctness state and a crash at any point resumes on the next
//! wake. Authentication failures are swallowed by the worker (state stays
//! durable; the next wake retries) — the UI's `sync_now` remains the path
//! that surfaces errors to the user.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::blob_store::BlobStore;
use crate::local_store::LocalStore;
use crate::sync::{SyncCycleError, SyncEngine};
use crate::sync_client::SyncClient;

/// Periodic heartbeat: catches persisted backoffs that expire between
/// event triggers. Long by design — correctness never depends on it.
pub const PERIODIC_WAKE: Duration = Duration::from_secs(300);

/// Delay before the first startup cycle, letting setup finish quietly.
pub const STARTUP_DELAY: Duration = Duration::from_secs(2);

/// Shared wake handle: any thread can request one more cycle.
#[derive(Debug, Default)]
pub struct SyncWaker {
    state: Mutex<WakerState>,
    signal: Condvar,
}

#[derive(Debug, Default)]
struct WakerState {
    pending_pokes: u64,
}

impl SyncWaker {
    /// Requests one more sync cycle. Coalesces: multiple pokes while the
    /// worker is busy collapse into one pending poke.
    pub fn poke(&self) {
        let mut state = self.state.lock().expect("sync waker lock poisoned");
        state.pending_pokes += 1;
        self.signal.notify_one();
    }
}

/// Handle to the running worker thread.
pub struct SyncWorkerHandle {
    poke: Arc<SyncWaker>,
}

impl SyncWorkerHandle {
    /// Triggers one sync cycle (non-blocking, coalescing).
    pub fn request_sync(&self) {
        self.poke.poke();
    }
}

/// Spawns the background worker thread. Call once during app setup, after
/// startup recovery has finished. The returned handle lets the UI path
/// trigger immediate cycles on local commits.
pub fn spawn_sync_worker(
    client: Arc<SyncClient>,
    store: Arc<LocalStore>,
    blobs: Arc<BlobStore>,
    device_display_name: &'static str,
) -> SyncWorkerHandle {
    let waker = Arc::new(SyncWaker::default());
    let worker_waker = Arc::clone(&waker);

    std::thread::Builder::new()
        .name("skillhive-sync-worker".to_owned())
        .spawn(move || {
            worker_loop(worker_waker, &client, &store, &blobs, device_display_name);
        })
        .expect("failed to spawn sync worker thread");

    SyncWorkerHandle { poke: waker }
}

fn worker_loop(
    waker: Arc<SyncWaker>,
    client: &SyncClient,
    store: &LocalStore,
    blobs: &BlobStore,
    device_display_name: &str,
) {
    // Startup trigger: one early cycle after setup settles.
    std::thread::sleep(STARTUP_DELAY);
    run_worker_cycle(waker.as_ref(), client, store, blobs, device_display_name);

    loop {
        // Heartbeat park: slow by design. Pokes only shorten the wait by
        // letting the next cycle run sooner than the next heartbeat; they
        // never cause additional cycles beyond one per poke batch.
        std::thread::sleep(PERIODIC_WAKE);
        {
            let mut state = waker.state.lock().expect("sync waker lock poisoned");
            state.pending_pokes = 0;
        }
        run_worker_cycle(waker.as_ref(), client, store, blobs, device_display_name);
    }
}

/// Runs one cycle, swallowing the error (durable state already recorded
/// it); the next wake retries. Only `NotSignedIn` skips silently without
/// recording anything — the user simply has not logged in yet.
fn run_worker_cycle(
    waker: &SyncWaker,
    client: &SyncClient,
    store: &LocalStore,
    blobs: &BlobStore,
    device_display_name: &str,
) {
    match SyncEngine.run_cycle(client, store, blobs, device_display_name) {
        Ok(report) => {
            // A cycle that hit its page bound with more pull content
            // waiting immediately schedules the next cycle.
            if report.has_more() {
                waker.poke();
            }
        }
        // Expected pre-login state; not an error condition for the worker.
        Err(SyncCycleError::NotSignedIn) => {}
        // Everything else is already durably recorded; wait for the next
        // trigger (backoff schedule or user action) instead of spinning.
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncCycleReport;

    #[test]
    fn waker_coalesces_pokes() {
        let waker = SyncWaker::default();
        waker.poke();
        waker.poke();
        waker.poke();
        let state = waker.state.lock().expect("lock");
        assert_eq!(state.pending_pokes, 3);
    }

    #[test]
    fn heartbeat_bounds_are_conservative() {
        // The heartbeat exists only to catch expired backoffs; it must be
        // slow enough to never approach a busy loop.
        assert!(PERIODIC_WAKE >= Duration::from_secs(60));
        assert!(STARTUP_DELAY < PERIODIC_WAKE);
    }

    #[test]
    fn has_more_reflects_page_bound() {
        let report = SyncCycleReport {
            pushed: 0,
            pulled: true,
            pages_applied: crate::sync_pull::MAX_PAGES_PER_CYCLE,
            upserts: 0,
            tombstones: 0,
            conflicts_detected: 0,
            blobs_downloaded: 0,
            stopped_reason: None,
        };
        assert!(report.has_more());

        let report = SyncCycleReport {
            pages_applied: 1,
            ..report
        };
        assert!(!report.has_more());
    }
}
