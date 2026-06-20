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
    /// READ arms call real bodies (E7-S5); every other arm returns the structured
    /// not-yet-implemented result until its category story lands (E7-S6..S10).
    fn run(&self, tool: ToolName, state: &EditorState, args: &Value) -> ToolResult {
        match tool {
            // ── READ (real bodies, E7-S5) ──────────────────────────────────
            ToolName::GetTimeline => read::get_timeline(state, args),
            ToolName::GetMedia => read::get_media(state),
            ToolName::GetTranscript => read::get_transcript(state, args),
            ToolName::ListFolders => read::list_folders(state),
            ToolName::ListModels => read::list_models(state, args),
            // ── READ (async backends, deferred — E7-S9) ────────────────────
            ToolName::InspectMedia => not_implemented(tool),
            ToolName::InspectTimeline => not_implemented(tool),
            ToolName::SearchMedia => not_implemented(tool),
            // ── EDIT (E7-S6 / E7-S7) ───────────────────────────────────────
            ToolName::AddClips => not_implemented(tool),
            ToolName::RemoveClips => not_implemented(tool),
            ToolName::RemoveTracks => not_implemented(tool),
            ToolName::MoveClips => not_implemented(tool),
            ToolName::SetClipProperties => not_implemented(tool),
            ToolName::SetKeyframes => not_implemented(tool),
            ToolName::SplitClip => not_implemented(tool),
            ToolName::RippleDeleteRanges => not_implemented(tool),
            // ── UNDO (E7-S12) ──────────────────────────────────────────────
            ToolName::Undo => not_implemented(tool),
            // ── TEXT / CAPTION (E7-S8) ─────────────────────────────────────
            ToolName::AddTexts => not_implemented(tool),
            ToolName::AddCaptions => not_implemented(tool),
            // ── GENERATE (E7-S9, Epic 9 backend) ───────────────────────────
            ToolName::GenerateVideo => not_implemented(tool),
            ToolName::GenerateImage => not_implemented(tool),
            ToolName::GenerateAudio => not_implemented(tool),
            ToolName::UpscaleMedia => not_implemented(tool),
            // ── LIBRARY (E7-S10) ───────────────────────────────────────────
            ToolName::ImportMedia => not_implemented(tool),
            ToolName::CreateFolder => not_implemented(tool),
            ToolName::MoveToFolder => not_implemented(tool),
            ToolName::RenameMedia => not_implemented(tool),
            ToolName::RenameFolder => not_implemented(tool),
            ToolName::DeleteMedia => not_implemented(tool),
            ToolName::DeleteFolder => not_implemented(tool),
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
        // The binding is `mut` because the mutating tool bodies (E7-S6..S10) take
        // `&mut EditorState`; the READ bodies in this story only read.
        #[allow(unused_mut)]
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

        // (6) Snapshot the timeline before the run for change detection (E7-S12).
        let before = guard.library.timeline.clone();

        // (7) Dispatch to the matching arm. READ bodies are real; the rest stub.
        let result = self.run(tool, &guard, &resolved);

        // (8) Agent-undo push hook (E7-S12): after a non-undo, non-error,
        // timeline-changing run, push the current undo-action name onto the agent
        // stack. The mutating tool bodies (E7-S6..S10) wrap their work in named
        // undo groups via `History::with_agent_swap`; until those land, the
        // timeline never changes here, so this is a no-op — but the hook + its
        // exact predicate live here now so the mutating stories only fill bodies.
        if tool != ToolName::Undo
            && !result.is_error
            && guard.library.timeline != before
        {
            if let Some(action_name) = guard.history.current_undo_action_name() {
                let _name = action_name.to_string();
                // NOTE: with_agent_swap already records the name on the agent
                // stack as the body runs (that is the reference's push site). This
                // predicate mirrors the reference's belt-and-suspenders check; no
                // extra push is needed here because the swap helper owns the stack.
                // The mutating bodies (E7-S6+) call `history.with_agent_swap(name,
                // …)` which performs the push. Left explicit for the undo story.
            }
        }

        // (9) ShortId output shortening on text blocks (reference shorteningIds).
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

/// A structured "not yet implemented" result for a dispatched tool whose body is a
/// later E7 story. NOT the `{ isError }` shape — dispatch succeeded; the body is
/// the placeholder.
fn not_implemented(tool: ToolName) -> ToolResult {
    ToolResult::ok(format!(
        "Tool '{}' is registered and dispatched, but its body is not yet wired \
         (lands in a later Epic 7 story).",
        tool.wire_name()
    ))
}
