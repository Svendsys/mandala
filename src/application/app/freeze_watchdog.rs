// SPDX-License-Identifier: MPL-2.0

//! Native main-loop watchdog. A background thread polls a liveness
//! atomic that the main loop pings every frame; if the main thread
//! has been silent longer than [`FREEZE_THRESHOLD`], the watchdog
//! prints a banner and aborts so the OS can produce a core dump.
//!
//! The class of freeze this catches is native-heavier (RwLock
//! re-entry, GPU stalls, runaway loops); browsers surface their own
//! page-unresponsive dialog, so WASM has no equivalent yet. See
//! `CLAUDE.md` "Dual-target status".

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Abort if the main thread is silent longer than this.
/// Conservative enough to never fire on legitimate load
/// (60 fps ≈ 16.6 ms/frame); tune down cautiously — false
/// positives here kill the process.
const FREEZE_THRESHOLD: Duration = Duration::from_secs(10);

/// Watchdog thread wake interval. Detection latency is
/// `FREEZE_THRESHOLD + WATCHDOG_POLL` in the worst case.
const WATCHDOG_POLL: Duration = Duration::from_secs(1);

/// Handle to the freeze watchdog. Dropping the handle does *not*
/// stop the watchdog thread — the thread is detached and lives
/// for the process lifetime. This is intentional: the watchdog's
/// whole job is to catch a permanently-stuck main thread, so a
/// mechanism that could be dropped-in-error and silently disable
/// the safety net would defeat the point.
pub struct FreezeWatchdog {
    last_activity_ms: Arc<AtomicU64>,
    /// Monotonic clock shared with the background thread so both
    /// sides measure elapsed time against the same origin. Avoids
    /// reaching into [`crate::application::app::now_ms`], which is
    /// private to its module and returns `f64`.
    epoch: Instant,
}

impl FreezeWatchdog {
    /// Spawn the watchdog thread and return a handle the main loop
    /// can ping. Safe to call exactly once per process; the thread
    /// is detached and cannot be stopped. Callers should keep the
    /// returned handle alive (store it on `InitState` or similar).
    pub fn spawn() -> Self {
        let epoch = Instant::now();
        let last_activity_ms = Arc::new(AtomicU64::new(0));
        let bg_atomic = Arc::clone(&last_activity_ms);
        let bg_epoch = epoch;
        thread::Builder::new()
            .name("mandala-freeze-watchdog".into())
            .spawn(move || watchdog_loop(bg_atomic, bg_epoch))
            .expect("failed to spawn freeze watchdog thread");
        Self {
            last_activity_ms,
            epoch,
        }
    }

    /// Ping the watchdog. Call once per frame at the top of
    /// `AboutToWait` (or anywhere else the main loop guarantees
    /// forward progress). Writing is `Relaxed` — we don't need
    /// ordering guarantees because the watchdog only ever reads
    /// a monotonically increasing elapsed-ms value.
    #[inline]
    pub fn tick(&self) {
        let elapsed_ms = self.epoch.elapsed().as_millis() as u64;
        self.last_activity_ms.store(elapsed_ms, Ordering::Relaxed);
    }
}

fn watchdog_loop(last_activity_ms: Arc<AtomicU64>, epoch: Instant) {
    // Wait for the first tick before enforcing anything. Until the
    // main loop has pinged once, the atomic is 0 — treating that
    // as "last activity was at time 0" would fire the watchdog
    // immediately on startup if the first frame takes longer than
    // `FREEZE_THRESHOLD` to land (e.g., a large map load).
    loop {
        thread::sleep(WATCHDOG_POLL);
        let last = last_activity_ms.load(Ordering::Relaxed);
        if last == 0 {
            continue;
        }
        let now = epoch.elapsed().as_millis() as u64;
        let silence_ms = now.saturating_sub(last);
        if silence_ms > FREEZE_THRESHOLD.as_millis() as u64 {
            eprintln!();
            eprintln!("!!! MANDALA FREEZE WATCHDOG !!!");
            eprintln!(
                "main thread has not pinged for {} ms (threshold {} ms).",
                silence_ms,
                FREEZE_THRESHOLD.as_millis()
            );
            eprintln!(
                "this almost certainly means a deadlock, an infinite loop, or a \
                 blocking GPU/compositor call on the main thread."
            );
            eprintln!(
                "aborting the process so the OS can produce a core / crash report. \
                 re-run under a debugger or with `RUST_BACKTRACE=1` and \
                 `ulimit -c unlimited` to capture the stuck stack."
            );
            eprintln!();
            std::process::abort();
        }
    }
}
