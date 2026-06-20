//! The dispatch seam — `ToolDispatch` + the 30-arm `run` skeleton
//! (reference `ToolExecutor.execute(name:args:)` / `run(_:_:_:)`).
//!
//! This is the **single code path** both the MCP server (E7-S11) and the in-app
//! agent (E8) invoke. [`ToolDispatch::execute`] resolves the wire name, expands
//! ShortId prefixes on the input, dispatches to the matching arm, and wraps any
//! error into the `{ isError: true }` [`ToolResult`] shape. Tool *bodies* are
//! later E7 stories — every arm here routes to [`not_implemented`], a structured
//! "not yet implemented" result, so the surface compiles and dispatches now while
//! preserving the exact dispatch + arg-parsing + error-wrapping seam.
//!
//! ## What is real in this scaffold
//! - **Name resolution** — unknown wire name → `{ isError }` "Unknown tool".
//! - **ShortId input expansion** — runs before the arm; an ambiguous prefix is
//!   wrapped into the tool-error shape (E7-S4 logic).
//! - **Exhaustive 30-arm `match`** — no `_`/`default` arm, mirroring the
//!   reference's exhaustive switch. Adding a 31st `ToolName` variant would fail to
//!   compile here, the same gate the reference relies on.
//! - **Error wrapping** — every failure path returns the contract error shape.
//!
//! ## What is deferred
//! - The 30 tool bodies (E7-S5..S10) — currently [`not_implemented`].
//! - Arg validation (E7-S3) — the `validate` seam is marked `TODO(E7-S3)`.
//! - Agent undo stack push (E7-S12) — marked `TODO(E7-S12)`.
//! - Output ShortId shortening is applied here (E7-S4) since the universe snapshot
//!   is already taken for input expansion.

use serde_json::Value;

use crate::result::{Block, ToolResult};
use crate::schema::ToolName;
use crate::short_id::{expand_id_prefixes, IdUniverse};

/// Context handed to the dispatcher for one tool call. In later stories this
/// carries the editor handle (timeline/library + edit engines + history). For the
/// scaffold it provides the [`IdUniverse`] snapshot used by ShortId expand/shorten.
///
/// The reference serializes all tool calls through one `@MainActor` owner; the
/// port replaces that with single-owner serialization (a `Mutex<EditorState>` or a
/// command actor, mcp-tools.md "macOS APIs to replace"). The owner is introduced
/// in E7-S2's executor; this `ToolContext` is the seam it will satisfy.
pub trait ToolContext {
    /// The id universe snapshot for this call (reference `currentIdUniverse`).
    /// Taken once per [`ToolDispatch::execute`] and reused for both input
    /// expansion and output shortening.
    fn id_universe(&self) -> IdUniverse;
}

/// The single dispatch seam. Implemented by the executor (E7-S2) and called by
/// both the MCP server and the in-app agent.
pub trait ToolDispatch {
    /// Dispatch a tool by wire `name` with JSON `args`, returning a [`ToolResult`].
    ///
    /// Pipeline (reference `execute`): resolve name → expand ShortId prefixes →
    /// run the matching arm → shorten ids on output → wrap errors. Never panics;
    /// every error path yields the `{ isError: true }` shape.
    fn execute(&self, name: &str, args: Value, ctx: &dyn ToolContext) -> ToolResult;
}

/// The scaffold dispatcher. Owns no editor state yet (E7-S2 will add the
/// single-owner `EditorState`); implements the full dispatch *seam* with stubbed
/// tool bodies so MCP/agent integration can be built against a stable entry point.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScaffoldDispatcher;

impl ScaffoldDispatcher {
    pub fn new() -> ScaffoldDispatcher {
        ScaffoldDispatcher
    }

    /// The 30-arm exhaustive dispatch (reference `run`). No `_`/`default` arm:
    /// every [`ToolName`] variant routes explicitly, so the compiler enforces the
    /// 30-tool surface. Each arm currently returns [`not_implemented`]; later
    /// stories replace the body per category.
    fn run(&self, tool: ToolName, _args: &Value, _ctx: &dyn ToolContext) -> ToolResult {
        match tool {
            // READ (E7-S5 / E7-S9)
            ToolName::GetTimeline => not_implemented(tool),
            ToolName::GetMedia => not_implemented(tool),
            ToolName::InspectMedia => not_implemented(tool),
            ToolName::GetTranscript => not_implemented(tool),
            ToolName::InspectTimeline => not_implemented(tool),
            ToolName::SearchMedia => not_implemented(tool),
            ToolName::ListFolders => not_implemented(tool),
            ToolName::ListModels => not_implemented(tool),
            // EDIT (E7-S6 / E7-S7)
            ToolName::AddClips => not_implemented(tool),
            ToolName::RemoveClips => not_implemented(tool),
            ToolName::RemoveTracks => not_implemented(tool),
            ToolName::MoveClips => not_implemented(tool),
            ToolName::SetClipProperties => not_implemented(tool),
            ToolName::SetKeyframes => not_implemented(tool),
            ToolName::SplitClip => not_implemented(tool),
            ToolName::RippleDeleteRanges => not_implemented(tool),
            // UNDO (E7-S12)
            ToolName::Undo => not_implemented(tool),
            // TEXT / CAPTION (E7-S8)
            ToolName::AddTexts => not_implemented(tool),
            ToolName::AddCaptions => not_implemented(tool),
            // GENERATE (E7-S9, Epic 9 backend)
            ToolName::GenerateVideo => not_implemented(tool),
            ToolName::GenerateImage => not_implemented(tool),
            ToolName::GenerateAudio => not_implemented(tool),
            ToolName::UpscaleMedia => not_implemented(tool),
            // LIBRARY (E7-S10)
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

impl ToolDispatch for ScaffoldDispatcher {
    fn execute(&self, name: &str, args: Value, ctx: &dyn ToolContext) -> ToolResult {
        // (1) Resolve name → ToolName. Unknown name → tool-error shape.
        let Some(tool) = ToolName::from_wire(name) else {
            return ToolResult::error(format!("Unknown tool: {name}"));
        };

        // Snapshot the id universe ONCE for this call (reference: one
        // currentIdUniverse per execute), reused for expand + shorten.
        let universe = ctx.id_universe();

        // (2) TODO(E7-S3): arg validation (unknown-key / non-finite / decode).

        // (3) ShortId input expansion. An ambiguous prefix → tool-error shape.
        let resolved = match expand_id_prefixes(&args, &universe) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(e.message),
        };

        // (4) Dispatch to the matching arm (stubbed bodies for now).
        let result = self.run(tool, &resolved, ctx);

        // (6) TODO(E7-S12): push the agent undo stack on a successful,
        // timeline-changing, mutating, non-undo run. The hook lands here, between
        // the run and the output-shortening pass, once the executor owns the
        // editor state and the named undo groups.

        // (5) ShortId output shortening on text blocks (reference shorteningIds).
        shorten_result(result, &universe)
    }
}

/// Apply ShortId shortening to every text block of a result (reference
/// `shorteningIds(in:editor:)`). Image blocks pass through untouched.
fn shorten_result(result: ToolResult, universe: &IdUniverse) -> ToolResult {
    if universe.is_empty() {
        return result;
    }
    let content = result
        .content
        .into_iter()
        .map(|block| match block {
            Block::Text(s) => Block::Text(universe.shorten_text(&s)),
            other => other,
        })
        .collect();
    ToolResult { content, is_error: result.is_error }
}

/// A structured "not yet implemented" result for a dispatched tool. Real bodies
/// land in E7-S5..S10. This is NOT the `{ isError }` shape — dispatch succeeded;
/// the body is the placeholder. It names the tool and its owning later story so a
/// client (or a test) sees the tool was routed, not rejected.
fn not_implemented(tool: ToolName) -> ToolResult {
    ToolResult::ok(format!(
        "Tool '{}' is registered but its implementation is not yet wired (E7-S1 dispatch scaffold). \
         The body lands in a later Epic 7 story.",
        tool.wire_name()
    ))
}
