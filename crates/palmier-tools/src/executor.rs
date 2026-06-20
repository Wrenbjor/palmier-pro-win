//! `ToolExecutor` — the real single-owner dispatcher (E7-S2; reference
//! `ToolExecutor`, `execute(name:args:)` / `run(_:_:_:)`).
//!
//! This replaces the [`ScaffoldDispatcher`](crate::dispatch::ScaffoldDispatcher)
//! with an executor that **owns the editor state** behind a `Mutex` and routes
//! every tool call through one serialized code path:
//!
//! ```text
//! execute(name, args):
//!   1. resolve name → ToolName            (unknown → error shape)
//!   2. lock the EditorState mutex          (single-owner serialization = @MainActor)
//!   3. snapshot the id universe ONCE
//!   4. validate args (E7-S3)               (failure → error shape)
//!   5. ShortId input expansion (E7-S4)     (ambiguous prefix → error shape)
//!   6. run the matching arm                (READ bodies real; mutate/generate stub)
//!   7. agent-undo push (E7-S12)            (hook present; mutating bodies arrive later)
//!   8. ShortId output shortening (E7-S4)
//! ```
//!
//! ## Single-owner serialization
//! [`ToolExecutor`] holds `Mutex<EditorState>`. Every `execute` takes the lock for
//! its whole duration, so concurrent calls serialize — no two tool calls ever touch
//! the editor at once (the reference's `@MainActor`). The
//! [`concurrent_serialization`](crate::tests) test drives two threads at one
//! executor and asserts no data race + a serialized outcome.
//!
//! ## What's real vs deferred in this story
//! - **Real:** the executor, the `Mutex<EditorState>` owner, name resolution, arg
//!   validation wiring, ShortId expand/shorten, and the **READ tool bodies**
//!   (`get_timeline` / `get_media` / `get_transcript` / `list_folders` /
//!   `list_models`).
//! - **Deferred:** the mutate/generate/async tool bodies (E7-S5..S10) return a
//!   structured "not yet implemented" result; the agent-undo push fires only once
//!   those bodies wrap their work in named undo groups (E7-S12).

use std::sync::Mutex;

use serde_json::Value;

use crate::dispatch::{ToolContext, ToolDispatch};
use crate::editor::EditorState;
use crate::read;
use crate::result::ToolResult;
use crate::schema::ToolName;
use crate::short_id::{expand_id_prefixes, IdUniverse};
use crate::validate::validate;

/// The shared tool dispatcher with a live, single-owner editor state. Constructed
/// once per document; both the MCP server (E7-S11) and the in-app agent (E8) call
/// [`ToolExecutor::execute`].
pub struct ToolExecutor {
    /// The single owner of the editor state. The `Mutex` is the serialization
    /// boundary (reference `@MainActor`): one tool call at a time.
    state: Mutex<EditorState>,
}

impl Default for ToolExecutor {
    fn default() -> ToolExecutor {
        ToolExecutor::new()
    }
}

impl ToolExecutor {
    /// A new executor over a fresh, empty editor.
    pub fn new() -> ToolExecutor {
        ToolExecutor { state: Mutex::new(EditorState::new()) }
    }

    /// A new executor over an existing editor state (a loaded project).
    pub fn with_state(state: EditorState) -> ToolExecutor {
        ToolExecutor { state: Mutex::new(state) }
    }

    /// Run `f` with shared access to the editor state (test/introspection helper).
    /// Takes the same lock `execute` uses, so it serializes with tool calls.
    pub fn with_state_ref<R>(&self, f: impl FnOnce(&EditorState) -> R) -> R {
        let guard = self.state.lock().expect("editor state mutex poisoned");
        f(&guard)
    }

    /// Run `f` with mutable access to the editor state (used by tests/host to set
    /// up a project, e.g. load a library or toggle `can_generate`).
    pub fn with_state_mut<R>(&self, f: impl FnOnce(&mut EditorState) -> R) -> R {
        let mut guard = self.state.lock().expect("editor state mutex poisoned");
        f(&mut guard)
    }

    /// The exhaustive 30-arm dispatch (reference `run`). No `_`/`default` arm: each
    /// [`ToolName`] routes explicitly, so adding a 31st variant fails to compile —
    /// the same SM-C2 gate the reference relies on.
    ///
    /// READ arms call real bodies (E7-S5) with `&EditorState`; the EDIT + UNDO arms
    /// (E7-S4 / E7-S12) take `&mut EditorState` and self-register their **one** agent
    /// undo step (via [`crate::undo::agent_edit`] / [`crate::undo::undo`]). The
    /// generate / library / async arms remain structured "not yet implemented" until
    /// their later stories land (E7-S8..S10).
    fn run(&self, tool: ToolName, state: &mut EditorState, args: &Value) -> ToolResult {
        match tool {
            // ── READ (real bodies, E7-S5) ──────────────────────────────────
            ToolName::GetTimeline => read::get_timeline(state, args),
            ToolName::GetMedia => read::get_media(state),
            ToolName::GetTranscript => read::get_transcript(state, args),
            ToolName::ListFolders => read::list_folders(state),
            ToolName::ListModels => read::list_models(state, args),
            // ── READ — inspect (E7-S5) ─────────────────────────────────────
            ToolName::InspectMedia => crate::inspect::inspect_media(state, args),
            ToolName::InspectTimeline => crate::inspect::inspect_timeline(state, args),
            // search_media stays stubbed (visual index = Epic 11/M4) ────────
            ToolName::SearchMedia => crate::inspect::search_media(state, args),
            // ── EDIT — clips (E7-S6 / E7-S4) ───────────────────────────────
            ToolName::AddClips => crate::clips::add_clips(state, args),
            ToolName::RemoveClips => crate::clips::remove_clips(state, args),
            ToolName::RemoveTracks => crate::clips::remove_tracks(state, args),
            ToolName::MoveClips => crate::clips::move_clips(state, args),
            ToolName::SplitClip => crate::clips::split_clip(state, args),
            // ── EDIT — properties / keyframes / ripple (E7-S7 / E7-S4) ─────
            ToolName::SetClipProperties => crate::properties::set_clip_properties(state, args),
            ToolName::SetKeyframes => crate::properties::set_keyframes(state, args),
            ToolName::RippleDeleteRanges => crate::properties::ripple_delete_ranges(state, args),
            // ── UNDO (E7-S12) ──────────────────────────────────────────────
            ToolName::Undo => crate::undo::undo(state),
            // ── TEXT / CAPTION (E7-S8) ─────────────────────────────────────
            ToolName::AddTexts => crate::texts::add_texts(state, args),
            ToolName::AddCaptions => crate::texts::add_captions(state, args),
            // ── GENERATE (E9-S11 — wired to palmier-gen via the gateway seam) ──
            ToolName::GenerateVideo => crate::generate::generate_video(state, args),
            ToolName::GenerateImage => crate::generate::generate_image(state, args),
            ToolName::GenerateAudio => crate::generate::generate_audio(state, args),
            ToolName::UpscaleMedia => crate::generate::upscale_media(state, args),
            // ── LIBRARY (E7-S10) ───────────────────────────────────────────
            ToolName::ImportMedia => crate::library::import_media(state, args),
            ToolName::CreateFolder => crate::library::create_folder(state, args),
            ToolName::MoveToFolder => crate::library::move_to_folder(state, args),
            ToolName::RenameMedia => crate::library::rename_media(state, args),
            ToolName::RenameFolder => crate::library::rename_folder(state, args),
            ToolName::DeleteMedia => crate::library::delete_media(state, args),
            ToolName::DeleteFolder => crate::library::delete_folder(state, args),
        }
    }
}

impl ToolDispatch for ToolExecutor {
    fn execute(&self, name: &str, args: Value, _ctx: &dyn ToolContext) -> ToolResult {
        // (1) Resolve name → ToolName. Unknown name → tool-error shape.
        let Some(tool) = ToolName::from_wire(name) else {
            return ToolResult::error(format!("Unknown tool: {name}"));
        };

        // (2) Lock the editor for the WHOLE call — the serialization boundary
        // (reference @MainActor). Concurrent execute() calls queue on this lock.
        // The binding is `mut` because the mutating tool bodies (E7-S4/S12) take
        // `&mut EditorState`; the READ bodies only read.
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        // (3) Snapshot the id universe ONCE for this call (reference
        // currentIdUniverse), reused for expand + shorten.
        let universe = guard.id_universe();

        // (4) Arg validation (E7-S3). Failure → contract error shape.
        if let Err(e) = validate(tool, &args) {
            return ToolResult::error(e.message);
        }

        // (5) ShortId input expansion. An ambiguous prefix → tool-error shape.
        let resolved = match expand_id_prefixes(&args, &universe) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(e.message),
        };

        // (6) Dispatch to the matching arm. READ bodies read; the EDIT + UNDO
        // bodies mutate and self-register their ONE agent-undo step (E7-S12, via
        // `agent_edit` / `undo`). The agent-stack push therefore happens INSIDE the
        // body, on the exact before/after diff — no separate push hook is needed
        // here (the reference's `agentUndoStack.append` lives at the body's edge;
        // we co-locate it with the swap so the name + change-detection are atomic).
        let result = self.run(tool, &mut guard, &resolved);

        // (7) ShortId output shortening on text blocks (reference shorteningIds).
        // Re-snapshot the universe so ids created by a (future) mutating body are
        // shortened too — matches the reference shortening on the post-run state.
        let post_universe = guard.id_universe();
        shorten_result(result, &post_universe)
    }
}

/// Apply ShortId shortening to every text block of a result (reference
/// `shorteningIds`). Image blocks pass through untouched.
fn shorten_result(result: ToolResult, universe: &IdUniverse) -> ToolResult {
    if universe.is_empty() {
        return result;
    }
    let content = result
        .content
        .into_iter()
        .map(|block| match block {
            crate::result::Block::Text(s) => crate::result::Block::Text(universe.shorten_text(&s)),
            other => other,
        })
        .collect();
    ToolResult { content, is_error: result.is_error }
}

