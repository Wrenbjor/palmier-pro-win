//! Concurrency gates + in-flight dedup for the media visual cache (E4-S2).
//!
//! Port of the Swift `MediaVisualCache` concurrency machinery:
//!
//! * `AsyncSemaphore(value: 2)` for waveform extraction  → [`CacheKind::Waveform`]
//! * `AsyncSemaphore(value: 4)` for image thumbnails      → [`CacheKind::ImageThumbnail`]
//! * **no semaphore** for video thumbnails (ungated)      → [`CacheKind::VideoThumbnail`]
//! * a per-kind in-flight `Set<String>` that dedupes duplicate requests for the
//!   same cache key so two concurrent callers share one computation.
//!
//! `AsyncSemaphore` → [`tokio::sync::Semaphore`]; the in-flight set →
//! `Mutex<HashMap<String, Weak<Shared<…>>>>` so concurrent same-key requests
//! await **one** [`tokio::sync::OnceCell`]-style shared future instead of each
//! recomputing (ruling #16).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{Notify, Semaphore};

/// The three cache kinds, each with its own gate + in-flight set, matching the
/// reference's separate waveform / image-thumb / video-thumb tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheKind {
    /// Audio waveform extraction. Gated at **2** concurrent (`utility` priority).
    Waveform,
    /// Single still image thumbnail. Gated at **4** concurrent (`utility`).
    ImageThumbnail,
    /// Video sprite-sheet strip. **Ungated** (`userInitiated` priority).
    VideoThumbnail,
}

impl CacheKind {
    /// Concurrency cap for this kind. `None` == ungated (video thumbnails).
    pub fn concurrency_limit(self) -> Option<usize> {
        match self {
            CacheKind::Waveform => Some(2),
            CacheKind::ImageThumbnail => Some(4),
            CacheKind::VideoThumbnail => None,
        }
    }
}

/// A single in-flight computation for one `(kind, key)`. The first caller (the
/// *leader*) runs the job and stores the result in `value`, then wakes every
/// follower via `done`. Followers never touch the compute closure — they just
/// await `done` and clone the published `value`. This keeps the "exactly one
/// computation" guarantee without depending on who polls a cell first.
struct InFlight<T> {
    /// Set once by the leader; `None` until the job completes.
    value: Mutex<Option<T>>,
    /// Notifies waiting followers when `value` is populated. `notify_waiters`
    /// only wakes tasks already waiting, so followers register their wait
    /// (`notified()`) *before* re-checking `value` to avoid a lost wakeup.
    done: Notify,
}

type InFlightMap<T> = Mutex<HashMap<String, Weak<InFlight<T>>>>;

/// Per-kind limiter: an optional semaphore (the concurrency gate) plus the
/// in-flight dedup map. One [`Gate`] instance is held per [`CacheKind`] in
/// [`CacheGates`].
struct Gate<T> {
    semaphore: Option<Arc<Semaphore>>,
    in_flight: InFlightMap<T>,
}

impl<T: Clone + Send + Sync + 'static> Gate<T> {
    fn new(limit: Option<usize>) -> Self {
        Gate {
            semaphore: limit.map(|n| Arc::new(Semaphore::new(n))),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// Look up an existing in-flight entry for `key`, or insert a fresh one.
    /// Returns `(shared, is_leader)` — `is_leader` is true for the caller that
    /// created the entry and must therefore run the job.
    fn get_or_insert(&self, key: &str) -> (Arc<InFlight<T>>, bool) {
        let mut map = self.in_flight.lock().expect("in-flight mutex poisoned");
        if let Some(existing) = map.get(key).and_then(Weak::upgrade) {
            return (existing, false);
        }
        let shared = Arc::new(InFlight {
            value: Mutex::new(None),
            done: Notify::new(),
        });
        map.insert(key.to_string(), Arc::downgrade(&shared));
        (shared, true)
    }

    /// Drop the in-flight entry for `key` once the leader is done (best-effort;
    /// only removes it if it still points at a dead/leader weak ref).
    fn remove(&self, key: &str) {
        let mut map = self.in_flight.lock().expect("in-flight mutex poisoned");
        // Only remove if nobody else has re-registered a *live* newer entry.
        if let Some(weak) = map.get(key) {
            if weak.upgrade().is_none() || Weak::strong_count(weak) <= 1 {
                map.remove(key);
            }
        }
    }

    /// Number of currently-registered in-flight keys (best-effort, for tests).
    fn in_flight_len(&self) -> usize {
        let map = self.in_flight.lock().expect("in-flight mutex poisoned");
        map.values().filter(|w| w.upgrade().is_some()).count()
    }
}

/// The full set of gates the visual cache uses — one per [`CacheKind`].
///
/// Generic over the produced value `T` (e.g. `Vec<f32>` for waveforms, a thumb
/// handle for thumbnails). Cheaply [`Clone`]able via the inner `Arc` so it can
/// be shared across tasks.
#[derive(Clone)]
pub struct CacheGates<T> {
    inner: Arc<GatesInner<T>>,
}

struct GatesInner<T> {
    waveform: Gate<T>,
    image_thumbnail: Gate<T>,
    video_thumbnail: Gate<T>,
}

impl<T: Clone + Send + Sync + 'static> Default for CacheGates<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync + 'static> CacheGates<T> {
    /// Construct gates with the reference caps (waveform 2, image-thumb 4,
    /// video-thumb ungated).
    pub fn new() -> Self {
        CacheGates {
            inner: Arc::new(GatesInner {
                waveform: Gate::new(CacheKind::Waveform.concurrency_limit()),
                image_thumbnail: Gate::new(CacheKind::ImageThumbnail.concurrency_limit()),
                video_thumbnail: Gate::new(CacheKind::VideoThumbnail.concurrency_limit()),
            }),
        }
    }

    fn gate(&self, kind: CacheKind) -> &Gate<T> {
        match kind {
            CacheKind::Waveform => &self.inner.waveform,
            CacheKind::ImageThumbnail => &self.inner.image_thumbnail,
            CacheKind::VideoThumbnail => &self.inner.video_thumbnail,
        }
    }

    /// Number of in-flight jobs for `kind` (test/diagnostic helper).
    pub fn in_flight_len(&self, kind: CacheKind) -> usize {
        self.gate(kind).in_flight_len()
    }

    /// Run `compute` for `(kind, key)` under the kind's concurrency gate, with
    /// in-flight dedup: if another caller is already computing the same key,
    /// this awaits that one computation instead of starting a second.
    ///
    /// `compute` is a closure returning a future, so it is only ever invoked by
    /// the *leader* (the first caller for a key). Followers await the leader's
    /// result via a shared [`OnceCell`]. The semaphore permit is acquired by the
    /// leader only — gating *distinct* keys, while duplicates collapse for free.
    pub async fn run<F, Fut>(&self, kind: CacheKind, key: &str, compute: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let gate = self.gate(kind);
        let (shared, is_leader) = gate.get_or_insert(key);

        if !is_leader {
            // Follower: wait for the leader to publish, then clone its value.
            // Register the wait BEFORE checking `value` so a notify that fires
            // between the check and the await is not lost.
            loop {
                let notified = shared.done.notified();
                if let Some(v) = shared.value.lock().expect("value mutex").as_ref() {
                    return v.clone();
                }
                notified.await;
            }
        }

        // Leader: acquire the gate permit (if any), run the compute once, then
        // publish the result and wake every follower.
        let _permit = match &gate.semaphore {
            Some(sem) => Some(
                sem.clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore not closed"),
            ),
            None => None,
        };

        let value = compute().await;
        {
            let mut slot = shared.value.lock().expect("value mutex");
            *slot = Some(value.clone());
        }
        shared.done.notify_waiters();

        // Drop the in-flight registration so a later request (e.g. after a
        // source edit changes the key) recomputes instead of reusing this.
        gate.remove(key);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Barrier;

    #[test]
    fn concurrency_limits_match_reference() {
        assert_eq!(CacheKind::Waveform.concurrency_limit(), Some(2));
        assert_eq!(CacheKind::ImageThumbnail.concurrency_limit(), Some(4));
        assert_eq!(CacheKind::VideoThumbnail.concurrency_limit(), None);
    }

    /// Drive N distinct-key jobs through a gate and assert the peak number
    /// running at once never exceeds the kind's cap.
    async fn assert_peak_under_cap(kind: CacheKind, cap: usize) {
        let gates: CacheGates<u32> = CacheGates::new();
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let total = cap * 4;
        let mut handles = Vec::new();
        for i in 0..total {
            let gates = gates.clone();
            let current = current.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let key = format!("key-{i}"); // distinct keys → all gated, no dedup
                gates
                    .run(kind, &key, || async {
                        let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        // Hold the permit long enough that all tasks overlap.
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        current.fetch_sub(1, Ordering::SeqCst);
                        i as u32
                    })
                    .await
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert!(
            peak.load(Ordering::SeqCst) <= cap,
            "{kind:?}: peak concurrency {} exceeded cap {cap}",
            peak.load(Ordering::SeqCst)
        );
        assert!(
            peak.load(Ordering::SeqCst) >= 1,
            "{kind:?}: expected at least one concurrent job"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn waveform_gate_caps_at_two() {
        assert_peak_under_cap(CacheKind::Waveform, 2).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn image_thumbnail_gate_caps_at_four() {
        assert_peak_under_cap(CacheKind::ImageThumbnail, 4).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn video_thumbnail_is_ungated() {
        // Ungated → all N can run at once; peak should reach N.
        let gates: CacheGates<u32> = CacheGates::new();
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let n = 8;
        let barrier = Arc::new(Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let gates = gates.clone();
            let current = current.clone();
            let peak = peak.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                let key = format!("vkey-{i}");
                gates
                    .run(CacheKind::VideoThumbnail, &key, || async {
                        let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        // Rendezvous: every task must be inside the job at once;
                        // if video were gated this would deadlock/timeout.
                        barrier.wait().await;
                        current.fetch_sub(1, Ordering::SeqCst);
                        i as u32
                    })
                    .await
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            peak.load(Ordering::SeqCst),
            n,
            "ungated video thumbnails should all run concurrently"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn in_flight_dedup_collapses_same_key_to_one_job() {
        let gates: CacheGates<u32> = CacheGates::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(2));

        let g1 = gates.clone();
        let r1 = runs.clone();
        let s1 = start.clone();
        let h1 = tokio::spawn(async move {
            s1.wait().await;
            g1.run(CacheKind::Waveform, "same-key", || async {
                runs_inc(&r1).await
            })
            .await
        });

        let g2 = gates.clone();
        let r2 = runs.clone();
        let s2 = start.clone();
        let h2 = tokio::spawn(async move {
            s2.wait().await;
            g2.run(CacheKind::Waveform, "same-key", || async {
                runs_inc(&r2).await
            })
            .await
        });

        let (v1, v2) = (h1.await.unwrap(), h2.await.unwrap());
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "two concurrent same-key requests must share ONE computation"
        );
        assert_eq!(v1, v2, "both callers must observe the same result");
        assert_eq!(v1, 42);
    }

    async fn runs_inc(runs: &Arc<AtomicUsize>) -> u32 {
        runs.fetch_add(1, Ordering::SeqCst);
        // Give the second caller time to attach to the in-flight entry.
        tokio::time::sleep(Duration::from_millis(30)).await;
        42
    }

    #[tokio::test]
    async fn in_flight_entry_cleared_after_completion() {
        let gates: CacheGates<u32> = CacheGates::new();
        let _ = gates
            .run(CacheKind::ImageThumbnail, "k", || async { 7u32 })
            .await;
        assert_eq!(
            gates.in_flight_len(CacheKind::ImageThumbnail),
            0,
            "completed job should clear its in-flight registration"
        );
    }
}
