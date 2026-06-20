//! Process-global export-pause refcount — the shared counter indexing pauses on
//! (story E11-S6). Port of the reference `SearchIndexCoordinator.ExportPauseCounter`
//! + `waitWhileExportActive` (search.md §"Coordinator queue + scheduling",
//! §"Port risks & gotchas" → "Export-pause coupling").
//!
//! ## What this is
//! In the macOS reference the export-pause counter is a **process-global refcount**
//! (`static var exportPause`) that any export window increments while it runs; the
//! indexing worker sleeps in **2 s loops** while `exportActive` so frame embedding
//! (candle/ort + wgpu) does not contend with the FFmpeg + wgpu export pipeline for
//! the CPU/GPU. The counter is refcounted because several exports can overlap across
//! windows — indexing resumes only when the **last** export ends.
//!
//! ## The cross-crate seam (FLAG — palmier-export wiring is a FOLLOW-UP)
//! Epic 6 export (`palmier-export`) is what should bump this counter around an export
//! run. This story deliberately does **NOT** edit `palmier-export` (no cross-crate
//! edit here). Instead it defines the shared counter + a clean public guard API in
//! `palmier-search`, with the **default state = not active** (so indexing works today),
//! and flags the wiring as a follow-up:
//!
//! > **FOLLOW-UP (palmier-export / Epic 6):** wrap each export run in
//! > [`ExportPauseGuard::begin`] (or call [`export_did_begin`] / [`export_did_end`]
//! > as a matched pair) so visual indexing pauses for the duration of the export.
//! > Until that lands, [`export_active`] is always `false` and indexing never pauses.
//!
//! ## Why a static, not a field
//! The pause is **process-global** (all projects' coordinators, all windows share it),
//! exactly like the reference `static`. A per-coordinator field could not see another
//! window's export. We use a `static AtomicUsize` so any thread (an export task on one
//! window, an index worker on another) reads/writes it lock-free.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::indexer::ExportYield;

/// The process-global in-flight-export refcount. `0` ⇒ no export active ⇒ indexing
/// runs. Each [`export_did_begin`] increments, each [`export_did_end`] decrements
/// (saturating at 0, mirroring the reference `end()`'s `max(0, count - 1)`).
static EXPORT_PAUSE: AtomicUsize = AtomicUsize::new(0);

/// The poll interval the indexer sleeps for while an export is active — the
/// reference's `Task.sleep(for: .seconds(2))` loop body.
pub const EXPORT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// `true` iff at least one export is in flight (`exportPause.isActive`). Indexing
/// pauses while this holds. Default (no export, no palmier-export wiring) ⇒ `false`.
pub fn export_active() -> bool {
    EXPORT_PAUSE.load(Ordering::Acquire) > 0
}

/// Begin an export (reference `exportDidBegin` → `exportPause.begin()`). Increments
/// the global refcount; indexing pauses until the matching [`export_did_end`].
///
/// **Prefer [`ExportPauseGuard::begin`]** for RAII safety; this raw pair exists for
/// callers (e.g. an FFI/command boundary) that cannot hold a guard across the export.
pub fn export_did_begin() {
    EXPORT_PAUSE.fetch_add(1, Ordering::AcqRel);
}

/// End an export (reference `exportDidEnd` → `exportPause.end()`). Decrements the
/// refcount, saturating at 0 so an unmatched `end` cannot underflow (reference
/// `count = max(0, count - 1)`).
pub fn export_did_end() {
    // Saturating decrement: never go below 0 even on an unbalanced call.
    let _ = EXPORT_PAUSE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |c| {
        Some(c.saturating_sub(1))
    });
}

/// RAII guard that holds the export-pause refcount for its lifetime.
///
/// [`ExportPauseGuard::begin`] increments the global counter; dropping the guard
/// decrements it. This is the API **palmier-export should consume** (Epic 6
/// follow-up): hold one for the duration of an export run and indexing pauses for
/// exactly that span, even on early return / panic unwinding.
#[derive(Debug)]
#[must_use = "dropping the guard immediately ends the export pause"]
pub struct ExportPauseGuard {
    _private: (),
}

impl ExportPauseGuard {
    /// Begin an export pause; the returned guard ends it on drop.
    pub fn begin() -> Self {
        export_did_begin();
        ExportPauseGuard { _private: () }
    }
}

impl Drop for ExportPauseGuard {
    fn drop(&mut self) {
        export_did_end();
    }
}

/// A real [`ExportYield`] over the process-global [`EXPORT_PAUSE`] refcount: blocks
/// the calling thread in [`EXPORT_POLL_INTERVAL`] (2 s) sleeps while [`export_active`]
/// — the reference's `while exportActive { Task.sleep(2s) }`. This is the impl the
/// E11-S6 coordinator hands to the [`crate::VisualIndexer`] (replacing E11-S4's
/// [`crate::NoExportYield`] stub), so visual indexing pauses during export.
///
/// The wait is **cooperatively cancellable**: the coordinator passes a stop flag
/// (the worker-generation staleness / cancel signal) so a paused worker that is
/// cancelled mid-export does not block forever.
#[derive(Clone)]
pub struct RefcountedExportYield {
    /// When this returns `true`, the wait loop bails out early (the worker was
    /// cancelled or superseded). Default impl never cancels.
    should_stop: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Default for RefcountedExportYield {
    fn default() -> Self {
        Self { should_stop: std::sync::Arc::new(|| false) }
    }
}

impl std::fmt::Debug for RefcountedExportYield {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefcountedExportYield").finish_non_exhaustive()
    }
}

impl RefcountedExportYield {
    /// A yield that waits on the global refcount and never cancels its own wait.
    pub fn new() -> Self {
        Self::default()
    }

    /// A yield whose 2 s wait loop also bails when `should_stop()` becomes true —
    /// the coordinator wires this to its cancel / worker-generation signal so a
    /// paused-during-export worker can still be cancelled.
    pub fn with_stop(should_stop: std::sync::Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self { should_stop }
    }
}

impl ExportYield for RefcountedExportYield {
    fn wait_while_export_active(&self) -> anyhow::Result<()> {
        // Reference: `while exportActive, !cancelled { sleep(2s) }`.
        while export_active() && !(self.should_stop)() {
            std::thread::sleep(EXPORT_POLL_INTERVAL);
        }
        Ok(())
    }
}

/// **Test-only:** force the process-global export refcount back to 0. Tests that run a
/// worker must call this first so a leaked count from another test (the counter is a
/// process-global static) can't wedge the worker in its pause loop. Always called under
/// the crate-wide [`crate::test_guard`].
#[cfg(test)]
pub(crate) fn reset_for_test() {
    EXPORT_PAUSE.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The export-pause counter is process-global and shared with the coordinator's
    // worker — serialize on the SAME crate-wide lock the coordinator tests use so a
    // bumped counter here can't make a coordinator worker (in a parallel test) pause.
    use crate::test_guard as guard;

    fn reset() {
        EXPORT_PAUSE.store(0, Ordering::Release);
    }

    #[test]
    fn default_state_is_not_active() {
        let _g = guard();
        reset();
        reset();
        assert!(!export_active(), "default (no wiring) must not pause indexing");
    }

    #[test]
    fn begin_end_pair_toggles_active() {
        let _g = guard();
        reset();
        reset();
        assert!(!export_active());
        export_did_begin();
        assert!(export_active());
        export_did_end();
        assert!(!export_active());
    }

    #[test]
    fn refcount_resumes_only_after_last_export_ends() {
        let _g = guard();
        reset();
        reset();
        export_did_begin();
        export_did_begin();
        assert!(export_active());
        export_did_end();
        assert!(export_active(), "still one export in flight");
        export_did_end();
        assert!(!export_active(), "last export ended ⇒ indexing resumes");
    }

    #[test]
    fn end_saturates_at_zero() {
        let _g = guard();
        reset();
        reset();
        // Unbalanced end must not underflow (reference max(0, count-1)).
        export_did_end();
        export_did_end();
        assert!(!export_active());
        export_did_begin();
        assert!(export_active());
    }

    #[test]
    fn guard_holds_pause_for_its_lifetime() {
        let _g = guard();
        reset();
        reset();
        assert!(!export_active());
        {
            let _pause = ExportPauseGuard::begin();
            assert!(export_active(), "guard begins the pause");
        }
        assert!(!export_active(), "drop ends the pause");
    }

    #[test]
    fn yield_returns_immediately_when_not_active() {
        let _g = guard();
        reset();
        reset();
        let y = RefcountedExportYield::new();
        // No export ⇒ no sleep ⇒ returns Ok promptly.
        y.wait_while_export_active().unwrap();
    }

    #[test]
    fn yield_bails_via_stop_flag_even_while_active() {
        let _g = guard();
        reset();
        reset();
        export_did_begin();
        // Stop is already true ⇒ the loop never sleeps and returns at once.
        let y = RefcountedExportYield::with_stop(std::sync::Arc::new(|| true));
        y.wait_while_export_active().unwrap();
        reset();
    }
}
