//! `EditorState` — the single-owner editor handle the tools read and mutate
//! (E7-S2; reference `EditorViewModel`, `@MainActor`).
//!
//! The macOS reference runs every tool call on the `@MainActor`, so all mutations
//! serialize through one owner and never race. The Windows/Linux port has no
//! main-actor; instead the [`ToolExecutor`](crate::executor::ToolExecutor) wraps
//! this `EditorState` in a `Mutex` and every [`ToolDispatch::execute`] call takes
//! the lock for its whole duration — reproducing the reference's single-owner
//! serialization (mcp-tools.md "macOS APIs to replace": `@MainActor` →
//! `Mutex<EditorState>` / single command actor).
//!
//! ## What it bundles
//! - The [`MediaLibrary`] — the snapshot unit the reference's
//!   `mediaLibraryUndoSnapshot` captures: the [`Timeline`] (tracks/clips), the
//!   persisted [`MediaManifest`] (folders + entries), and the runtime
//!   [`MediaAsset`] catalog. Read tools read it; mutating tools (E7-S6..S10) edit
//!   it through the edit engines.
//! - The [`History`] over [`Timeline`] — the user + **agent** undo stacks with the
//!   named-action refusal rule. The agent-undo bookkeeping (E7-S12) and the `undo`
//!   tool drive `History::with_agent_swap` / `History::agent_undo`.
//! - `can_generate` — whether generation tools are usable (signed in AND has
//!   credits, reference `AccountService.isSignedIn && hasCredits`). In M2 it is a
//!   plain field defaulting to `false`; Epic 9 wires the real account state.
//!
//! [`MediaManifest`]: palmier_model::MediaManifest
//! [`MediaAsset`]: palmier_model::MediaAsset
//! [`ToolDispatch::execute`]: crate::dispatch::ToolDispatch::execute

use palmier_history::History;
use palmier_model::{MediaLibrary, Timeline};

use crate::short_id::IdUniverse;

/// The single-owner editor state. Owned by the [`ToolExecutor`] behind a `Mutex`;
/// all tool calls serialize through that lock (the reference `@MainActor`).
///
/// [`ToolExecutor`]: crate::executor::ToolExecutor
pub struct EditorState {
    /// Timeline + media manifest + runtime asset catalog — the whole library the
    /// tools read and mutate, and the snapshot unit for undo.
    pub library: MediaLibrary,
    /// User + agent undo history over the [`Timeline`]. Agent edits land here via
    /// `with_agent_swap`; the `undo` tool calls `agent_undo` (E7-S12).
    pub history: History<Timeline>,
    /// Whether the account can run generation/upscale tools (reference
    /// `canGenerate` = signed-in AND has credits). M2 default `false`; Epic 9
    /// supplies the real value.
    pub can_generate: bool,
}

impl Default for EditorState {
    fn default() -> EditorState {
        EditorState::new()
    }
}

impl EditorState {
    /// A fresh, empty editor: empty library, empty history, generation disabled.
    pub fn new() -> EditorState {
        EditorState {
            library: MediaLibrary::new(),
            history: History::new(),
            can_generate: false,
        }
    }

    /// Build an editor over an existing [`MediaLibrary`] (the dominant constructor
    /// — a project is loaded, then tools operate on it).
    pub fn with_library(library: MediaLibrary) -> EditorState {
        EditorState {
            library,
            history: History::new(),
            can_generate: false,
        }
    }

    /// Convenience read of the timeline (reference `editor.timeline`).
    pub fn timeline(&self) -> &Timeline {
        &self.library.timeline
    }

    /// The id-universe snapshot for the current library state — every track/clip/
    /// caption-group/link-group/asset/folder id in one set (reference
    /// `currentIdUniverse`). Taken once per tool call by the executor.
    pub fn id_universe(&self) -> IdUniverse {
        IdUniverse::from_library(&self.library)
    }
}
