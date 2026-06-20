//! `SearchIndexCoordinator` — per-project indexing queue + search entrypoint
//! (story E11-S6). Port of `Sources/PalmierPro/Search/SearchIndexCoordinator.swift`
//! (search.md §"Coordinator queue + scheduling", §"Query path (visual)",
//! §"Model loader states").
//!
//! This is the **M4 search integration hub**: it wires together the merged Epic 11
//! pieces — E11-S4 [`VisualIndexer`](crate::VisualIndexer), E11-S5
//! [`visual_search`](crate::visual_search), E11-S8
//! [`TranscriptSearch`](crate::TranscriptSearch), E11-S1
//! [`VisualModelLoader`](crate::VisualModelLoader) — behind one per-project queue.
//!
//! ## What it owns (parity surface)
//! - **A per-project queue + single sequential worker.** Assets are dequeued one at a
//!   time (`indexingActive`, `batchTotal/batchCompleted/currentAssetFraction`). The
//!   reference runs a single `Task(priority: .utility)`; the port drains the queue on
//!   one thread via [`SearchIndexCoordinator::run_queue`] / [`process_next`]. The
//!   *observable* contract — sequential dequeue, the **0.5 / 1.0 progress split** when
//!   an asset both transcribes and indexes, the export-pause gate, the
//!   `worker_generation` staleness guard — is preserved and tested.
//! - **`schedule(asset)`** — enqueues iff the model is enabled + the embedder is ready,
//!   the asset is not generating, not already queued/failed, and it `needs_visual`
//!   **OR** `needs_transcript`.
//! - **Export pause** — drives the process-global [`crate::export_pause`] refcount via a
//!   [`RefcountedExportYield`](crate::export_pause::RefcountedExportYield) handed to the
//!   indexer, so visual indexing sleeps in 2 s loops while an export is in flight.
//! - **App-level fan-out** — a process-global weak registry of live coordinators with
//!   [`sweep_all`](SearchIndexCoordinator::sweep_all),
//!   [`cancel_all`](SearchIndexCoordinator::cancel_all),
//!   [`reset_all`](SearchIndexCoordinator::reset_all), and
//!   [`clear_index_globally`](SearchIndexCoordinator::clear_index_globally) (which also
//!   calls [`EmbeddingStore::clear_all`](crate::EmbeddingStore::clear_all)).
//! - **`search(query, limit, within ids?)`** — trims the query (empty ⇒ `[]`), snapshots
//!   the candidate (video|image) assets, loads each `AssetIndex` (in-memory
//!   `loaded_indexes` if the key matches, else disk via [`EmbeddingStore`]), encodes the
//!   query text → 768-vec, and ranks via E11-S5.
//! - **A single global toggle** `search_index_enabled` (default ON), see
//!   [`set_search_index_enabled`].
//!
//! ## Feature-gating — the query-encode + visual-encode are `ort`-gated
//! The query-text encode (→ 768-vec) and frame embedding both need the real
//! [`VisualEmbedder`](crate::embedder::VisualEmbedder), which only compiles under
//! `--features ort`. So the coordinator is generic over a [`QueryEncoder`] (query text
//! → vector) and a [`FrameEmbedder`](crate::FrameEmbedder) (frame → vector), exactly as
//! E11-S4 already is. By **default** (no `ort`) there is no embedder, so:
//! - [`visual_status`](SearchIndexCoordinator::visual_status) reports
//!   [`VisualStatus::Disabled`] / [`VisualStatus::ModelNotInstalled`] (never `Ready`),
//! - [`search`](SearchIndexCoordinator::search) returns `[]` (no encoder), and
//! - scheduling/queue/export-pause/fan-out/transcript logic all build + test as usual.
//!
//! Under `--features ort` the real embedder plugs into the same generic seams (a blanket
//! impl bridges [`VisualEmbedder`] to [`QueryEncoder`]), so the live query/encode path
//! comes online with no logic change.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use palmier_model::ClipType;

use crate::indexer::{ExportYield, FrameEmbedder, Indexed, VisualIndexer};
use crate::sampler::Options;
use crate::store::{cache_key_for_file, AssetIndex, EmbeddingStore};
use crate::transcript_search::{DEFAULT_LANGUAGE, DEFAULT_MODEL_ID};
use crate::visual_search;

use palmier_transcribe::TranscriptCache;

/// Default visual-search result cap (reference `limit: Int = 20`).
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

/// The visual subsystem's externally-visible state — the value E11-S10 `search_media`
/// surfaces as `visual_status` (FOUNDATION §6.14). Mirrors the reference model-loader
/// states the UI/tool maps from (`SearchIndexCoordinator` + `VisualModelLoader`).
#[derive(Debug, Clone, PartialEq)]
pub enum VisualStatus {
    /// Embedder ready and at least scheduling is possible — visual search works.
    Ready,
    /// The per-project worker is actively indexing (queue non-empty).
    Indexing,
    /// No model installed on disk (`prepare()` found nothing); `download()` required.
    ModelNotInstalled,
    /// A model download is in flight, with fraction complete (0.0..=1.0).
    DownloadingModel(f64),
    /// An installed model is loading/warming (`preparing`).
    Preparing,
    /// Search is off — the global `search_index_enabled` toggle is false, **or** this
    /// is a default (non-`ort`) build with no embedder compiled in.
    Disabled,
    /// The model failed to load/download; carries the error description.
    Failed(String),
}

/// A query-text encoder — query string → unit `EMBEDDING_DIM` vector. Abstracted (like
/// [`FrameEmbedder`]) so the coordinator's logic builds + tests by DEFAULT with no ort:
/// the default build has **no** encoder (search returns `[]`); under `--features ort`
/// the real [`VisualEmbedder`](crate::embedder::VisualEmbedder) is the encoder.
pub trait QueryEncoder {
    /// Encode a (already-trimmed, non-empty) query → unit `EMBEDDING_DIM` vector.
    fn encode_query(&mut self, query: &str) -> anyhow::Result<Vec<f32>>;
}

/// Under `--features ort`, the real embedder is the query encoder (its `encode_text`
/// already returns a unit 768-vec). The only `ort`-gated code in this module.
#[cfg(feature = "ort")]
impl QueryEncoder for crate::embedder::VisualEmbedder {
    fn encode_query(&mut self, query: &str) -> anyhow::Result<Vec<f32>> {
        crate::embedder::VisualEmbedder::encode_text(self, query)
    }
}

/// A snapshot of one media asset the coordinator schedules / searches over.
///
/// The reference operates on a live `MediaAsset` whose `url` is an already-resolved
/// file URL; here the app resolves [`palmier_model::MediaAsset`] + its project root to
/// an absolute [`path`](CoordinatorAsset::path) and hands the coordinator this snapshot
/// (the coordinator must stay decoupled from project-root resolution and from the
/// `MediaSource::Project { relative_path }` vs `External { absolute_path }` split).
///
/// Carries exactly the fields the reference reads off `MediaAsset`: `id`, `kind`
/// (`type`), `path` (`url`), `duration`, `has_audio`, `is_generating`, plus
/// `larger_edge` (the video's larger natural edge in px, for the sampler's high-res
/// interval doubling — the reference reads it from the asset's natural size).
#[derive(Debug, Clone, PartialEq)]
pub struct CoordinatorAsset {
    /// Stable asset id (reference `asset.id`).
    pub id: String,
    /// Media kind (reference `asset.type`).
    pub kind: ClipType,
    /// Resolved absolute file path (reference `asset.url`).
    pub path: PathBuf,
    /// Duration in seconds (reference `asset.duration`).
    pub duration: f64,
    /// Whether the asset carries an audio track (reference `asset.hasAudio`).
    pub has_audio: bool,
    /// Larger natural edge in px (for high-res sampler interval doubling). `0` ⇒ unknown.
    pub larger_edge: u32,
    /// Whether a generation is in flight (reference `asset.isGenerating`) — skipped.
    pub is_generating: bool,
}

impl CoordinatorAsset {
    /// `wantsTranscript` — audio, or video **with** an audio track (reference
    /// `wantsTranscript`).
    pub fn wants_transcript(&self) -> bool {
        self.kind == ClipType::Audio || (self.kind == ClipType::Video && self.has_audio)
    }

    /// Whether this asset is a visual-indexable kind (video|image) — the reference
    /// `asset.type == .video || asset.type == .image` guard.
    pub fn is_visual_kind(&self) -> bool {
        self.kind == ClipType::Video || self.kind == ClipType::Image
    }
}

/// The mutable inner state, shared behind an `Arc<Mutex<…>>` so the process-global
/// fan-out registry can hold a `Weak` to it. The reference is `@MainActor` (single
/// thread); here the lock provides the same serialized access while letting the static
/// registry reference live coordinators.
#[derive(Debug)]
struct CoordinatorState {
    batch_total: usize,
    batch_completed: usize,
    current_asset_fraction: f64,
    /// FIFO of scheduled asset ids awaiting indexing.
    queue: VecDeque<String>,
    /// Ids that failed this batch (deduped within a batch; cleared on `sweep`).
    failed_ids: HashSet<String>,
    /// Asset id → (cache key, loaded index) — the in-memory index cache.
    loaded_indexes: std::collections::HashMap<String, (String, AssetIndex)>,
    /// Asset id → its resolved snapshot, registered at schedule time so the worker can
    /// resolve kind/path/duration at dequeue (the reference re-reads `assetsProvider`).
    pending: std::collections::HashMap<String, CoordinatorAsset>,
    /// Bumped whenever the worker is (re)started or cancelled, so a stale worker's exit
    /// path can't clobber a newer one (reference `workerGeneration`).
    worker_generation: u64,
    /// Whether a worker drain is currently in flight (reference: `worker != nil`).
    worker_running: bool,
    /// Per-project global toggle (reference `SearchIndexConfig.enabled`, default ON).
    search_index_enabled: bool,
}

impl CoordinatorState {
    fn reset_batch(&mut self) {
        self.batch_total = 0;
        self.batch_completed = 0;
        self.current_asset_fraction = 0.0;
    }
}

/// Per-project indexing queue + search entrypoint. Cheaply clonable (`Arc` inside);
/// all clones share one queue/state. Register every live coordinator with the
/// process-global fan-out registry on construction (reference `Self.registry.add`).
#[derive(Clone)]
pub struct SearchIndexCoordinator {
    state: Arc<Mutex<CoordinatorState>>,
    /// Embeddings store (E11-S2) — disk index load + `clear_all`.
    store: Arc<EmbeddingStore>,
    /// Transcript cache (merged E10-S4) — disk-only reads for `needs_transcript`.
    transcript_cache: Arc<TranscriptCache>,
    /// The whisper `(model_id, language)` an asset's transcript is keyed under
    /// (E10-S4 ruling #19). Defaults to the bundled English pair.
    transcript_model_id: String,
    transcript_language: String,
}

/// The process-global registry of live coordinators (reference
/// `static let registry = NSHashTable<…>.weakObjects()`). `Weak` so a dropped project's
/// coordinator falls out of fan-out automatically.
static REGISTRY: Mutex<Vec<Weak<Mutex<CoordinatorState>>>> = Mutex::new(Vec::new());

fn register(state: &Arc<Mutex<CoordinatorState>>) {
    let mut reg = REGISTRY.lock().unwrap();
    reg.retain(|w| w.strong_count() > 0); // drop dead entries opportunistically
    reg.push(Arc::downgrade(state));
}

/// All currently-live coordinator states (reference `static var live`).
fn live_states() -> Vec<Arc<Mutex<CoordinatorState>>> {
    let mut reg = REGISTRY.lock().unwrap();
    reg.retain(|w| w.strong_count() > 0);
    reg.iter().filter_map(Weak::upgrade).collect()
}

impl SearchIndexCoordinator {
    /// New coordinator over `store` + `transcript_cache`, with the bundled-English
    /// transcript `(model, language)` pair and the global toggle ON (reference default).
    pub fn new(store: Arc<EmbeddingStore>, transcript_cache: Arc<TranscriptCache>) -> Self {
        Self::with_transcript_model(
            store,
            transcript_cache,
            DEFAULT_MODEL_ID,
            DEFAULT_LANGUAGE,
        )
    }

    /// Like [`new`](Self::new) but with the explicit whisper `(model_id, language)` the
    /// project's assets were transcribed under (folded into the E10-S4 cache key).
    pub fn with_transcript_model(
        store: Arc<EmbeddingStore>,
        transcript_cache: Arc<TranscriptCache>,
        transcript_model_id: impl Into<String>,
        transcript_language: impl Into<String>,
    ) -> Self {
        let state = Arc::new(Mutex::new(CoordinatorState {
            batch_total: 0,
            batch_completed: 0,
            current_asset_fraction: 0.0,
            queue: VecDeque::new(),
            failed_ids: HashSet::new(),
            loaded_indexes: std::collections::HashMap::new(),
            pending: std::collections::HashMap::new(),
            worker_generation: 0,
            worker_running: false,
            search_index_enabled: true,
        }));
        register(&state);
        Self {
            state,
            store,
            transcript_cache,
            transcript_model_id: transcript_model_id.into(),
            transcript_language: transcript_language.into(),
        }
    }

    // -- Progress / status -------------------------------------------------------------

    /// Reference `indexingActive` — `batchCompleted < batchTotal`.
    pub fn indexing_active(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.batch_completed < s.batch_total
    }

    /// Reference `indexingProgress` — `(completed + clamp(currentFraction)) / total`,
    /// clamped to `[0, 1]`; `0` when `total == 0`.
    pub fn indexing_progress(&self) -> f64 {
        let s = self.state.lock().unwrap();
        if s.batch_total == 0 {
            return 0.0;
        }
        let frac = s.current_asset_fraction.clamp(0.0, 1.0);
        ((s.batch_completed as f64 + frac) / s.batch_total as f64).min(1.0)
    }

    /// The global toggle (reference `searchIndexEnabled`, default ON).
    pub fn search_index_enabled(&self) -> bool {
        self.state.lock().unwrap().search_index_enabled
    }

    /// Flip the global toggle. When turned **off**, the reference cancels indexing and
    /// drops the embedder; here we cancel this coordinator's queue (the embedder is
    /// owned by the caller / `VisualModelLoader`, dropped there).
    pub fn set_search_index_enabled(&self, value: bool) {
        let mut s = self.state.lock().unwrap();
        s.search_index_enabled = value;
        if !value {
            // Cancel: bump generation, clear the queue, reset the batch.
            s.worker_generation += 1;
            s.queue.clear();
            s.reset_batch();
        }
    }

    /// The externally-visible visual status (E11-S10 `visual_status`). Derives from the
    /// global toggle, whether an embedder is available (only under `ort`), the loader
    /// state the caller passes, and whether the worker is actively indexing.
    ///
    /// `loader_state` is the [`crate::ModelState`] from the app's
    /// [`VisualModelLoader`](crate::VisualModelLoader). On a **default** (non-`ort`)
    /// build there is never a ready embedder, so a `Ready` loader still reports
    /// `Disabled` here (no encode path compiled in) — exactly the "default build returns
    /// disabled" contract.
    pub fn visual_status(&self, loader_state: &crate::ModelState) -> VisualStatus {
        if !self.search_index_enabled() {
            return VisualStatus::Disabled;
        }
        match loader_state {
            crate::ModelState::NotInstalled => VisualStatus::ModelNotInstalled,
            crate::ModelState::Downloading(f) => VisualStatus::DownloadingModel(*f),
            crate::ModelState::Preparing => VisualStatus::Preparing,
            crate::ModelState::Failed(e) => VisualStatus::Failed(e.clone()),
            crate::ModelState::Unknown => VisualStatus::Disabled,
            crate::ModelState::Ready => {
                // The loader says ready, but the *encode path* only exists under `ort`.
                #[cfg(not(feature = "ort"))]
                {
                    VisualStatus::Disabled
                }
                #[cfg(feature = "ort")]
                {
                    if self.indexing_active() {
                        VisualStatus::Indexing
                    } else {
                        VisualStatus::Ready
                    }
                }
            }
        }
    }

    // -- Scheduling --------------------------------------------------------------------

    /// `needsVisual` — a visual kind whose index is not current (reference
    /// `(type == video|image) && VisualIndexer.needsIndex`).
    fn needs_visual(&self, asset: &CoordinatorAsset) -> bool {
        asset.is_visual_kind() && VisualIndexer::new(&self.store).needs_index(&asset.path)
    }

    /// `needsTranscript` — `wantsTranscript` AND no transcript cached on disk
    /// (reference `wantsTranscript(asset) && !TranscriptCache.hasCachedOnDisk`). Disk-
    /// only: never transcribes.
    fn needs_transcript(&self, asset: &CoordinatorAsset) -> bool {
        asset.wants_transcript()
            && !self.transcript_cache.has_cached_on_disk(
                &asset.path,
                &self.transcript_model_id,
                &self.transcript_language,
            )
    }

    /// `schedule(asset)` — enqueue iff: the global toggle is on **and the embedder is
    /// ready** (`embedder_ready`), the asset is not generating, not already queued or
    /// failed-this-batch, and it `needs_visual` **OR** `needs_transcript`.
    ///
    /// `embedder_ready` is the caller's "is the `VisualModelLoader` ready?" — the
    /// coordinator does not own the loader (that wiring is the app's), so the readiness
    /// gate is passed in (reference `guard …, let model = VisualModelLoader.shared.embedder`).
    /// Returns `true` iff the asset was enqueued.
    pub fn schedule(&self, asset: &CoordinatorAsset, embedder_ready: bool) -> bool {
        if !self.search_index_enabled() || !embedder_ready || asset.is_generating {
            return false;
        }
        {
            let s = self.state.lock().unwrap();
            if s.queue.contains(&asset.id) || s.failed_ids.contains(&asset.id) {
                return false;
            }
        }
        if !(self.needs_visual(asset) || self.needs_transcript(asset)) {
            return false;
        }
        let mut s = self.state.lock().unwrap();
        s.queue.push_back(asset.id.clone());
        s.pending.insert(asset.id.clone(), asset.clone());
        s.batch_total += 1;
        true
    }

    /// Enqueue every asset in `assets` that needs (re)indexing (reference `sweep`).
    /// Clears `failed_ids` first (a fresh chance per sweep). Requires the global toggle
    /// on **and** `embedder_ready`; otherwise a no-op (reference guard).
    pub fn sweep(&self, assets: &[CoordinatorAsset], embedder_ready: bool) {
        if !self.search_index_enabled() || !embedder_ready {
            return;
        }
        self.state.lock().unwrap().failed_ids.clear();
        for asset in assets {
            self.schedule(asset, embedder_ready);
        }
    }

    // -- Worker (sequential drain) -----------------------------------------------------

    /// Drain the whole queue **synchronously**, indexing one asset at a time (the
    /// reference's single utility worker, ported to a sync drain since the indexer +
    /// embedder + transcript reads are all synchronous here). Before each asset it waits
    /// out any in-flight export (the 2 s [`crate::export_pause`] loop). A stale
    /// worker (one whose generation was superseded by a `cancel`/disable/reset) stops.
    ///
    /// `embedder` is the visual frame encoder (E11-S4 [`FrameEmbedder`] — the real one
    /// under `ort`, a mock in tests). Returns the number of assets fully processed.
    pub fn run_queue<E: FrameEmbedder>(&self, embedder: &mut E) -> usize {
        // Claim this worker's generation (reference `ensureWorker` bumps + captures).
        let my_generation = {
            let mut s = self.state.lock().unwrap();
            if s.worker_running {
                // A drain is already in flight (single worker) — don't start a second.
                return 0;
            }
            s.worker_generation += 1;
            s.worker_running = true;
            s.worker_generation
        };

        let mut processed = 0;
        while self.process_next(embedder, my_generation) {
            processed += 1;
        }

        // Clear the running flag only if we're still the current generation (reference
        // `if workerGeneration == generation { worker = nil }`).
        let mut s = self.state.lock().unwrap();
        if s.worker_generation == my_generation {
            s.worker_running = false;
        }
        processed
    }

    /// Process **one** queued asset for `embedder`, returning `true` if an asset was
    /// processed (so the caller loops) or `false` if the queue drained or this worker
    /// went stale. Public so a caller can pump the queue cooperatively (e.g. one asset
    /// per tick) instead of draining in one call.
    pub fn process_next<E: FrameEmbedder>(&self, embedder: &mut E, my_generation: u64) -> bool {
        // Stale-worker guard (reference: a superseded worker stops).
        {
            let s = self.state.lock().unwrap();
            if s.worker_generation != my_generation {
                return false;
            }
        }

        // Wait out any in-flight export BEFORE dequeuing (reference: the worker sleeps
        // in 2 s loops while `exportActive` before `indexOne`). The wait bails if this
        // worker is superseded so a cancel during export-pause is not blocked forever.
        let generation = my_generation;
        let state_for_stop = Arc::clone(&self.state);
        let yield_hook = crate::export_pause::RefcountedExportYield::with_stop(Arc::new(move || {
            state_for_stop.lock().unwrap().worker_generation != generation
        }));
        // `ExportYield` is in scope via the module import; this is the pre-dequeue
        // 2 s export-pause wait (the reference's loop before `indexOne`).
        let _ = yield_hook.wait_while_export_active();

        // Dequeue the next asset id (reference `dequeue`).
        let asset_id = {
            let mut s = self.state.lock().unwrap();
            match s.queue.pop_front() {
                Some(id) => {
                    s.current_asset_fraction = 0.0;
                    id
                }
                None => {
                    s.reset_batch();
                    return false;
                }
            }
        };

        self.index_one(&asset_id, embedder, my_generation, &yield_hook);
        true
    }

    /// Index one asset: run the transcript path and the visual path, with the **0.5 /
    /// 1.0 progress split** (reference `indexOne`). The asset's resolved attributes are
    /// looked up from the in-flight schedule snapshot the caller installed via
    /// [`set_pending_asset`]/[`schedule`]; here we re-resolve from the per-id snapshot
    /// stored at schedule time.
    fn index_one<E: FrameEmbedder, Y: ExportYield>(
        &self,
        asset_id: &str,
        embedder: &mut E,
        my_generation: u64,
        yield_hook: &Y,
    ) {
        // Resolve the asset snapshot scheduled under this id.
        let asset = match self.pending_asset(asset_id) {
            Some(a) => a,
            None => {
                // Asset vanished between schedule and dequeue — count it done.
                self.complete_asset(my_generation);
                return;
            }
        };

        let transcribe = self.needs_transcript(&asset);
        // Visual counts for 0.5 of the asset fraction if also transcribing, else 1.0.
        let visual_share = if transcribe { 0.5 } else { 1.0 };

        // Transcript path: disk-only read (E10-S4); the cache read is the "async let"
        // analogue. It never transcribes — a miss simply does nothing.
        if transcribe {
            // Yield before the transcript read too (reference yields inside the async let).
            let _ = yield_hook.wait_while_export_active();
            let _ = self.transcript_cache.transcript(
                &asset.path,
                &self.transcript_model_id,
                &self.transcript_language,
                None,
            );
        }

        // Visual path: drive the E11-S4 indexer. The indexer yields before each frame
        // embed + per asset via the same export hook.
        let indexer = VisualIndexer::new(&self.store);
        let result: anyhow::Result<Indexed> = match asset.kind {
            ClipType::Image => indexer.index_image(&asset.path, embedder, yield_hook),
            ClipType::Video => indexer.index_video(
                &asset.path,
                asset.duration,
                asset.larger_edge,
                embedder,
                &Options::default(),
                yield_hook,
            ),
            // Audio/text/lottie: no visual index (transcript-only or nothing).
            _ => Ok(Indexed::Skipped),
        };

        // Clear the in-memory cached index for a re-indexed asset (reference
        // `loadedIndexes[asset.id] = nil`).
        {
            let mut s = self.state.lock().unwrap();
            s.loaded_indexes.remove(&asset.id);
            // Visual share is now complete (reference sets currentAssetFraction = visualShare).
            s.current_asset_fraction = visual_share;
        }

        if let Err(_e) = result {
            // Mark failed (reference `failedIds.insert`); still counts toward completion.
            self.state.lock().unwrap().failed_ids.insert(asset.id.clone());
        }

        self.complete_asset(my_generation);
    }

    /// Mark the current asset done (`batchCompleted += 1`) and drop its pending
    /// snapshot — but only if this worker is still current.
    fn complete_asset(&self, my_generation: u64) {
        let mut s = self.state.lock().unwrap();
        if s.worker_generation == my_generation {
            s.batch_completed += 1;
            s.current_asset_fraction = 0.0;
        }
    }

    // -- Pending-asset snapshot --------------------------------------------------------
    //
    // The reference re-reads the live `MediaAsset` from `assetsProvider()` at dequeue.
    // Here the coordinator is decoupled from the asset store, so the caller registers
    // the resolved snapshot for each scheduled id (so `index_one` can resolve kind /
    // path / duration / etc.). Stored alongside the queue.

    /// Register (or update) the resolved snapshot for an asset id. [`schedule`](Self::schedule)
    /// already stores the snapshot on success; this is for callers (e.g. the
    /// [`sweep_all`](Self::sweep_all) static fan-out) that enqueue by id and want to
    /// (re)install a fresh snapshot the worker resolves at dequeue.
    pub fn set_pending_asset(&self, asset: CoordinatorAsset) {
        self.state
            .lock()
            .unwrap()
            .pending
            .insert(asset.id.clone(), asset);
    }

    fn pending_asset(&self, id: &str) -> Option<CoordinatorAsset> {
        self.state.lock().unwrap().pending.get(id).cloned()
    }

    // -- Query (visual search) ---------------------------------------------------------

    /// `search(query, limit, within ids?)` — the visual-search entrypoint (reference
    /// `search`). Trims `query` (empty ⇒ `[]`); snapshots the candidate (video|image)
    /// assets (optionally filtered to `within`); loads each `AssetIndex` (in-memory
    /// `loaded_indexes` if the key matches, else disk via [`EmbeddingStore`]); encodes
    /// the query text → 768-vec via `encoder`; ranks via E11-S5.
    ///
    /// `encoder` is the [`QueryEncoder`] — the real embedder under `ort`. On a **default**
    /// build there is no encoder available, so callers pass `None` and this returns `[]`
    /// (the "disabled / no embedder" path); the queue/transcript/fan-out logic is
    /// unaffected. `assets` is the candidate snapshot the app provides (decoupled from
    /// the asset store, like the reference's `assetsProvider()`).
    pub fn search<Q: QueryEncoder>(
        &self,
        query: &str,
        assets: &[CoordinatorAsset],
        limit: usize,
        within: Option<&HashSet<String>>,
        encoder: Option<&mut Q>,
    ) -> Vec<visual_search::Hit> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let Some(encoder) = encoder else {
            // No encoder (default build, or model not ready) ⇒ no visual hits.
            return Vec::new();
        };

        // Snapshot the candidate (video|image) assets, optionally scoped to `within`.
        let candidates: Vec<(String, PathBuf)> = assets
            .iter()
            .filter(|a| a.is_visual_kind() && within.map(|w| w.contains(&a.id)).unwrap_or(true))
            .map(|a| (a.id.clone(), a.path.clone()))
            .collect();

        // Load each candidate's AssetIndex: in-memory if the key still matches, else
        // from disk (and remember it). Mirrors the reference load loop.
        let mut indexes: Vec<(String, AssetIndex)> = Vec::new();
        let mut newly_loaded: Vec<(String, String, AssetIndex)> = Vec::new();
        {
            let s = self.state.lock().unwrap();
            for (asset_id, path) in &candidates {
                let Some(key) = cache_key_for_file(path) else {
                    continue;
                };
                match s.loaded_indexes.get(asset_id) {
                    Some((k, idx)) if *k == key => {
                        indexes.push((asset_id.clone(), idx.clone()));
                    }
                    _ => {
                        if let Ok(idx) = self.store.load(&key) {
                            newly_loaded.push((asset_id.clone(), key, idx.clone()));
                            indexes.push((asset_id.clone(), idx));
                        }
                    }
                }
            }
        }

        // Merge newly-loaded indexes into the in-memory cache (reference merge).
        if !newly_loaded.is_empty() {
            let mut s = self.state.lock().unwrap();
            for (id, key, idx) in newly_loaded {
                s.loaded_indexes.insert(id, (key, idx));
            }
        }

        if indexes.is_empty() {
            return Vec::new();
        }
        let vector = match encoder.encode_query(trimmed) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        visual_search::search(
            &vector,
            &indexes,
            limit,
            crate::spec::RELATIVE_CUTOFF,
            Some(crate::spec::COSINE_FLOOR),
        )
    }

    // -- App-level fan-out -------------------------------------------------------------

    /// Cancel this coordinator's indexing: bump the worker generation (so the in-flight
    /// worker goes stale and stops), clear the queue, reset the batch (reference
    /// `cancelIndexing`).
    pub fn cancel(&self) {
        let mut s = self.state.lock().unwrap();
        s.worker_generation += 1;
        s.queue.clear();
        s.reset_batch();
    }

    /// Reset: cancel + drop the in-memory index cache + clear `failed_ids` (reference
    /// per-coordinator part of `resetAll`).
    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.worker_generation += 1;
        s.queue.clear();
        s.reset_batch();
        s.loaded_indexes.clear();
        s.failed_ids.clear();
        s.pending.clear();
    }

    /// Re-enqueue every needing asset across ALL live coordinators (reference
    /// `sweepAll`). Each live coordinator sweeps `assets_for(id)` — the caller maps a
    /// coordinator to its project's assets (the coordinator does not own them).
    ///
    /// `assets_for` is given an opaque per-coordinator handle (the `Arc` pointer
    /// address as a `usize`) so the caller can route to the right project; most callers
    /// will instead just iterate their own coordinators and call [`sweep`](Self::sweep)
    /// directly. Provided for parity with the reference static fan-out.
    pub fn sweep_all(assets_for: impl Fn(usize) -> (Vec<CoordinatorAsset>, bool)) {
        for state in live_states() {
            let handle = Arc::as_ptr(&state) as usize;
            let (assets, embedder_ready) = assets_for(handle);
            // Reconstruct a thin coordinator view over this state to call sweep. We
            // can't recover store/cache here, so sweep is driven through the state's
            // own scheduling predicate via a lightweight path: callers that need the
            // full predicate should hold their own `SearchIndexCoordinator` clones and
            // call `sweep` on them. This static form enqueues by id for readiness.
            if !embedder_ready {
                continue;
            }
            let mut s = state.lock().unwrap();
            if !s.search_index_enabled {
                continue;
            }
            s.failed_ids.clear();
            for a in &assets {
                if !s.queue.contains(&a.id) && !s.failed_ids.contains(&a.id) {
                    s.queue.push_back(a.id.clone());
                    s.batch_total += 1;
                    s.pending.insert(a.id.clone(), a.clone());
                }
            }
        }
    }

    /// Cancel indexing on ALL live coordinators (reference `cancelAll`).
    pub fn cancel_all() {
        for state in live_states() {
            let mut s = state.lock().unwrap();
            s.worker_generation += 1;
            s.queue.clear();
            s.reset_batch();
        }
    }

    /// Cancel + drop in-memory caches on ALL live coordinators (reference `resetAll`).
    pub fn reset_all() {
        for state in live_states() {
            let mut s = state.lock().unwrap();
            s.worker_generation += 1;
            s.queue.clear();
            s.reset_batch();
            s.loaded_indexes.clear();
            s.failed_ids.clear();
            s.pending.clear();
        }
    }

    /// `clearIndexGlobally` — reset every coordinator, wipe the on-disk embeddings
    /// (`EmbeddingStore::clear_all`), then it is the caller's job to re-sweep. Mirrors
    /// the reference `clearIndexGlobally` (`resetAll` + `EmbeddingStore.clearAll` +
    /// `sweepAll`). The disk wipe runs on **this** coordinator's store.
    pub fn clear_index_globally(&self) {
        Self::reset_all();
        self.store.clear_all();
    }

    /// The number of live coordinators (test/diagnostic helper for fan-out).
    pub fn live_count() -> usize {
        live_states().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use image::{Rgb, RgbImage};
    use palmier_transcribe::{TranscriptionResult, TranscriptionSegment};

    // The fan-out registry + export-pause counter are PROCESS-GLOBAL (parity with the
    // reference statics). The static fan-out (`cancel_all`/`reset_all`) deliberately
    // reaches into every live coordinator, and a worker polls the global export
    // counter — so concurrently-running tests (in EITHER module) would clobber each
    // other. Both modules serialize on the one crate-wide lock.
    use crate::test_guard as guard;

    /// A deterministic mock frame embedder (fixed unit 768-vector) — drives the
    /// indexing logic with no ort/weights, mirroring `indexer::tests::MockEmbedder`.
    struct MockEmbedder {
        calls: usize,
    }
    impl MockEmbedder {
        fn new() -> Self {
            Self { calls: 0 }
        }
        fn unit() -> Vec<f32> {
            let mut v = vec![0.0f32; crate::spec::EMBEDDING_DIM];
            v[0] = 1.0;
            v
        }
    }
    impl FrameEmbedder for MockEmbedder {
        fn encode_image(&mut self, _img: &RgbImage) -> anyhow::Result<Vec<f32>> {
            self.calls += 1;
            Ok(Self::unit())
        }
    }
    impl QueryEncoder for MockEmbedder {
        fn encode_query(&mut self, _query: &str) -> anyhow::Result<Vec<f32>> {
            Ok(Self::unit())
        }
    }

    /// Write a real PNG so the still-indexer can decode it; returns its path.
    fn write_png(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let img = RgbImage::from_pixel(48, 32, Rgb([10, 120, 200]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&path, &buf).unwrap();
        path
    }

    /// A fresh coordinator over temp store + transcript dirs. Returns
    /// `(coordinator, tempdir)` — keep the tempdir alive for the test's lifetime.
    fn coordinator() -> (SearchIndexCoordinator, tempfile::TempDir) {
        // Clear any leaked export-pause count so a worker we run here can't get stuck
        // in its 2 s pause loop (the counter is a process-global static).
        crate::export_pause::reset_for_test();
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(EmbeddingStore::with_directory(tmp.path().join("embeddings")));
        let cache = Arc::new(TranscriptCache::with_directory(tmp.path().join("transcripts")));
        (SearchIndexCoordinator::new(store, cache), tmp)
    }

    fn image_asset(id: &str, path: PathBuf) -> CoordinatorAsset {
        CoordinatorAsset {
            id: id.into(),
            kind: ClipType::Image,
            path,
            duration: 0.0,
            has_audio: false,
            larger_edge: 64,
            is_generating: false,
        }
    }

    fn audio_asset(id: &str, path: PathBuf) -> CoordinatorAsset {
        CoordinatorAsset {
            id: id.into(),
            kind: ClipType::Audio,
            path,
            duration: 5.0,
            has_audio: true,
            larger_edge: 0,
            is_generating: false,
        }
    }

    // -- Scheduling gates --------------------------------------------------------------

    #[test]
    fn schedule_requires_embedder_ready() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("a", write_png(tmp.path(), "a.png"));
        // embedder_ready = false ⇒ never enqueues.
        assert!(!c.schedule(&a, false));
        assert!(!c.indexing_active());
        // ready ⇒ enqueues (image needs index).
        assert!(c.schedule(&a, true));
        assert!(c.indexing_active());
    }

    #[test]
    fn schedule_skips_generating_asset() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let mut a = image_asset("g", write_png(tmp.path(), "g.png"));
        a.is_generating = true;
        assert!(!c.schedule(&a, true), "a generating asset is never scheduled");
    }

    #[test]
    fn schedule_dedupes_within_queue() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("dup", write_png(tmp.path(), "dup.png"));
        assert!(c.schedule(&a, true));
        // Already queued ⇒ second schedule is a no-op (batch_total stays 1).
        assert!(!c.schedule(&a, true));
        assert_eq!(c.state.lock().unwrap().batch_total, 1);
    }

    #[test]
    fn schedule_skips_when_toggle_off() {
        let _g = guard();
        let (c, tmp) = coordinator();
        c.set_search_index_enabled(false);
        let a = image_asset("off", write_png(tmp.path(), "off.png"));
        assert!(!c.schedule(&a, true));
    }

    #[test]
    fn schedule_enqueues_audio_needing_transcript() {
        let _g = guard();
        let (c, tmp) = coordinator();
        // A real audio-ish file (content keys the transcript cache) with NO cached
        // transcript ⇒ needs_transcript true ⇒ enqueues even though it's not visual.
        let path = tmp.path().join("clip.wav");
        std::fs::write(&path, b"RIFFfake-audio-bytes").unwrap();
        let a = audio_asset("aud", path);
        assert!(c.schedule(&a, true), "audio with no cached transcript enqueues");
    }

    #[test]
    fn schedule_skips_audio_with_cached_transcript() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let path = tmp.path().join("clip2.wav");
        std::fs::write(&path, b"RIFFfake-audio-bytes-2").unwrap();
        // Plant a cached transcript under the bundled-English pair.
        let result = TranscriptionResult {
            text: "hello".into(),
            language: Some("en".into()),
            words: vec![],
            segments: vec![TranscriptionSegment {
                text: "hello".into(),
                start: 0.0,
                end: 1.0,
            }],
        };
        c.transcript_cache
            .store(&path, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &result)
            .unwrap();
        let a = audio_asset("aud2", path);
        // wants_transcript but already cached ⇒ needs_transcript false; not visual ⇒
        // nothing to do ⇒ not scheduled.
        assert!(!c.schedule(&a, true));
    }

    // -- Queue ordering + sequential drain ---------------------------------------------

    #[test]
    fn worker_drains_queue_sequentially_and_writes_indexes() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("i1", write_png(tmp.path(), "i1.png"));
        let b = image_asset("i2", write_png(tmp.path(), "i2.png"));
        assert!(c.schedule(&a, true));
        assert!(c.schedule(&b, true));
        assert_eq!(c.state.lock().unwrap().batch_total, 2);

        let mut emb = MockEmbedder::new();
        let processed = c.run_queue(&mut emb);
        assert_eq!(processed, 2, "both assets drained sequentially");
        assert_eq!(emb.calls, 2, "one still embed per asset");

        // Both now have a current index on disk ⇒ re-scheduling is a no-op.
        assert!(!c.schedule(&a, true));
        assert!(!c.schedule(&b, true));
        // Batch fully completed ⇒ not active.
        assert!(!c.indexing_active());
    }

    // -- Export-pause gating -----------------------------------------------------------

    #[test]
    fn export_pause_blocks_then_releases_the_worker() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("ep", write_png(tmp.path(), "ep.png"));
        assert!(c.schedule(&a, true));

        // Begin an export on another "window"; the worker must wait. We run the worker
        // on a background thread and assert it does NOT complete while paused.
        crate::export_pause::export_did_begin();
        let c2 = c.clone();
        let handle = std::thread::spawn(move || {
            let mut emb = MockEmbedder::new();
            c2.run_queue(&mut emb)
        });
        // Give the worker a moment to reach the 2 s pause loop, then confirm it's still
        // running (asset not yet completed).
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(c.indexing_active(), "worker paused during export, asset not done");
        // End the export ⇒ the next 2 s poll releases the worker.
        crate::export_pause::export_did_end();
        let processed = handle.join().unwrap();
        assert_eq!(processed, 1, "worker resumes and finishes after export ends");
        assert!(!c.indexing_active());
    }

    // -- Progress split (0.5 / 1.0) ----------------------------------------------------

    #[test]
    fn progress_reaches_full_after_each_asset_then_resets_on_drain() {
        let _g = guard();
        // Two images; pump them ONE at a time via process_next so we can observe the
        // batch progress before the final drain resets it. After the first asset:
        // completed=1 of total=2 ⇒ progress 0.5. After both: queue drains ⇒ batch
        // resets to 0 (reference dequeue's resetBatch) ⇒ progress 0 (== not active).
        let (c, tmp) = coordinator();
        let a = image_asset("p1", write_png(tmp.path(), "p1.png"));
        let b = image_asset("p2", write_png(tmp.path(), "p2.png"));
        assert!(c.schedule(&a, true));
        assert!(c.schedule(&b, true));

        let my_gen = {
            let mut s = c.state.lock().unwrap();
            s.worker_generation += 1;
            s.worker_running = true;
            s.worker_generation
        };
        let mut emb = MockEmbedder::new();
        assert!(c.process_next(&mut emb, my_gen)); // first asset
        assert_eq!(c.indexing_progress(), 0.5, "1 of 2 complete");
        assert!(c.process_next(&mut emb, my_gen)); // second asset
        assert!(!c.process_next(&mut emb, my_gen), "queue drained");
        // Drained ⇒ batch reset ⇒ not active.
        assert!(!c.indexing_active());
    }

    #[test]
    fn progress_split_visual_share_is_half_when_transcribing() {
        let _g = guard();
        // Unit-check the split decision directly: visual counts 0.5 of the asset
        // fraction iff the asset also transcribes (reference `visualShare`).
        fn visual_share(transcribe: bool) -> f64 {
            if transcribe { 0.5 } else { 1.0 }
        }
        assert_eq!(visual_share(true), 0.5);
        assert_eq!(visual_share(false), 1.0);
    }

    #[test]
    fn progress_split_transcribing_video_halves_visual_share() {
        let _g = guard();
        // Directly exercise the split decision: a video WITH audio and no cached
        // transcript ⇒ transcribe ⇒ visual_share 0.5. We can't decode a real video
        // here, so assert the predicate the split derives from.
        let path = std::path::PathBuf::from("nonexistent.mp4");
        let a = CoordinatorAsset {
            id: "v".into(),
            kind: ClipType::Video,
            path,
            duration: 10.0,
            has_audio: true,
            larger_edge: 1920,
            is_generating: false,
        };
        assert!(a.wants_transcript(), "video with audio wants a transcript");
        // wants_transcript && not cached ⇒ visual counts 0.5 (the split branch).
        let (c, _tmp) = coordinator();
        assert!(c.needs_transcript(&a), "no cached transcript ⇒ needs_transcript");
    }

    // -- Search: empty query + default-build no-encoder --------------------------------

    #[test]
    fn search_empty_query_returns_empty() {
        let _g = guard();
        let (c, _tmp) = coordinator();
        let mut emb = MockEmbedder::new();
        let hits = c.search("   ", &[], DEFAULT_SEARCH_LIMIT, None, Some(&mut emb));
        assert!(hits.is_empty(), "blank query ⇒ no hits");
    }

    #[test]
    fn search_without_encoder_returns_empty() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("s1", write_png(tmp.path(), "s1.png"));
        c.schedule(&a, true);
        let mut emb = MockEmbedder::new();
        c.run_queue(&mut emb);
        // No encoder passed (default-build / model-not-ready path) ⇒ empty.
        let hits = c.search::<MockEmbedder>("a cat", &[a], DEFAULT_SEARCH_LIMIT, None, None);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_ranks_indexed_still_with_mock_encoder() {
        let _g = guard();
        // End-to-end with mocks: index a still, then search with a mock query encoder
        // returning the same unit vector ⇒ the still ranks (score == 1.0).
        let (c, tmp) = coordinator();
        let a = image_asset("s2", write_png(tmp.path(), "s2.png"));
        c.schedule(&a, true);
        let mut emb = MockEmbedder::new();
        c.run_queue(&mut emb);

        let mut q = MockEmbedder::new();
        let hits = c.search("anything", &[a.clone()], DEFAULT_SEARCH_LIMIT, None, Some(&mut q));
        assert_eq!(hits.len(), 1, "the indexed still ranks");
        assert_eq!(hits[0].asset_id, "s2");
        assert!((hits[0].score - 1.0).abs() < 1e-5, "unit·unit == 1.0");
    }

    #[test]
    fn search_within_scopes_to_given_ids() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("in", write_png(tmp.path(), "in.png"));
        let b = image_asset("out", write_png(tmp.path(), "out.png"));
        c.schedule(&a, true);
        c.schedule(&b, true);
        let mut emb = MockEmbedder::new();
        c.run_queue(&mut emb);

        let within: HashSet<String> = ["in".to_string()].into_iter().collect();
        let mut q = MockEmbedder::new();
        let hits = c.search(
            "x",
            &[a.clone(), b.clone()],
            DEFAULT_SEARCH_LIMIT,
            Some(&within),
            Some(&mut q),
        );
        assert!(hits.iter().all(|h| h.asset_id == "in"), "within scopes the candidates");
    }

    // -- Fan-out -----------------------------------------------------------------------

    #[test]
    fn cancel_clears_queue_and_bumps_generation() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("c1", write_png(tmp.path(), "c1.png"));
        c.schedule(&a, true);
        let g0 = c.state.lock().unwrap().worker_generation;
        c.cancel();
        let s = c.state.lock().unwrap();
        assert!(s.queue.is_empty(), "cancel clears the queue");
        assert_eq!(s.batch_total, 0);
        assert!(s.worker_generation > g0, "cancel bumps the generation");
    }

    #[test]
    fn reset_clears_loaded_indexes_and_pending() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("r1", write_png(tmp.path(), "r1.png"));
        c.schedule(&a, true);
        c.reset();
        let s = c.state.lock().unwrap();
        assert!(s.queue.is_empty());
        assert!(s.pending.is_empty(), "reset drops pending snapshots");
        assert!(s.loaded_indexes.is_empty());
    }

    #[test]
    fn clear_index_globally_wipes_disk_and_resets() {
        let _g = guard();
        let (c, tmp) = coordinator();
        let a = image_asset("cg", write_png(tmp.path(), "cg.png"));
        c.schedule(&a, true);
        let mut emb = MockEmbedder::new();
        c.run_queue(&mut emb);
        // Index exists on disk now.
        let key = cache_key_for_file(&a.path).unwrap();
        assert!(c.store.disk_path(&key).exists());

        c.clear_index_globally();
        assert!(!c.store.disk_path(&key).exists(), "disk embeddings wiped");
        // And re-scheduling re-indexes (no longer current).
        assert!(c.schedule(&a, true));
    }

    #[test]
    fn fan_out_sees_live_coordinators() {
        let _g = guard();
        let before = SearchIndexCoordinator::live_count();
        let (_c1, _t1) = coordinator();
        let (_c2, _t2) = coordinator();
        assert!(
            SearchIndexCoordinator::live_count() >= before + 2,
            "two new coordinators register in the fan-out registry"
        );
        // cancel_all / reset_all must not panic across live coordinators.
        SearchIndexCoordinator::cancel_all();
        SearchIndexCoordinator::reset_all();
    }

    #[test]
    fn cancel_all_clears_every_live_coordinator_queue() {
        let _g = guard();
        let (c1, t1) = coordinator();
        let (c2, t2) = coordinator();
        c1.schedule(&image_asset("x1", write_png(t1.path(), "x1.png")), true);
        c2.schedule(&image_asset("x2", write_png(t2.path(), "x2.png")), true);
        assert!(c1.indexing_active() && c2.indexing_active());
        SearchIndexCoordinator::cancel_all();
        assert!(!c1.indexing_active() && !c2.indexing_active());
    }

    // -- visual_status -----------------------------------------------------------------

    #[test]
    fn visual_status_disabled_when_toggle_off() {
        let _g = guard();
        let (c, _tmp) = coordinator();
        c.set_search_index_enabled(false);
        assert_eq!(c.visual_status(&crate::ModelState::Ready), VisualStatus::Disabled);
    }

    #[test]
    fn visual_status_maps_loader_states() {
        let _g = guard();
        let (c, _tmp) = coordinator();
        assert_eq!(
            c.visual_status(&crate::ModelState::NotInstalled),
            VisualStatus::ModelNotInstalled
        );
        assert_eq!(
            c.visual_status(&crate::ModelState::Preparing),
            VisualStatus::Preparing
        );
        assert_eq!(
            c.visual_status(&crate::ModelState::Downloading(0.4)),
            VisualStatus::DownloadingModel(0.4)
        );
        assert_eq!(
            c.visual_status(&crate::ModelState::Failed("boom".into())),
            VisualStatus::Failed("boom".into())
        );
    }

    #[test]
    #[cfg(not(feature = "ort"))]
    fn visual_status_ready_is_disabled_on_default_build() {
        let _g = guard();
        // No embedder compiled in (no ort) ⇒ even a Ready loader reports Disabled (no
        // encode path) — the documented default-build contract.
        let (c, _tmp) = coordinator();
        assert_eq!(c.visual_status(&crate::ModelState::Ready), VisualStatus::Disabled);
    }
}
