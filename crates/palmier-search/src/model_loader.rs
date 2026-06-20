//! Visual-model loader state machine — port of the reference `VisualModelLoader`.
//!
//! Ported from the macOS reference
//! `Sources/PalmierPro/Search/Models/VisualModelLoader.swift` (search.md "Model
//! loader states"). The reference is a `@MainActor @Observable` app-level singleton;
//! this port is the *state machine + load path* only — the app-level fan-out
//! (`SearchIndexCoordinator.sweepAll/cancelAll/resetAll`, the `@Observable` wiring,
//! the global enabled toggle) belongs to **E11-S6** (the coordinator). This module
//! owns the state transitions and the "load an installed model, never download"
//! `prepare()` path that E11-S6 drives.
//!
//! ## State machine (search.md L82-84)
//! ```text
//! unknown → notInstalled | preparing → ready | downloading(frac) | failed
//! ```
//! - `prepare()` loads an **installed** model but **never downloads** (idempotent;
//!   only acts from `unknown`).
//! - `download()` (E11-S6) fetches + installs + warms, then loads. **Left a
//!   clearly-marked stub here** — no network in this story.
//! - On load, the reference warms with `encode(text:"warm up")`; the `ort`-gated
//!   load path does the same before transitioning to `ready`.

use std::path::{Path, PathBuf};

/// Loader state — `Equatable` parity with the reference `enum State`.
///
/// `downloading(fraction)` carries 0.0..=1.0 progress; `failed(reason)` carries the
/// error string the UI surfaces (and E11-S10 maps to `visual_status = failed`).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    /// Initial — `prepare()` has not run yet.
    Unknown,
    /// No installed model on disk; `download()` (E11-S6) is required.
    NotInstalled,
    /// Download in flight, with fraction complete (0.0..=1.0). E11-S6 drives this.
    Downloading(f64),
    /// Installed model found; sessions loading + warming.
    Preparing,
    /// Embedder ready — `encode_image`/`encode_text` available.
    Ready,
    /// Load or download failed; carries the error description.
    Failed(String),
}

impl ModelState {
    /// Whether the embedder is loaded and usable.
    pub fn is_ready(&self) -> bool {
        matches!(self, ModelState::Ready)
    }
}

/// Resolved on-disk locations of an installed model (mirrors
/// `ModelDownloader.InstalledModel`). E11-S6's downloader produces this; `prepare()`
/// consumes it. The onnx-community layout flattens to two `.onnx` + `tokenizer.json`
/// under one dir — no `.mlpackage` compile step (ONNX needs none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    /// Directory containing `vision_model.onnx`, `text_model.onnx`, `tokenizer.json`.
    pub dir: PathBuf,
}

impl InstalledModel {
    pub fn vision_model_path(&self) -> PathBuf {
        self.dir.join("vision_model.onnx")
    }
    pub fn text_model_path(&self) -> PathBuf {
        self.dir.join("text_model.onnx")
    }
    pub fn tokenizer_path(&self) -> PathBuf {
        self.dir.join("tokenizer.json")
    }

    /// All three required files exist on disk. Mirrors `ModelDownloader.installed`'s
    /// presence check (the port's "is a complete model installed?").
    pub fn is_complete(&self) -> bool {
        self.vision_model_path().exists()
            && self.text_model_path().exists()
            && self.tokenizer_path().exists()
    }

    /// Probe a directory: `Some(installed)` iff all three files are present.
    pub fn probe(dir: &Path) -> Option<Self> {
        let m = InstalledModel { dir: dir.to_path_buf() };
        if m.is_complete() { Some(m) } else { None }
    }
}

/// The visual-model loader. Holds the state + (under `ort`) the loaded embedder.
///
/// Single-threaded by intent (the reference is `@MainActor`); E11-S6 owns the async
/// driving + coordinator sweep. This struct deliberately does **not** reach into the
/// coordinator — that wiring is E11-S6's, keeping this story's edits additive.
pub struct VisualModelLoader {
    state: ModelState,
    enabled: bool,
    #[cfg(feature = "ort")]
    embedder: Option<crate::embedder::VisualEmbedder>,
}

impl Default for VisualModelLoader {
    fn default() -> Self {
        Self::new(true)
    }
}

impl VisualModelLoader {
    /// New loader in `Unknown`. `enabled` mirrors `SearchIndexConfig.enabled`
    /// (default ON); when disabled, `prepare()` is a no-op.
    pub fn new(enabled: bool) -> Self {
        Self {
            state: ModelState::Unknown,
            enabled,
            #[cfg(feature = "ort")]
            embedder: None,
        }
    }

    pub fn state(&self) -> &ModelState {
        &self.state
    }

    pub fn is_ready(&self) -> bool {
        self.state.is_ready()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Mirrors the reference `setEnabled`: flips the flag. The coordinator-side
    /// effects (sweep on enable, cancel + drop embedder on disable) are E11-S6's; here
    /// we only handle the local state reset on disable.
    pub fn set_enabled(&mut self, value: bool) {
        self.enabled = value;
        if !value {
            #[cfg(feature = "ort")]
            {
                self.embedder = None;
            }
            if matches!(self.state, ModelState::Ready | ModelState::Preparing) {
                self.state = ModelState::Unknown;
            }
        }
    }

    /// `prepare()` — load an **installed** model if present; **never downloads**.
    /// Idempotent: only acts from `Unknown` and when enabled (reference guard
    /// `guard enabled, state == .unknown else { return }`). Resolves the installed
    /// model via `resolver`, transitions `Unknown → NotInstalled | Preparing → Ready
    /// | Failed`.
    ///
    /// `resolver` is the model-location lookup (E11-S6 supplies the real models-dir
    /// probe; tests pass a closure). Returning `None` ⇒ `NotInstalled`.
    pub fn prepare(&mut self, resolver: impl FnOnce() -> Option<InstalledModel>) {
        if !self.enabled || self.state != ModelState::Unknown {
            return;
        }
        let Some(installed) = resolver() else {
            self.state = ModelState::NotInstalled;
            return;
        };
        self.state = ModelState::Preparing;
        self.load(&installed);
    }

    /// `download()` — fetch + install + warm + load. **STUB for E11-S6.**
    ///
    /// The reference streams the manifest files, verifies SHA256, installs under the
    /// models dir, then `load`s + warms + sweeps all coordinators. Network download is
    /// explicitly OUT OF SCOPE for E11-S1; E11-S6 implements `reqwest` streaming +
    /// `sha2` verify against [`crate::manifest::OnnxManifest`] and then calls
    /// [`Self::load`]. This stub only sets the `Downloading(0.0)` entry state so the
    /// state machine is wired; it performs NO network I/O.
    pub fn download_stub(&mut self) {
        match self.state {
            ModelState::Downloading(_) | ModelState::Preparing | ModelState::Ready => return,
            _ => {}
        }
        // E11-S6: replace with real fetch+verify+install → load(installed) → sweep.
        self.state = ModelState::Downloading(0.0);
    }

    /// Update download progress (0.0..=1.0). Used by E11-S6's progress callback; a
    /// no-op unless currently `Downloading`.
    pub fn set_download_progress(&mut self, fraction: f64) {
        if matches!(self.state, ModelState::Downloading(_)) {
            self.state = ModelState::Downloading(fraction.clamp(0.0, 1.0));
        }
    }

    /// Mark a failure (reference `state = .failed(...)`).
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.state = ModelState::Failed(reason.into());
    }

    // --- load path ---------------------------------------------------------------

    /// Load the installed encoders + tokenizer, warm with `encode("warm up")`, and
    /// transition to `Ready` (or `Failed`). The reference does this on a detached
    /// `userInitiated` task; the port keeps it synchronous here — E11-S6 owns the
    /// async scheduling.
    #[cfg(feature = "ort")]
    fn load(&mut self, installed: &InstalledModel) {
        match crate::embedder::VisualEmbedder::from_dir(&installed.dir) {
            Ok(mut embedder) => {
                // Warm the model exactly like the reference (`encode(text:"warm up")`).
                if let Err(e) = embedder.encode_text("warm up") {
                    self.state = ModelState::Failed(format!("warm up failed: {e}"));
                    return;
                }
                self.embedder = Some(embedder);
                self.state = ModelState::Ready;
            }
            Err(e) => {
                self.state = ModelState::Failed(format!("model load failed: {e}"));
            }
        }
    }

    /// Default-build `load`: no ONNX runtime compiled, so a "load" that reaches here
    /// can only confirm the files are present and then fail loudly — the real encode
    /// needs the `ort` feature. This keeps the state machine testable without weights.
    #[cfg(not(feature = "ort"))]
    fn load(&mut self, installed: &InstalledModel) {
        if installed.is_complete() {
            self.state = ModelState::Failed(
                "model files present but `ort` feature not enabled — rebuild with --features ort"
                    .into(),
            );
        } else {
            self.state = ModelState::Failed("installed model incomplete".into());
        }
    }

    /// The loaded embedder, if `Ready` (feature `ort`). E11-S6 borrows this for the
    /// query path; E11-S4 for indexing.
    #[cfg(feature = "ort")]
    pub fn embedder_mut(&mut self) -> Option<&mut crate::embedder::VisualEmbedder> {
        self.embedder.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_unknown_enabled() {
        let l = VisualModelLoader::default();
        assert_eq!(*l.state(), ModelState::Unknown);
        assert!(l.enabled());
        assert!(!l.is_ready());
    }

    #[test]
    fn prepare_with_no_model_is_not_installed() {
        let mut l = VisualModelLoader::new(true);
        l.prepare(|| None);
        assert_eq!(*l.state(), ModelState::NotInstalled);
    }

    #[test]
    fn prepare_is_noop_when_disabled() {
        let mut l = VisualModelLoader::new(false);
        l.prepare(|| None);
        assert_eq!(*l.state(), ModelState::Unknown);
    }

    #[test]
    fn prepare_is_idempotent_only_from_unknown() {
        let mut l = VisualModelLoader::new(true);
        l.prepare(|| None); // → NotInstalled
        assert_eq!(*l.state(), ModelState::NotInstalled);
        // A second prepare must not move from NotInstalled (guard: state==Unknown).
        l.prepare(|| Some(InstalledModel { dir: PathBuf::from("nope") }));
        assert_eq!(*l.state(), ModelState::NotInstalled);
    }

    #[test]
    fn download_stub_enters_downloading_and_progress_tracks() {
        let mut l = VisualModelLoader::new(true);
        l.download_stub();
        assert_eq!(*l.state(), ModelState::Downloading(0.0));
        l.set_download_progress(0.5);
        assert_eq!(*l.state(), ModelState::Downloading(0.5));
        // Clamp out-of-range.
        l.set_download_progress(2.0);
        assert_eq!(*l.state(), ModelState::Downloading(1.0));
        // No-progress no-op when not downloading.
        l.fail("x");
        l.set_download_progress(0.3);
        assert_eq!(*l.state(), ModelState::Failed("x".into()));
    }

    #[test]
    fn download_stub_noop_when_ready_or_preparing() {
        let mut l = VisualModelLoader::new(true);
        l.state = ModelState::Ready;
        l.download_stub();
        assert_eq!(*l.state(), ModelState::Ready);
        l.state = ModelState::Preparing;
        l.download_stub();
        assert_eq!(*l.state(), ModelState::Preparing);
    }

    #[test]
    fn disable_resets_ready_to_unknown() {
        let mut l = VisualModelLoader::new(true);
        l.state = ModelState::Ready;
        l.set_enabled(false);
        assert_eq!(*l.state(), ModelState::Unknown);
        assert!(!l.enabled());
    }

    #[test]
    fn installed_model_probe_requires_all_three() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(InstalledModel::probe(dir).is_none());
        std::fs::write(dir.join("vision_model.onnx"), b"x").unwrap();
        std::fs::write(dir.join("text_model.onnx"), b"x").unwrap();
        assert!(InstalledModel::probe(dir).is_none(), "missing tokenizer");
        std::fs::write(dir.join("tokenizer.json"), b"x").unwrap();
        assert!(InstalledModel::probe(dir).is_some());
    }
}
