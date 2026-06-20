//! `SampleProjectService` — list, resolve, and materialize Convex sample bundles
//! (story E1-S8).
//!
//! Ports `Project/SampleProjectService.swift`: fetch a sample catalog from Convex
//! over HTTP, resolve a slug to a full payload, and materialize it to a `.palmier`
//! bundle on disk (with download progress) using the **reference filenames**
//! (`project.json` / `media.json` / `generation-log.json` / `thumbnail.jpg` /
//! `media/` / `chat/`) so import does not break (ruling #3, FR-4). See
//! docs/reference/project-io.md "Sample materialization".
//!
//! ## Endpoints (reference)
//! - List:    `GET {convexHttpUrl}/v1/samples` → `[Summary { slug, title, posterUrl? }]`
//! - Resolve: `GET {convexHttpUrl}/v1/samples/resolve?slug=<slug>` →
//!   `{ title, project, manifest, generationLog?, posterUrl?,
//!      downloads:[{id, relativePath, url}], chat:[{name, url}] }`
//!
//! ## Materialization (reference `SampleProjectService` build)
//! Bundle at `cacheRoot/<safeSlug>/<safeTitle>.palmier` where
//! `cacheRoot = %APPDATA%\PalmierProWin\Samples` / `~/.config/palmier-pro/Samples`.
//! `safe_name` strips `/ : \`. The stale slug dir is cleared first, `media/` is
//! created, and the JSON sub-documents are written under the **reference
//! filenames**. Media downloads use the server `relativePath` AS-IS (already
//! `media/<file>`); chat entries get `relativePath = "chat/<name>"`. All downloads
//! run **concurrently**; on any failure the whole slug dir is removed and the error
//! surfaced. `cached_url(slug)` returns the first `*.palmier` in the slug dir to
//! skip re-download.
//!
//! ## Offline degradation (OQ-9 / R-4)
//! Every network call returns a `Result`; the caller (the Tauri command) maps a
//! list failure to an **empty carousel** rather than an app error. The transport is
//! behind the [`SampleBackend`] trait so the materialization + progress logic is
//! unit-tested without a live Convex (a fixture backend serves captured payloads —
//! mirrors the spec's "captured fixture `/v1/samples/resolve` payload" fallback).
//!
//! ## Concurrency choice (decision)
//! The reference uses a Swift `withThrowingTaskGroup`; the spec suggests a
//! `tokio JoinSet`. To keep `palmier-project` **runtime-agnostic** (it has no async
//! runtime today, and adding `tokio` here would pull a heavy dep into the project
//! crate) the downloads run concurrently on `std::thread`s via a scoped pool,
//! matching `palmier-auth`'s blocking-`reqwest` precedent. The observable behavior
//! — concurrent downloads, `completed/total` progress, all-or-nothing cleanup — is
//! identical.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::bundle::project;

/// A sample summary for the Home carousel (reference `Summary`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleSummary {
    /// Stable slug used by `resolve`.
    pub slug: String,
    /// Display title.
    pub title: String,
    /// Optional poster image URL (carousel thumbnail).
    #[serde(rename = "posterUrl", skip_serializing_if = "Option::is_none", default)]
    pub poster_url: Option<String>,
}

/// One downloadable media file in a resolved sample (reference `downloads[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleDownload {
    /// Opaque id (unused by materialization beyond diagnostics).
    #[serde(default)]
    pub id: String,
    /// Path under the bundle (already `media/<file>` for media entries).
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    /// Where to fetch the bytes.
    pub url: String,
}

/// One chat session file in a resolved sample (reference `chat[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleChat {
    /// Session file name (becomes `chat/<name>`).
    pub name: String,
    /// Where to fetch the bytes.
    pub url: String,
}

/// The full resolved sample payload (reference `/v1/samples/resolve`).
///
/// `project` / `manifest` / `generation_log` are kept as raw [`serde_json::Value`]
/// and written verbatim to `project.json` / `media.json` / `generation-log.json`.
/// Keeping them opaque means materialization does NOT depend on the exact
/// `Timeline` / `MediaManifest` serde shapes (Epic 2 owns those; the round-trip
/// fidelity gate is S-1b) — the bytes the server sends are the bytes we persist,
/// and `read_bundle` decodes them on open.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolvedSample {
    /// Display title (drives the `<safeTitle>.palmier` filename).
    pub title: String,
    /// The `Timeline` JSON → `project.json` (required).
    pub project: serde_json::Value,
    /// The `MediaManifest` JSON → `media.json`.
    pub manifest: serde_json::Value,
    /// Optional `GenerationLog` JSON → `generation-log.json`.
    #[serde(rename = "generationLog", default)]
    pub generation_log: Option<serde_json::Value>,
    /// Optional poster URL → `thumbnail.jpg`.
    #[serde(rename = "posterUrl", default)]
    pub poster_url: Option<String>,
    /// Media files to download (relative paths already `media/<file>`).
    #[serde(default)]
    pub downloads: Vec<SampleDownload>,
    /// Chat session files to download (named `chat/<name>`).
    #[serde(default)]
    pub chat: Vec<SampleChat>,
}

/// An error from the sample service.
#[derive(Debug)]
pub enum SampleError {
    /// A transport / HTTP error (offline, non-2xx, decode). The caller degrades a
    /// **list** failure to an empty carousel; a **resolve/materialize** failure is
    /// surfaced to the UI.
    Network(String),
    /// A filesystem error while materializing the bundle.
    Io(std::io::Error),
}

impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SampleError::Network(m) => write!(f, "sample network error: {m}"),
            SampleError::Io(e) => write!(f, "sample io error: {e}"),
        }
    }
}

impl std::error::Error for SampleError {}

impl From<std::io::Error> for SampleError {
    fn from(e: std::io::Error) -> Self {
        SampleError::Io(e)
    }
}

/// A `Result` over [`SampleError`].
pub type Result<T> = std::result::Result<T, SampleError>;

/// The Convex transport the sample service needs. Implemented by
/// [`HttpSampleBackend`] (real `reqwest`) and a fixture backend in tests, so the
/// materialization + progress logic is exercised without a live backend (and so
/// the build-time "Convex blocked → captured fixture" fallback has a seam).
pub trait SampleBackend: Send + Sync {
    /// `GET /v1/samples` → the carousel summaries.
    fn list(&self) -> Result<Vec<SampleSummary>>;
    /// `GET /v1/samples/resolve?slug=<slug>` → the full payload.
    fn resolve(&self, slug: &str) -> Result<ResolvedSample>;
    /// Download the bytes at `url` (a media/chat/poster file).
    fn download(&self, url: &str) -> Result<Vec<u8>>;
}

/// Real Convex HTTP backend over blocking `reqwest` (mirrors
/// `palmier_auth::HttpConvexBackend`'s transport choice: blocking keeps
/// `palmier-project` runtime-agnostic).
#[derive(Clone)]
pub struct HttpSampleBackend {
    http_base: String,
    client: reqwest::blocking::Client,
}

impl HttpSampleBackend {
    /// Construct against the Convex HTTP base URL (`PALMIER_CONVEX_HTTP_URL`).
    pub fn new(http_base: impl Into<String>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .map_err(|e| SampleError::Network(e.to_string()))?;
        Ok(Self {
            http_base: http_base.into(),
            client,
        })
    }

    fn join(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.http_base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .client
            .get(url)
            .send()
            .map_err(|e| SampleError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SampleError::Network(format!("status {}", resp.status())));
        }
        Ok(resp
            .bytes()
            .map_err(|e| SampleError::Network(e.to_string()))?
            .to_vec())
    }
}

impl SampleBackend for HttpSampleBackend {
    fn list(&self) -> Result<Vec<SampleSummary>> {
        let url = self.join("v1/samples");
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| SampleError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SampleError::Network(format!("status {}", resp.status())));
        }
        resp.json::<Vec<SampleSummary>>()
            .map_err(|e| SampleError::Network(e.to_string()))
    }

    fn resolve(&self, slug: &str) -> Result<ResolvedSample> {
        let url = format!("{}?slug={}", self.join("v1/samples/resolve"), urlencode(slug));
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| SampleError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SampleError::Network(format!("status {}", resp.status())));
        }
        resp.json::<ResolvedSample>()
            .map_err(|e| SampleError::Network(e.to_string()))
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        self.get_bytes(url)
    }
}

/// Minimal percent-encoding for the `slug` query param (alnum / `-_.~` pass
/// through; everything else is `%XX`). Avoids pulling a urlencoding dep for one
/// param.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The sample service: lists, resolves, and materializes sample bundles into a
/// cache root. Holds a [`SampleBackend`] (real HTTP or a fixture).
pub struct SampleProjectService {
    backend: Box<dyn SampleBackend>,
    cache_root: PathBuf,
}

impl SampleProjectService {
    /// Build a service over `backend`, caching bundles under `cache_root`
    /// (`%APPDATA%\PalmierProWin\Samples` / `~/.config/palmier-pro/Samples`).
    pub fn new(backend: Box<dyn SampleBackend>, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            cache_root: cache_root.into(),
        }
    }

    /// The default sample cache root next to the registry/settings dir.
    pub fn default_cache_root() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        #[cfg(windows)]
        let dir = base.join("PalmierProWin").join("Samples");
        #[cfg(not(windows))]
        let dir = base.join("palmier-pro").join("Samples");
        Some(dir)
    }

    /// `GET /v1/samples` for the carousel. A network failure is returned to the
    /// caller, which degrades it to an **empty carousel** (OQ-9 / R-4) — this
    /// method does not itself swallow the error so the caller can log it.
    pub fn list(&self) -> Result<Vec<SampleSummary>> {
        self.backend.list()
    }

    /// The slug dir under the cache root (`<cacheRoot>/<safeSlug>`).
    fn slug_dir(&self, slug: &str) -> PathBuf {
        self.cache_root.join(safe_name(slug))
    }

    /// Return the first `*.palmier` bundle already materialized for `slug`, to skip
    /// re-download (reference `cachedURL`). `None` if the slug dir has none.
    pub fn cached_url(&self, slug: &str) -> Option<PathBuf> {
        let dir = self.slug_dir(slug);
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some(project::FILE_EXTENSION) {
                return Some(p);
            }
        }
        None
    }

    /// Resolve + materialize a sample bundle for `slug`, reporting download
    /// progress as `completed/total` in `0.0..=1.0` via `on_progress`.
    ///
    /// Returns the path to the materialized `.palmier` bundle. If a bundle is
    /// already cached for the slug, it is returned immediately (no re-download).
    /// On any failure the whole slug dir is removed and the error surfaced
    /// (reference "any failure → remove whole slug dir, rethrow").
    pub fn materialize<F: Fn(f64) + Sync>(&self, slug: &str, on_progress: F) -> Result<PathBuf> {
        if let Some(cached) = self.cached_url(slug) {
            on_progress(1.0);
            return Ok(cached);
        }

        let resolved = self.backend.resolve(slug)?;
        match self.build_bundle(slug, &resolved, &on_progress) {
            Ok(path) => Ok(path),
            Err(e) => {
                // Any failure removes the whole slug dir (reference cleanup).
                let _ = std::fs::remove_dir_all(self.slug_dir(slug));
                Err(e)
            }
        }
    }

    /// Build the `.palmier` bundle for a resolved sample. Separated so
    /// [`materialize`](Self::materialize) can wrap it in the all-or-nothing
    /// cleanup.
    fn build_bundle<F: Fn(f64) + Sync>(
        &self,
        slug: &str,
        resolved: &ResolvedSample,
        on_progress: &F,
    ) -> Result<PathBuf> {
        let slug_dir = self.slug_dir(slug);
        // Clear any stale slug dir first (reference `removeItem`).
        let _ = std::fs::remove_dir_all(&slug_dir);
        std::fs::create_dir_all(&slug_dir)?;

        let bundle = slug_dir.join(format!(
            "{}.{}",
            safe_name(&resolved.title),
            project::FILE_EXTENSION
        ));
        std::fs::create_dir_all(&bundle)?;
        std::fs::create_dir_all(bundle.join(project::MEDIA_DIR))?;

        // Write the JSON sub-documents under the REFERENCE filenames (ruling #3).
        write_json(&bundle.join(project::TIMELINE_FILE), &resolved.project)?;
        write_json(&bundle.join(project::MANIFEST_FILE), &resolved.manifest)?;
        if let Some(log) = &resolved.generation_log {
            write_json(&bundle.join(project::GENERATION_LOG_FILE), log)?;
        }

        // Assemble the download list: media entries use server relativePath AS-IS;
        // chat entries get relativePath = "chat/<name>"; the poster (if any) →
        // thumbnail.jpg.
        let mut jobs: Vec<(String, String)> = Vec::new(); // (relative_path, url)
        for d in &resolved.downloads {
            jobs.push((d.relative_path.clone(), d.url.clone()));
        }
        for c in &resolved.chat {
            jobs.push((format!("{}/{}", project::CHAT_DIR, c.name), c.url.clone()));
        }
        if let Some(poster) = &resolved.poster_url {
            jobs.push((project::THUMBNAIL_FILE.to_string(), poster.clone()));
        }

        self.download_all(&bundle, &jobs, on_progress)?;
        Ok(bundle)
    }

    /// Download every `(relative_path, url)` job CONCURRENTLY into `bundle`,
    /// reporting `completed/total` progress. All-or-nothing: the first error is
    /// returned (the caller removes the slug dir).
    fn download_all<F: Fn(f64) + Sync>(
        &self,
        bundle: &Path,
        jobs: &[(String, String)],
        on_progress: &F,
    ) -> Result<()> {
        let total = jobs.len();
        if total == 0 {
            on_progress(1.0);
            return Ok(());
        }

        let completed = Arc::new(AtomicUsize::new(0));
        let backend = &self.backend;

        // Scoped threads: borrow `backend`, `bundle`, and `on_progress` without
        // `'static` bounds. Each thread downloads one file and writes it; every
        // outcome is collected and the first error returned after the join.
        let results: std::sync::Mutex<Vec<Result<()>>> = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(total);
            for (rel, url) in jobs {
                let completed = Arc::clone(&completed);
                let results = &results;
                handles.push(scope.spawn(move || {
                    let outcome = (|| -> Result<()> {
                        let bytes = backend.download(url)?;
                        let dest = bundle.join(rel);
                        if let Some(parent) = dest.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&dest, &bytes)?;
                        Ok(())
                    })();
                    if outcome.is_ok() {
                        let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        on_progress(done as f64 / total as f64);
                    }
                    results.lock().expect("results mutex").push(outcome);
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        });

        // Surface the first error, if any.
        let collected = results.into_inner().expect("results mutex");
        for r in collected {
            r?;
        }
        Ok(())
    }
}

/// Strip the path-unsafe characters `/ : \` from a name (reference `safeName`).
pub fn safe_name(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '/' | ':' | '\\'))
        .collect()
}

/// Write a `serde_json::Value` to `path` (compact, matching the reference
/// default-encoder; the server already sent the canonical bytes — we persist them
/// faithfully).
fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| SampleError::Network(e.to_string()))?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// A fixture-backed [`SampleBackend`] for tests and the build-time "Convex blocked
/// → captured fixture" fallback: serves in-memory summaries / payloads / file
/// bytes without any network.
#[derive(Default)]
pub struct FixtureSampleBackend {
    pub summaries: Vec<SampleSummary>,
    pub resolved: HashMap<String, ResolvedSample>,
    pub files: HashMap<String, Vec<u8>>,
    /// When true, `list` fails (simulating offline) so the caller's empty-carousel
    /// degradation can be tested.
    pub offline: bool,
}

impl SampleBackend for FixtureSampleBackend {
    fn list(&self) -> Result<Vec<SampleSummary>> {
        if self.offline {
            return Err(SampleError::Network("offline".into()));
        }
        Ok(self.summaries.clone())
    }

    fn resolve(&self, slug: &str) -> Result<ResolvedSample> {
        self.resolved
            .get(slug)
            .cloned()
            .ok_or_else(|| SampleError::Network(format!("no fixture for slug {slug}")))
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| SampleError::Network(format!("no fixture file at {url}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e1s8-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn fixture_resolved() -> ResolvedSample {
        ResolvedSample {
            title: "My / Sample : Project".into(),
            project: serde_json::json!({ "fps": 30, "width": 1920, "height": 1080, "tracks": [] }),
            manifest: serde_json::json!({ "version": 1, "entries": [], "folders": [] }),
            generation_log: Some(serde_json::json!({ "version": 1, "entries": [] })),
            poster_url: Some("https://cdn.example/poster.jpg".into()),
            downloads: vec![SampleDownload {
                id: "m1".into(),
                relative_path: "media/clip.mov".into(),
                url: "https://cdn.example/clip.mov".into(),
            }],
            chat: vec![SampleChat {
                name: "session-a.json".into(),
                url: "https://cdn.example/session-a.json".into(),
            }],
        }
    }

    fn fixture_backend() -> FixtureSampleBackend {
        let mut files = HashMap::new();
        files.insert(
            "https://cdn.example/clip.mov".to_string(),
            b"\x00fake-mov".to_vec(),
        );
        files.insert(
            "https://cdn.example/session-a.json".to_string(),
            br#"{"id":"a"}"#.to_vec(),
        );
        files.insert(
            "https://cdn.example/poster.jpg".to_string(),
            b"\xFF\xD8jpeg".to_vec(),
        );
        let mut resolved = HashMap::new();
        resolved.insert("hero".to_string(), fixture_resolved());
        FixtureSampleBackend {
            summaries: vec![SampleSummary {
                slug: "hero".into(),
                title: "Hero".into(),
                poster_url: Some("https://cdn.example/poster.jpg".into()),
            }],
            resolved,
            files,
            offline: false,
        }
    }

    #[test]
    fn list_returns_summaries() {
        let svc = SampleProjectService::new(Box::new(fixture_backend()), scratch());
        let s = svc.list().unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].slug, "hero");
    }

    #[test]
    fn offline_list_errors_so_caller_can_degrade() {
        let mut b = fixture_backend();
        b.offline = true;
        let svc = SampleProjectService::new(Box::new(b), scratch());
        assert!(
            svc.list().is_err(),
            "offline list errors; caller shows empty carousel"
        );
    }

    #[test]
    fn materialize_writes_reference_filenames_and_downloads() {
        let root = scratch();
        let svc = SampleProjectService::new(Box::new(fixture_backend()), &root);

        let progress = AtomicUsize::new(0);
        let bundle = svc
            .materialize("hero", |p| {
                if (p - 1.0).abs() < f64::EPSILON {
                    progress.store(100, Ordering::SeqCst);
                }
            })
            .unwrap();

        // Bundle named from the safe title (/ : \ stripped).
        assert!(
            bundle.ends_with("My  Sample  Project.palmier"),
            "got {bundle:?}"
        );
        // Reference filenames present.
        assert!(bundle.join("project.json").is_file());
        assert!(bundle.join("media.json").is_file());
        assert!(bundle.join("generation-log.json").is_file());
        assert!(bundle.join("thumbnail.jpg").is_file());
        // Media kept under server relativePath AS-IS.
        assert!(bundle.join("media").join("clip.mov").is_file());
        // Chat under chat/<name>.
        assert!(bundle.join("chat").join("session-a.json").is_file());
        // Progress reached 100%.
        assert_eq!(progress.load(Ordering::SeqCst), 100);
        // The materialized project.json round-trips through the bundle reader.
        let loaded = crate::bundle::read_bundle(&bundle).unwrap();
        assert_eq!(loaded.timeline.fps, 30);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cached_url_skips_redownload() {
        let root = scratch();
        let svc = SampleProjectService::new(Box::new(fixture_backend()), &root);
        let first = svc.materialize("hero", |_| {}).unwrap();
        // A second materialize returns the cached bundle (same path), no re-resolve.
        let second = svc.materialize("hero", |_| {}).unwrap();
        assert_eq!(first, second);
        assert!(svc.cached_url("hero").is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failed_download_removes_slug_dir() {
        let root = scratch();
        // Backend resolves but is missing the media file → download fails.
        let mut b = fixture_backend();
        b.files.remove("https://cdn.example/clip.mov");
        let svc = SampleProjectService::new(Box::new(b), &root);

        assert!(svc.materialize("hero", |_| {}).is_err());
        // The whole slug dir is gone (all-or-nothing cleanup).
        assert!(
            !root.join("hero").exists(),
            "failed materialize must remove the slug dir"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn safe_name_strips_path_chars() {
        assert_eq!(safe_name("a/b:c\\d"), "abcd");
    }
}
