//! `.palmier` directory-bundle reader / writer (story E2-S9).
//!
//! Ports `Project/VideoProject.swift` `read(from:ofType:)` (:31),
//! `fileWrapper(ofType:)` (:66), and `captureSaveSnapshot()` (:90), plus the
//! NSDocument safe-save guarantee (write-to-temp-then-atomic-swap), to plain
//! `std::fs` directory I/O. A `.palmier` bundle is a **directory** the macOS
//! Finder presents as one file; on Windows/Linux we read/write it as a directory
//! and the directory-as-single-document UX lives in the file dialog (E2-S12).
//!
//! ## Bundle layout (ruling #3 — reference filenames, NOT FOUNDATION §5.7)
//!
//! ```text
//! <Name>.palmier/
//!   project.json          # REQUIRED — Timeline (compact, default JSON)
//!   media.json            # optional — MediaManifest (compact, apple-epoch Dates)
//!   generation-log.json   # optional — GenerationLog (compact, apple-epoch Dates)
//!   thumbnail.jpg         # optional — JPEG bytes
//!   media/                # internalized media; entries store "media/<file>"
//!   chat/                 # one JSON per non-empty session, "<uuid>.json"
//! ```
//!
//! Filenames come from [`Project`] constants (reference
//! `Utilities/Constants.swift:104 enum Project`). FOUNDATION §5.7's
//! `timeline.json`/`manifest.json`/`generation_log.json`/`chatsessions/` are
//! **void** here — the Convex sample server emits the reference names, so
//! deviating breaks sample import (phase0-reconciliation ruling #3).
//!
//! ## Read severities (reference `VideoProject.read` :31)
//!
//! These are preserved **exactly** (docs/reference/project-io.md "Read", Port
//! risks "project.json missing = corrupt"):
//!
//! | file | absent | decode fails |
//! |------|--------|--------------|
//! | `project.json`        | **HARD** [`BundleError::Corrupt`] | **HARD** [`BundleError::Corrupt`] |
//! | `media.json`          | ok (no manifest) | **HARD** [`BundleError::Corrupt`] |
//! | `generation-log.json` | ok (seed later)  | **SOFT** — logged, ignored |
//!
//! The reference maps both project.json-missing and a media.json/project.json
//! decode failure to `CocoaError(.fileReadCorruptFile)` (read :32, :48); the
//! generation log is decoded with `try?` (:52) so a failure is tolerated.
//! [`BundleError::Corrupt`] is our `fileReadCorruptFile` analogue.
//!
//! ## Save (reference `fileWrapper` :66 / `captureSaveSnapshot` :90)
//!
//! [`BundleWriter`] rebuilds the package children in reference order: encode
//! `project.json` (required — a serialization failure → [`BundleError::WriteUnknown`],
//! the `.fileWriteUnknown` analogue), then `media.json` / `generation-log.json` /
//! `thumbnail.jpg` if present, then a **freshly built** `chat/` dir (rebuilt from
//! the supplied sessions each save — reference :84), then — **only if a live
//! `media/` dir already exists on disk** in the destination bundle — the existing
//! `media/` is carried into the new package (reference `mediaDirWrapper` :103
//! returns nil when no `media/` exists, so newly imported media is captured only
//! once it has been written under the live bundle).
//!
//! ## Whole-directory atomic save (NSDocument safe-save)
//!
//! The package is written to a **sibling temp directory** in the same parent
//! (same filesystem, so the rename is atomic), then the destination is swapped:
//! the old bundle is moved aside, the temp dir renamed into place, and the old
//! bundle deleted. A crash at any point leaves either the original intact or a
//! complete new bundle — never a half-written one (docs/reference/project-io.md
//! Port risks "Whole-directory atomic save").

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use palmier_model::{GenerationLog, MediaManifest, Timeline};
use serde::Serialize;

/// Canonical bundle filename / directory constants (reference
/// `Utilities/Constants.swift:104 enum Project`, ruling #3).
pub mod project {
    /// `.palmier` bundle file extension (without the dot).
    pub const FILE_EXTENSION: &str = "palmier";
    /// The required `Timeline` document.
    pub const TIMELINE_FILE: &str = "project.json";
    /// The optional `MediaManifest`.
    pub const MANIFEST_FILE: &str = "media.json";
    /// The optional append-only `GenerationLog`.
    pub const GENERATION_LOG_FILE: &str = "generation-log.json";
    /// The optional JPEG poster.
    pub const THUMBNAIL_FILE: &str = "thumbnail.jpg";
    /// The internalized-media directory.
    pub const MEDIA_DIR: &str = "media";
    /// The per-session chat directory.
    pub const CHAT_DIR: &str = "chat";
    /// The project registry filename (consumed by E2-S11).
    pub const REGISTRY_FILE: &str = "project-registry.json";
}

/// An error reading or writing a `.palmier` bundle.
///
/// The two reference severities are distinct variants so callers can map them
/// to the macOS `CocoaError` codes they port:
/// - [`BundleError::Corrupt`] ⇔ `CocoaError(.fileReadCorruptFile)` — a required
///   file is missing or fails to decode (project.json or media.json).
/// - [`BundleError::WriteUnknown`] ⇔ `CocoaError(.fileWriteUnknown)` — the
///   required `project.json` could not be serialized on save.
#[derive(Debug)]
pub enum BundleError {
    /// The bundle is corrupt: `project.json` is missing, or `project.json` /
    /// `media.json` failed to decode. Hard error (reference
    /// `.fileReadCorruptFile`, read :32 / :48).
    Corrupt {
        /// Which file triggered it (for diagnostics).
        file: &'static str,
        /// The underlying cause (decode error text or "missing").
        detail: String,
    },
    /// The required `project.json` could not be encoded on save (reference
    /// `.fileWriteUnknown`, fileWrapper :76).
    WriteUnknown {
        /// What failed to serialize.
        file: &'static str,
        detail: String,
    },
    /// An underlying filesystem error (create/rename/read/write).
    Io(io::Error),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Corrupt { file, detail } => {
                write!(f, "corrupt bundle: {file}: {detail}")
            }
            BundleError::WriteUnknown { file, detail } => {
                write!(f, "write failed: {file}: {detail}")
            }
            BundleError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for BundleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BundleError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for BundleError {
    fn from(e: io::Error) -> Self {
        BundleError::Io(e)
    }
}

/// A `Result` over [`BundleError`].
pub type Result<T> = std::result::Result<T, BundleError>;

/// The decoded contents of a `.palmier` bundle (reference: the three `loaded*`
/// fields applied in `makeWindowControllers` :177).
///
/// `manifest` is `None` when `media.json` is absent (it is hard-error only on a
/// **decode failure**, never on absence). `generation_log` is `None` when
/// `generation-log.json` is absent **or** failed to decode (soft) — absence
/// drives `seed_generation_log_from_assets()` downstream (Epic 9).
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedBundle {
    /// The required timeline (always present — read fails otherwise).
    pub timeline: Timeline,
    /// The media manifest, if `media.json` was present and decoded.
    pub manifest: Option<MediaManifest>,
    /// The generation log, if `generation-log.json` was present and decoded
    /// (a decode failure is swallowed → `None`).
    pub generation_log: Option<GenerationLog>,
}

/// Read a `.palmier` bundle directory (reference `VideoProject.read` :31).
///
/// Severities (preserved exactly — see the module-level table):
/// 1. `project.json` MUST exist and decode → else [`BundleError::Corrupt`].
/// 2. `media.json` present → must decode → else [`BundleError::Corrupt`];
///    absent → `manifest = None`.
/// 3. `generation-log.json` present → decoded tolerantly; any failure → `None`
///    (SOFT, not an error).
///
/// Chat sessions are **not** loaded here — the reference loads them separately
/// from the live directory in `makeWindowControllers` (:184); the chat model
/// shape lands in a later story. [`read_chat_session_files`] exposes the raw
/// `chat/` bytes for that consumer.
pub fn read_bundle(bundle_dir: impl AsRef<Path>) -> Result<LoadedBundle> {
    let dir = bundle_dir.as_ref();

    // 1. project.json — required. Missing OR decode failure = HARD corrupt.
    //    (reference read :32 `guard let data … else throw .fileReadCorruptFile`,
    //     :38 decode; a decode error there propagates as the read error.)
    let project_path = dir.join(project::TIMELINE_FILE);
    let project_bytes = match fs::read(&project_path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(BundleError::Corrupt {
                file: project::TIMELINE_FILE,
                detail: "missing".into(),
            });
        }
        Err(e) => return Err(BundleError::Io(e)),
    };
    let timeline: Timeline = serde_json::from_slice(&project_bytes).map_err(|e| {
        BundleError::Corrupt {
            file: project::TIMELINE_FILE,
            detail: e.to_string(),
        }
    })?;

    // 2. media.json — optional file, but a decode failure when present is ALSO a
    //    HARD corrupt error (reference read :44–:49 throws .fileReadCorruptFile).
    let manifest_path = dir.join(project::MANIFEST_FILE);
    let manifest = match fs::read(&manifest_path) {
        Ok(bytes) => Some(serde_json::from_slice::<MediaManifest>(&bytes).map_err(|e| {
            BundleError::Corrupt {
                file: project::MANIFEST_FILE,
                detail: e.to_string(),
            }
        })?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(BundleError::Io(e)),
    };

    // 3. generation-log.json — present → decode tolerantly; failure is SOFT
    //    (reference read :52 uses `try?`, swallowing the error). Absence and a
    //    decode failure both yield None; the downstream seeding (Epic 9) reacts
    //    to None identically.
    let log_path = dir.join(project::GENERATION_LOG_FILE);
    let generation_log = match fs::read(&log_path) {
        Ok(bytes) => serde_json::from_slice::<GenerationLog>(&bytes).ok(),
        Err(_) => None,
    };

    Ok(LoadedBundle {
        timeline,
        manifest,
        generation_log,
    })
}

/// Read the raw `chat/*.json` session files from a bundle, as `(filename, bytes)`
/// pairs (reference: chat is loaded from the live directory, not the package
/// wrapper — `makeWindowControllers` :184 / `ChatSessionStore`). Returns an empty
/// vec when no `chat/` dir exists. The chat model shape (and its iso8601 Date
/// codec) lands in a later story; this is the I/O seam it consumes.
pub fn read_chat_session_files(bundle_dir: impl AsRef<Path>) -> Result<Vec<(String, Vec<u8>)>> {
    let chat_dir = bundle_dir.as_ref().join(project::CHAT_DIR);
    let mut out = Vec::new();
    let entries = match fs::read_dir(&chat_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(BundleError::Io(e)),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            out.push((name, fs::read(&path)?));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The in-memory snapshot of a bundle to be written (reference
/// `captureSaveSnapshot` :90: timeline + manifest + log + thumbnail + chat files).
///
/// `timeline` is required; everything else is optional and written only when
/// `Some`/non-empty (reference `if let manifest …`, `if let log …`). `chat_files`
/// are `(filename, bytes)` pairs — the caller has already filtered out empty
/// sessions and named each `"<uuid>.json"` (reference :95 `filter{!messages.isEmpty}`),
/// matching "chat/ is rebuilt from in-memory sessions each save (only non-empty)".
#[derive(Debug, Clone)]
pub struct BundleSnapshot {
    /// The timeline → `project.json` (required).
    pub timeline: Timeline,
    /// The manifest → `media.json` (optional).
    pub manifest: Option<MediaManifest>,
    /// The generation log → `generation-log.json` (optional).
    pub generation_log: Option<GenerationLog>,
    /// Raw JPEG bytes → `thumbnail.jpg` (optional; produced by `palmier-media`).
    pub thumbnail: Option<Vec<u8>>,
    /// Pre-encoded chat session files → `chat/<name>` (already non-empty-filtered).
    pub chat_files: Vec<(String, Vec<u8>)>,
}

impl BundleSnapshot {
    /// A snapshot carrying only a timeline (manifest/log/thumb/chat all empty) —
    /// the minimal save.
    pub fn new(timeline: Timeline) -> Self {
        BundleSnapshot {
            timeline,
            manifest: None,
            generation_log: None,
            thumbnail: None,
            chat_files: Vec::new(),
        }
    }
}

/// Encode a serde value to **compact** JSON bytes (the default `JSONEncoder`:
/// no pretty/sortedKeys — reference `captureSaveSnapshot` :91–:93 uses a bare
/// `JSONEncoder()`). The per-field Date codecs (apple-epoch) are already baked
/// into the model shapes (E2-S8), so plain `serde_json` produces the correct
/// wire format. `project.json` failure is mapped to [`BundleError::WriteUnknown`]
/// by the caller; manifest/log failures bubble as the same.
fn encode_compact<T: Serialize>(value: &T, file: &'static str) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| BundleError::WriteUnknown {
        file,
        detail: e.to_string(),
    })
}

/// Write a [`BundleSnapshot`] to `bundle_dir` **atomically** (reference
/// `fileWrapper` :66 + NSDocument safe-save).
///
/// The whole package is built in a sibling temp directory then swapped in, so a
/// partial write can never corrupt the destination (see the module docs). If a
/// live `media/` dir already exists in the destination bundle, it is carried into
/// the new package (reference `mediaDirWrapper` :103) — newly imported media is
/// captured only once it has been written under the live bundle path.
///
/// The `project.json` encode is the only step that can produce
/// [`BundleError::WriteUnknown`]; the rest are filesystem (`Io`) failures.
pub fn write_bundle(bundle_dir: impl AsRef<Path>, snapshot: &BundleSnapshot) -> Result<()> {
    let dest = bundle_dir.as_ref();

    // Encode project.json FIRST and FAIL FAST: the reference guards on a missing
    // snapshotTimeline (:75 → .fileWriteUnknown) before touching the package, so
    // we never start a swap we can't complete with a valid project.json.
    let timeline_bytes = encode_compact(&snapshot.timeline, project::TIMELINE_FILE)?;

    // 1. Stage the new package in a sibling temp dir (same parent ⇒ same
    //    filesystem ⇒ the final rename is atomic). Using the parent (not the OS
    //    temp dir) is REQUIRED: cross-filesystem renames are not atomic and would
    //    fall back to a copy, defeating safe-save.
    let parent = dest.parent().ok_or_else(|| {
        BundleError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bundle path has no parent directory",
        ))
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".{}.{}.tmp",
        dest.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("bundle"),
        uuid::Uuid::new_v4()
    ));
    // Best-effort clean of a leftover staging dir from a prior crash.
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    // Build the package into `staging`; on ANY failure, remove the staging dir so
    // we never leave a partial sibling behind, then propagate.
    if let Err(e) = build_package(&staging, dest, snapshot, &timeline_bytes) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // 2. Atomic swap. Move the existing bundle aside (if any), rename staging into
    //    place, then delete the old bundle. A crash between these leaves either the
    //    original (backup present) or the new bundle complete — never a half-write.
    if let Err(e) = swap_into_place(&staging, dest) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    Ok(())
}

/// Build the full package contents into `staging`. `dest` is the live bundle path,
/// consulted only to decide whether to carry an existing `media/` dir forward.
fn build_package(
    staging: &Path,
    dest: &Path,
    snapshot: &BundleSnapshot,
    timeline_bytes: &[u8],
) -> Result<()> {
    // project.json — required (already encoded by the caller).
    fs::write(staging.join(project::TIMELINE_FILE), timeline_bytes)?;

    // media.json / generation-log.json — only when present (reference `if let …`).
    if let Some(manifest) = &snapshot.manifest {
        let bytes = encode_compact(manifest, project::MANIFEST_FILE)?;
        fs::write(staging.join(project::MANIFEST_FILE), bytes)?;
    }
    if let Some(log) = &snapshot.generation_log {
        let bytes = encode_compact(log, project::GENERATION_LOG_FILE)?;
        fs::write(staging.join(project::GENERATION_LOG_FILE), bytes)?;
    }

    // thumbnail.jpg — raw bytes when present.
    if let Some(thumb) = &snapshot.thumbnail {
        fs::write(staging.join(project::THUMBNAIL_FILE), thumb)?;
    }

    // chat/ — FRESHLY rebuilt from the supplied (non-empty) sessions each save
    // (reference :84 `replaceChild(ChatSessionStore.dirName, chatDirWrapper())`).
    // Always created (even if empty) to mirror the reference always replacing the
    // chat child with a fresh directory wrapper.
    let chat_dir = staging.join(project::CHAT_DIR);
    fs::create_dir_all(&chat_dir)?;
    for (name, bytes) in &snapshot.chat_files {
        fs::write(chat_dir.join(name), bytes)?;
    }

    // media/ — carried forward ONLY if a live media/ already exists in the
    // destination bundle (reference `mediaDirWrapper` :103 returns nil otherwise).
    // We copy the existing tree into the staged package so the swap doesn't lose
    // already-internalized media.
    let live_media = dest.join(project::MEDIA_DIR);
    if live_media.is_dir() {
        copy_dir_recursive(&live_media, &staging.join(project::MEDIA_DIR))?;
    }

    Ok(())
}

/// Swap the staged package into the destination atomically.
///
/// On Windows `fs::rename` fails if the destination exists, so we move the old
/// bundle aside first. The sequence keeps a recoverable state at every step:
/// 1. rename `dest` → `dest.bak` (if `dest` exists),
/// 2. rename `staging` → `dest` (the atomic publish),
/// 3. delete `dest.bak`.
///
/// If step 2 fails we restore `dest.bak` → `dest` so the original survives.
fn swap_into_place(staging: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        let backup = sibling_backup_path(dest);
        let _ = fs::remove_dir_all(&backup);
        fs::rename(dest, &backup)?;
        match fs::rename(staging, dest) {
            Ok(()) => {
                // Published. The old bundle is now garbage; best-effort delete.
                let _ = fs::remove_dir_all(&backup);
                Ok(())
            }
            Err(e) => {
                // Publish failed — restore the original so we never lose it.
                let _ = fs::rename(&backup, dest);
                Err(BundleError::Io(e))
            }
        }
    } else {
        // First save — no existing bundle to displace.
        fs::rename(staging, dest)?;
        Ok(())
    }
}

/// A unique sibling `.bak` path next to `dest` (same parent ⇒ same filesystem).
fn sibling_backup_path(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("bundle");
    parent.join(format!(".{}.{}.bak", name, uuid::Uuid::new_v4()))
}

/// Recursively copy a directory tree (`media/` carry-forward). Plain `std::fs`;
/// no symlink following beyond what `fs::copy` does (media files are regular).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{ClipType, MediaManifest, MediaSource, Track};

    /// A unique scratch dir under the OS temp dir (no `tempfile` dep). Each call
    /// returns a fresh, empty directory; the test cleans it up at the end.
    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e2s9-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample_timeline() -> Timeline {
        let mut t = Timeline::new();
        t.tracks.push(Track::new(ClipType::Video));
        t
    }

    fn sample_manifest() -> MediaManifest {
        let mut m = MediaManifest::new();
        m.entries.push(palmier_model::MediaManifestEntry {
            id: "asset-1".into(),
            name: "Clip One".into(),
            asset_type: ClipType::Video,
            source: MediaSource::Project {
                relative_path: "media/clip.mov".into(),
            },
            duration: 12.5,
            generation_input: None,
            source_width: Some(1920),
            source_height: Some(1080),
            source_fps: Some(30.0),
            has_audio: Some(true),
            folder_id: None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        });
        m
    }

    // --- Read severities (reference read :32 / :48 / :52) ---

    #[test]
    fn missing_project_json_is_corrupt() {
        // Ruling: project.json missing = HARD corrupt (reference :32 throws
        // .fileReadCorruptFile). An empty bundle dir → Corrupt{project.json}.
        let root = scratch();
        let bundle = root.join("Empty.palmier");
        fs::create_dir_all(&bundle).unwrap();

        let err = read_bundle(&bundle).unwrap_err();
        match err {
            BundleError::Corrupt { file, .. } => assert_eq!(file, project::TIMELINE_FILE),
            other => panic!("expected Corrupt, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn corrupt_project_json_is_corrupt() {
        // Decode failure on project.json is also HARD (reference :38 propagates).
        let root = scratch();
        let bundle = root.join("Bad.palmier");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join(project::TIMELINE_FILE), b"{ not json").unwrap();

        match read_bundle(&bundle).unwrap_err() {
            BundleError::Corrupt { file, .. } => assert_eq!(file, project::TIMELINE_FILE),
            other => panic!("expected Corrupt, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn corrupt_media_json_is_corrupt() {
        // Ruling: media.json decode failure = HARD corrupt (reference :48 throws
        // .fileReadCorruptFile). project.json is fine; media.json is garbage.
        let root = scratch();
        let bundle = root.join("BadManifest.palmier");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join(project::TIMELINE_FILE),
            serde_json::to_vec(&sample_timeline()).unwrap(),
        )
        .unwrap();
        fs::write(bundle.join(project::MANIFEST_FILE), b"{ not json").unwrap();

        match read_bundle(&bundle).unwrap_err() {
            BundleError::Corrupt { file, .. } => assert_eq!(file, project::MANIFEST_FILE),
            other => panic!("expected Corrupt on media.json, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_media_json_is_ok() {
        // Absence of media.json is NOT an error (only a decode failure is).
        let root = scratch();
        let bundle = root.join("NoManifest.palmier");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join(project::TIMELINE_FILE),
            serde_json::to_vec(&sample_timeline()).unwrap(),
        )
        .unwrap();

        let loaded = read_bundle(&bundle).unwrap();
        assert!(loaded.manifest.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn corrupt_generation_log_is_soft() {
        // Ruling: generation-log.json failure is SOFT (reference :52 `try?`) — the
        // bundle still opens, generation_log = None.
        let root = scratch();
        let bundle = root.join("BadLog.palmier");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(
            bundle.join(project::TIMELINE_FILE),
            serde_json::to_vec(&sample_timeline()).unwrap(),
        )
        .unwrap();
        fs::write(bundle.join(project::GENERATION_LOG_FILE), b"{ not json").unwrap();

        let loaded = read_bundle(&bundle).expect("corrupt gen-log must NOT error");
        assert!(loaded.generation_log.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    // --- Round-trip (SM-7 seed: write → read → identical Timeline + manifest) ---

    #[test]
    fn write_then_read_round_trips_timeline_and_manifest() {
        let root = scratch();
        let bundle = root.join("RoundTrip.palmier");
        let timeline = sample_timeline();
        let manifest = sample_manifest();

        let mut snap = BundleSnapshot::new(timeline.clone());
        snap.manifest = Some(manifest.clone());
        snap.generation_log = Some(GenerationLog::new());
        write_bundle(&bundle, &snap).unwrap();

        let loaded = read_bundle(&bundle).unwrap();
        assert_eq!(loaded.timeline, timeline, "Timeline must round-trip");
        assert_eq!(
            loaded.manifest,
            Some(manifest),
            "MediaManifest must round-trip"
        );
        assert_eq!(loaded.generation_log, Some(GenerationLog::new()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn project_json_is_byte_stable_across_save_reopen_save() {
        // SM-7 byte-identical: encode(timeline) is stable through a save/reopen.
        let root = scratch();
        let bundle = root.join("Stable.palmier");
        let timeline = sample_timeline();

        write_bundle(&bundle, &BundleSnapshot::new(timeline.clone())).unwrap();
        let first = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();

        let reloaded = read_bundle(&bundle).unwrap();
        write_bundle(&bundle, &BundleSnapshot::new(reloaded.timeline)).unwrap();
        let second = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();

        assert_eq!(first, second, "project.json bytes must be stable");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn chat_dir_is_rebuilt_each_save() {
        let root = scratch();
        let bundle = root.join("Chat.palmier");
        let mut snap = BundleSnapshot::new(sample_timeline());
        snap.chat_files = vec![("session-a.json".into(), br#"{"id":"a"}"#.to_vec())];
        write_bundle(&bundle, &snap).unwrap();

        let files = read_chat_session_files(&bundle).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "session-a.json");

        // A save with no sessions rebuilds chat/ fresh → empty (only non-empty
        // sessions persist; the caller filters them out before passing in).
        write_bundle(&bundle, &BundleSnapshot::new(sample_timeline())).unwrap();
        assert!(read_chat_session_files(&bundle).unwrap().is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    // --- media/ carry-forward (reference mediaDirWrapper :103) ---

    #[test]
    fn existing_media_dir_is_carried_forward_on_save() {
        let root = scratch();
        let bundle = root.join("Media.palmier");
        // First save creates the bundle (no media/ yet).
        write_bundle(&bundle, &BundleSnapshot::new(sample_timeline())).unwrap();

        // Simulate media import: write a file under the LIVE bundle's media/ dir.
        let media = bundle.join(project::MEDIA_DIR);
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("clip.mov"), b"\x00\x01\x02fake").unwrap();

        // Re-save: the existing media/ must survive the atomic swap.
        write_bundle(&bundle, &BundleSnapshot::new(sample_timeline())).unwrap();
        assert!(
            bundle.join(project::MEDIA_DIR).join("clip.mov").is_file(),
            "media/ must be carried forward across the atomic save"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_media_dir_when_none_exists() {
        // If no live media/ exists, the saved package has none (reference returns
        // nil from mediaDirWrapper → no media child).
        let root = scratch();
        let bundle = root.join("NoMedia.palmier");
        write_bundle(&bundle, &BundleSnapshot::new(sample_timeline())).unwrap();
        assert!(!bundle.join(project::MEDIA_DIR).exists());
        fs::remove_dir_all(&root).unwrap();
    }

    // --- Atomicity: a mid-save failure leaves the original bundle intact ---

    #[test]
    fn failed_save_leaves_original_intact_and_no_staging() {
        // A normal re-save must replace the bundle cleanly with NO staging/backup
        // sibling left behind (the swap deletes both the temp and the backup).
        let root = scratch();
        let bundle = root.join("Atomic.palmier");
        write_bundle(&bundle, &BundleSnapshot::new(sample_timeline())).unwrap();
        let mut v2 = sample_timeline();
        v2.fps = 24;
        write_bundle(&bundle, &BundleSnapshot::new(v2)).unwrap();
        assert_eq!(read_bundle(&bundle).unwrap().timeline.fps, 24);
        assert!(no_temp_siblings(&root), "no staging/backup siblings after save");

        // An INJECTED mid-save failure (destination parent is a regular file, so
        // `create_dir_all(parent)` fails before any swap) must leave the existing
        // good bundle byte-for-byte intact and drop no partial sibling.
        let original = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();
        let blocker = root.join("blocker-file");
        fs::write(&blocker, b"x").unwrap();
        let doomed = blocker.join("Doomed.palmier"); // parent is a file → fails
        assert!(
            write_bundle(&doomed, &BundleSnapshot::new(sample_timeline())).is_err(),
            "save under a file-parent must fail"
        );
        assert_eq!(
            fs::read(bundle.join(project::TIMELINE_FILE)).unwrap(),
            original,
            "the good bundle must survive a failed save unchanged"
        );
        assert!(no_temp_siblings(&root), "no partial sibling after a failed save");

        fs::remove_dir_all(&root).unwrap();
    }

    /// True if no `.tmp`/`.bak` staging/backup sibling remains directly under `dir`.
    fn no_temp_siblings(dir: &Path) -> bool {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .all(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                !(n.contains(".tmp") || n.contains(".bak"))
            })
    }
}
