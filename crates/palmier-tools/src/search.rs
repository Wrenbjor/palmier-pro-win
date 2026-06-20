//! `search_media` tool body — the agent's content-search surface (E11-S10;
//! reference `ToolExecutor+Search.swift`, `searchMedia(_:_:)`).
//!
//! One tool, two scopes (`visual | spoken | both`, default `both`):
//! - **visual** (async, coordinator) → the E11-S6
//!   [`SearchIndexCoordinator`](palmier_search::SearchIndexCoordinator)`.search`, ranked
//!   by E11-S5; each hit's `time`/`shotStart`/`shotEnd` map to the `range`. The coordinator's
//!   model-loader state surfaces as `visual_status` (`ready | indexing | model_not_installed |
//!   downloading_model | preparing | disabled | failed`).
//! - **spoken** (sync) → the E11-S8 [`TranscriptSearch`](palmier_search::TranscriptSearch)`.search`
//!   over the **disk-only** transcript cache (E10-S4 / E11-S7); each hit's `start`/`end` map to
//!   the `range`. Always available with **no model download** (keyword recall, FR-40).
//!
//! ## Why a [`VisualSearchGateway`] seam (parity with the [`generate`](crate::generate) gateway)
//! The visual encode path needs the real SigLIP2 embedder, which only compiles under
//! `palmier-search/ort`, and the coordinator's `search` is generic over a
//! [`QueryEncoder`](palmier_search::QueryEncoder). The synchronous tool layer cannot own the
//! `ort`-gated encoder directly, so the executor holds an **optional** trait-object gateway:
//! the host (`palmier-tauri`) wires a coordinator + a real query-encoder behind it; the body
//! calls [`VisualSearchGateway::search_visual`]. With **no gateway wired** (the default build,
//! or before the host attaches one) the visual path reports [`VisualStatus::Disabled`] / empty,
//! exactly the "search registered but not yet functional" contract (E7-S9 stub behavior, now
//! real for spoken and gated-real for visual).
//!
//! ## Spoken path uses the same transcript-cache plumbing as E10-S7
//! The spoken scope reads the on-device transcript cache through
//! [`acquire_cache`](crate::caption_transcribe::acquire_cache) (honoring the
//! [`CACHE_DIR_ENV`](crate::caption_transcribe::CACHE_DIR_ENV) override the caption tools'
//! tests already use), driven from a small per-call tokio runtime — the cache singleton init
//! is async — then runs the **synchronous** [`TranscriptSearch::search`] over it. No model,
//! no transcription at query time.

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use palmier_model::{ClipType, MediaAsset, MediaSource};
use palmier_search::{visual_search::Hit as VisualHit, TranscriptHit, VisualStatus};

use crate::caption_transcribe::acquire_cache;
use crate::editor::EditorState;
use crate::result::ToolResult;

/// Result cap defaults (reference `min(max(limit ?? 10, 1), 50)`).
const DEFAULT_LIMIT: usize = 10;
const MIN_LIMIT: usize = 1;
const MAX_LIMIT: usize = 50;

/// A visual-indexable candidate the gateway ranks — the decoupled snapshot the host
/// turns into a `palmier_search::CoordinatorAsset` (id + resolved file path + kind).
///
/// Kept gateway-facing (not `CoordinatorAsset`) so the tool layer never needs the
/// `palmier-search` coordinator types in its default-build signature, and so a test
/// gateway can ignore the path entirely.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualCandidate {
    /// Stable asset id.
    pub id: String,
    /// Resolved on-disk file path.
    pub path: PathBuf,
    /// `true` for image assets (the reference emits `type: "image"` and no range).
    pub is_image: bool,
}

/// The visual-search seam the host wires (mirrors the [`GenerationGateway`](crate::generate)
/// pattern). Implemented by the host over a [`SearchIndexCoordinator`] + a real
/// `ort` query-encoder; mocked in tests. Absent ⇒ the visual scope reports
/// [`VisualStatus::Disabled`] with no hits.
pub trait VisualSearchGateway: Send + Sync {
    /// Rank `query` against the indexed `candidates` (already filtered to the active
    /// scope's assets, and to `media_ref` if one was given), returning the top
    /// `limit` hits plus the live model-loader status.
    fn search_visual(
        &self,
        query: &str,
        candidates: &[VisualCandidate],
        limit: usize,
    ) -> (Vec<VisualHit>, VisualStatus);

    /// The model-loader status when there is nothing to rank (no candidates / wrong
    /// scope) — so `visual_status` still reflects `ready/indexing/...` truthfully.
    fn status(&self) -> VisualStatus;
}

/// Map a [`VisualStatus`] to its snake_case wire string (FOUNDATION §6.14
/// `visual_status` enum). `downloading_model` drops the fraction (the UI reads
/// progress separately; the tool surfaces the discrete state).
pub fn visual_status_wire(status: &VisualStatus) -> &'static str {
    match status {
        VisualStatus::Ready => "ready",
        VisualStatus::Indexing => "indexing",
        VisualStatus::ModelNotInstalled => "model_not_installed",
        VisualStatus::DownloadingModel(_) => "downloading_model",
        VisualStatus::Preparing => "preparing",
        VisualStatus::Disabled => "disabled",
        VisualStatus::Failed(_) => "failed",
    }
}

/// Resolve a `MediaAsset`'s on-disk path the same way the caption plumbing does
/// (`External` → absolute; `Project` → relative as-is, host resolves the bundle root).
fn asset_file(asset: &MediaAsset) -> PathBuf {
    match &asset.source {
        MediaSource::External { absolute_path } => PathBuf::from(absolute_path),
        MediaSource::Project { relative_path } => PathBuf::from(relative_path),
    }
}

/// `search_media` (reference `searchMedia`): `query` (required, trimmed, non-empty),
/// `scope?` (`visual | spoken | both`, default `both`), `mediaRef?` (restrict to one
/// asset), `limit?` (per-group cap, default 10, clamped `[1, 50]`).
///
/// ShortId expansion of `mediaRef` is already done by the executor (E7-S4) before this
/// body runs, so a prefix arrives here as a full id; we still emit a not-found error if
/// it names no asset.
pub fn search_media(state: &EditorState, args: &Value) -> ToolResult {
    // query — required, trimmed, non-empty (reference `requireString` + empty guard).
    let query = match args.get("query").and_then(Value::as_str) {
        Some(s) => s.trim(),
        None => return ToolResult::error("search_media: query is empty"),
    };
    if query.is_empty() {
        return ToolResult::error("search_media: query is empty");
    }

    // scope — validated enum (reference guard).
    let scope = args.get("scope").and_then(Value::as_str).unwrap_or("both");
    if !matches!(scope, "visual" | "spoken" | "both") {
        return ToolResult::error(format!(
            "search_media: scope must be visual, spoken, or both (got '{scope}')"
        ));
    }

    // limit — clamp to [1, 50], default 10 (reference `min(max(limit ?? 10, 1), 50)`).
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .map(|n| (n.max(MIN_LIMIT as i64) as usize).min(MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);

    // mediaRef — optional single-asset restriction. A given ref that names no asset is
    // an error (reference `asset(ref, editor:)` throws on a miss).
    let restrict: Option<String> = match args.get("mediaRef").and_then(Value::as_str) {
        Some(ref_id) => {
            if !state.library.assets.iter().any(|a| a.id == ref_id) {
                return ToolResult::error(format!("search_media: no media asset '{ref_id}'"));
            }
            Some(ref_id.to_string())
        }
        None => None,
    };

    let mut payload = serde_json::Map::new();
    payload.insert("query".into(), json!(query));
    payload.insert("scope".into(), json!(scope));

    if scope != "spoken" {
        payload.insert(
            "visual".into(),
            visual_results(state, query, limit, restrict.as_deref()),
        );
    }
    if scope != "visual" {
        payload.insert(
            "spoken".into(),
            json!(spoken_results(state, query, limit, restrict.as_deref())),
        );
    }

    match serde_json::to_string(&Value::Object(payload)) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("search_media: failed to encode results"),
    }
}

/// The visual scope: snapshot the (video|image) candidates (optionally scoped to
/// `restrict`), hand them to the wired [`VisualSearchGateway`], and shape the hits +
/// `visual_status`. With no gateway wired ⇒ `status: disabled`, `moments: []`.
fn visual_results(
    state: &EditorState,
    query: &str,
    limit: usize,
    restrict: Option<&str>,
) -> Value {
    // The (video|image) candidate snapshot, optionally restricted to one asset.
    let candidates: Vec<VisualCandidate> = state
        .library
        .assets
        .iter()
        .filter(|a| {
            (a.asset_type == ClipType::Video || a.asset_type == ClipType::Image)
                && restrict.map(|r| r == a.id).unwrap_or(true)
        })
        .map(|a| VisualCandidate {
            id: a.id.clone(),
            path: asset_file(a),
            is_image: a.asset_type == ClipType::Image,
        })
        .collect();

    let image_ids: HashSet<String> = candidates
        .iter()
        .filter(|c| c.is_image)
        .map(|c| c.id.clone())
        .collect();

    let (hits, status) = match state.visual_search_gateway() {
        Some(gw) => gw.search_visual(query, &candidates, limit),
        // No gateway (default build / not yet wired) ⇒ disabled + empty.
        None => (Vec::new(), VisualStatus::Disabled),
    };

    let moments: Vec<Value> = hits
        .iter()
        .map(|hit| visual_hit_json(hit, &image_ids, state))
        .collect();

    json!({
        "status": visual_status_wire(&status),
        "moments": moments,
    })
}

/// Shape one visual hit (reference `moments.map`): `mediaRef`, `name`, `score`, and
/// either `type: "image"` (stills carry no range) or `startSeconds`/`endSeconds` (the
/// shot range — `shot_start`/`shot_end` → `range`).
fn visual_hit_json(hit: &VisualHit, image_ids: &HashSet<String>, state: &EditorState) -> Value {
    let name = state
        .library
        .assets
        .iter()
        .find(|a| a.id == hit.asset_id)
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let mut entry = serde_json::Map::new();
    entry.insert("mediaRef".into(), json!(hit.asset_id));
    entry.insert("name".into(), json!(name));
    entry.insert("score".into(), json!(hit.score as f64));
    if image_ids.contains(&hit.asset_id) {
        entry.insert("type".into(), json!("image"));
    } else {
        entry.insert("startSeconds".into(), json!(hit.shot_start));
        entry.insert("endSeconds".into(), json!(hit.shot_end));
    }
    Value::Object(entry)
}

/// The spoken scope (sync): keyword search over the **disk-only** transcript cache via
/// E11-S8. Candidates are the (video|audio) assets, optionally scoped to `restrict`.
/// Each hit's `start`/`end` → `range` (`startSeconds`/`endSeconds`).
fn spoken_results(
    state: &EditorState,
    query: &str,
    limit: usize,
    restrict: Option<&str>,
) -> Vec<Value> {
    let candidates: Vec<(String, PathBuf)> = state
        .library
        .assets
        .iter()
        .filter(|a| {
            (a.asset_type == ClipType::Video || a.asset_type == ClipType::Audio)
                && restrict.map(|r| r == a.id).unwrap_or(true)
        })
        .map(|a| (a.id.clone(), asset_file(a)))
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    // The transcript cache singleton init is async — drive it from a small per-call
    // runtime (the E10-S7 pattern), then run the SYNCHRONOUS keyword search over it.
    let runtime = match crate::caption_transcribe::build_runtime() {
        Ok(rt) => rt,
        Err(_) => return Vec::new(),
    };
    let hits: Vec<TranscriptHit> = runtime.block_on(async {
        let cache = acquire_cache().await;
        palmier_search::TranscriptSearch::search(query, &candidates, limit, &cache)
    });

    hits.into_iter()
        .map(|hit| {
            let name = state
                .library
                .assets
                .iter()
                .find(|a| a.id == hit.asset_id)
                .map(|a| a.name.clone())
                .unwrap_or_default();
            json!({
                "mediaRef": hit.asset_id,
                "name": name,
                "startSeconds": hit.start,
                "endSeconds": hit.end,
                "text": hit.text,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ── status wire mapping ─────────────────────────────────────────────────
    #[test]
    fn visual_status_wire_covers_every_state() {
        assert_eq!(visual_status_wire(&VisualStatus::Ready), "ready");
        assert_eq!(visual_status_wire(&VisualStatus::Indexing), "indexing");
        assert_eq!(
            visual_status_wire(&VisualStatus::ModelNotInstalled),
            "model_not_installed"
        );
        assert_eq!(
            visual_status_wire(&VisualStatus::DownloadingModel(0.5)),
            "downloading_model"
        );
        assert_eq!(visual_status_wire(&VisualStatus::Preparing), "preparing");
        assert_eq!(visual_status_wire(&VisualStatus::Disabled), "disabled");
        assert_eq!(
            visual_status_wire(&VisualStatus::Failed("x".into())),
            "failed"
        );
    }

    // ── a mock gateway driving the visual path (no ort, no weights) ──────────
    struct MockGateway {
        hits: Mutex<Vec<VisualHit>>,
        status: VisualStatus,
        seen: Mutex<Vec<String>>,
    }
    impl VisualSearchGateway for MockGateway {
        fn search_visual(
            &self,
            _query: &str,
            candidates: &[VisualCandidate],
            _limit: usize,
        ) -> (Vec<VisualHit>, VisualStatus) {
            *self.seen.lock().unwrap() = candidates.iter().map(|c| c.id.clone()).collect();
            (self.hits.lock().unwrap().clone(), self.status.clone())
        }
        fn status(&self) -> VisualStatus {
            self.status.clone()
        }
    }

    fn video(id: &str) -> MediaAsset {
        MediaAsset::new(
            id,
            id,
            ClipType::Video,
            MediaSource::External { absolute_path: format!("/x/{id}.mov") },
            10.0,
        )
    }
    fn image(id: &str) -> MediaAsset {
        MediaAsset::new(
            id,
            id,
            ClipType::Image,
            MediaSource::External { absolute_path: format!("/x/{id}.png") },
            0.0,
        )
    }

    fn state_with(assets: Vec<MediaAsset>) -> EditorState {
        let mut lib = palmier_model::MediaLibrary::new();
        lib.assets = assets;
        EditorState::with_library(lib)
    }

    #[test]
    fn empty_query_is_an_error() {
        let st = state_with(vec![]);
        let r = search_media(&st, &json!({ "query": "   " }));
        assert!(r.is_error);
    }

    #[test]
    fn bad_scope_is_an_error() {
        let st = state_with(vec![]);
        let r = search_media(&st, &json!({ "query": "x", "scope": "sideways" }));
        assert!(r.is_error);
        if let crate::result::Block::Text(s) = &r.content[0] {
            assert!(s.contains("scope must be"), "{s}");
        }
    }

    #[test]
    fn unknown_media_ref_is_an_error() {
        let st = state_with(vec![video("v1")]);
        let r = search_media(&st, &json!({ "query": "x", "mediaRef": "nope" }));
        assert!(r.is_error);
    }

    #[test]
    fn visual_disabled_without_a_gateway() {
        let st = state_with(vec![video("v1")]);
        let r = search_media(&st, &json!({ "query": "harbor", "scope": "visual" }));
        assert!(!r.is_error);
        let v: Value = serde_json::from_str(body(&r)).unwrap();
        assert_eq!(v["visual"]["status"], json!("disabled"));
        assert_eq!(v["visual"]["moments"], json!([]));
        // spoken not requested.
        assert!(v.get("spoken").is_none());
    }

    #[test]
    fn visual_gateway_shapes_video_and_image_hits() {
        let mut st = state_with(vec![video("vid"), image("img")]);
        let hits = vec![
            VisualHit {
                asset_id: "vid".into(),
                time: 3.0,
                shot_start: 2.0,
                shot_end: 5.0,
                score: 0.91,
            },
            VisualHit {
                asset_id: "img".into(),
                time: 0.0,
                shot_start: 0.0,
                shot_end: 0.0,
                score: 0.80,
            },
        ];
        let gw = MockGateway {
            hits: Mutex::new(hits),
            status: VisualStatus::Ready,
            seen: Mutex::new(vec![]),
        };
        st.set_visual_search_gateway(Box::new(gw));

        let r = search_media(&st, &json!({ "query": "harbor", "scope": "visual" }));
        assert!(!r.is_error);
        let v: Value = serde_json::from_str(body(&r)).unwrap();
        assert_eq!(v["visual"]["status"], json!("ready"));
        let moments = v["visual"]["moments"].as_array().unwrap();
        assert_eq!(moments.len(), 2);
        // Video hit → shot range mapped.
        assert_eq!(moments[0]["mediaRef"], json!("vid"));
        assert_eq!(moments[0]["startSeconds"], json!(2.0));
        assert_eq!(moments[0]["endSeconds"], json!(5.0));
        assert!(moments[0].get("type").is_none());
        // Image hit → type:image, no range.
        assert_eq!(moments[1]["mediaRef"], json!("img"));
        assert_eq!(moments[1]["type"], json!("image"));
        assert!(moments[1].get("startSeconds").is_none());
    }

    #[test]
    fn media_ref_scopes_visual_candidates() {
        use std::sync::Arc;
        // A shared gateway whose `seen` we can inspect after the call (the trait object
        // erases the concrete type, so hold an Arc clone to read what it saw).
        let shared = Arc::new(MockGateway {
            hits: Mutex::new(vec![]),
            status: VisualStatus::Ready,
            seen: Mutex::new(vec![]),
        });
        let mut st = state_with(vec![video("a"), video("b"), image("c")]);
        st.set_visual_search_gateway(Box::new(ArcGateway(Arc::clone(&shared))));

        let r = search_media(
            &st,
            &json!({ "query": "x", "scope": "visual", "mediaRef": "a" }),
        );
        assert!(!r.is_error);
        // Only "a" survived the restrict filter (b and the image c are excluded).
        assert_eq!(*shared.seen.lock().unwrap(), vec!["a".to_string()]);
    }

    // A thin gateway forwarding to a shared `MockGateway` so a test can inspect the
    // candidate list the body handed in (the boxed trait object can't be downcast).
    struct ArcGateway(std::sync::Arc<MockGateway>);
    impl VisualSearchGateway for ArcGateway {
        fn search_visual(
            &self,
            q: &str,
            candidates: &[VisualCandidate],
            limit: usize,
        ) -> (Vec<VisualHit>, VisualStatus) {
            self.0.search_visual(q, candidates, limit)
        }
        fn status(&self) -> VisualStatus {
            self.0.status()
        }
    }

    fn body(r: &ToolResult) -> &str {
        match &r.content[0] {
            crate::result::Block::Text(s) => s,
            _ => "",
        }
    }
}
