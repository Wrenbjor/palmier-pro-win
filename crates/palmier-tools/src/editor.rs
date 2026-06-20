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
    /// User + agent undo history over the **whole [`MediaLibrary`]** snapshot — the
    /// snapshot unit the LIBRARY tools (E7-S10: import/folder/rename/delete) touch.
    /// Those tools mutate folders/assets (and, for delete cascades, the timeline)
    /// beyond what [`history`](Self::history)'s `Timeline`-only swap can reverse, so
    /// they register their **one** agent-undo step here over the full library
    /// snapshot. The `undo` tool reverses whichever agent stack
    /// ([`history`](Self::history) or this one) holds the most-recent agent edit
    /// (tracked by [`last_agent_edit`](Self::last_agent_edit)). Reference
    /// `mediaLibraryUndoSnapshot` (palmier-project `MediaLibraryHistory`).
    pub lib_history: History<MediaLibrary>,
    /// Which agent stack received the most recent agent edit — so the single `undo`
    /// tool reverses the genuinely most-recent agent step across both
    /// [`history`](Self::history) (timeline) and [`lib_history`](Self::lib_history)
    /// (library). `None` ⇒ no agent edit this session.
    pub last_agent_edit: Option<AgentStack>,
    /// Whether the account can run generation/upscale tools (reference
    /// `canGenerate` = signed-in AND has credits). M2 default `false`; Epic 9
    /// supplies the real value.
    pub can_generate: bool,
    /// The generation backend seam (E9-S11). `None` until the host
    /// (`palmier-tauri`) wires a `palmier-gen`-backed gateway; the 4 generate tool
    /// bodies return "backend not available" when it is absent. Kept private so the
    /// generate bodies route through [`EditorState::generation_gateway`].
    generation_gateway: Option<Box<dyn crate::generate::GenerationGateway>>,
    /// The visual-search seam (E11-S10). `None` until the host wires a
    /// [`SearchIndexCoordinator`](palmier_search::SearchIndexCoordinator) + a real
    /// `ort` query-encoder behind it; `search_media`'s visual scope reports
    /// `visual_status: disabled` / empty when it is absent (default build). The spoken
    /// scope needs no gateway (it reads the disk-only transcript cache directly).
    visual_search_gateway: Option<Box<dyn crate::search::VisualSearchGateway>>,
}

/// Which agent undo stack an agent edit landed on (timeline vs whole-library). The
/// `undo` tool uses [`EditorState::last_agent_edit`] to reverse the genuinely most
/// recent agent step across both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStack {
    /// The [`Timeline`] agent stack ([`EditorState::history`]) — clip/text edits.
    Timeline,
    /// The [`MediaLibrary`] agent stack ([`EditorState::lib_history`]) — library ops.
    Library,
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
            lib_history: History::new(),
            last_agent_edit: None,
            can_generate: false,
            generation_gateway: None,
            visual_search_gateway: None,
        }
    }

    /// Build an editor over an existing [`MediaLibrary`] (the dominant constructor
    /// — a project is loaded, then tools operate on it).
    pub fn with_library(library: MediaLibrary) -> EditorState {
        EditorState {
            library,
            history: History::new(),
            lib_history: History::new(),
            last_agent_edit: None,
            can_generate: false,
            generation_gateway: None,
            visual_search_gateway: None,
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

    /// Wire the generation backend seam (E9-S11). Called by the host once the
    /// `palmier-gen`-backed gateway is built; tests inject a mock.
    pub fn set_generation_gateway(
        &mut self,
        gateway: Box<dyn crate::generate::GenerationGateway>,
    ) {
        self.generation_gateway = Some(gateway);
    }

    /// The wired generation gateway, if any (the 4 generate tool bodies route
    /// through this; `None` ⇒ "backend not available").
    pub fn generation_gateway(&self) -> Option<&dyn crate::generate::GenerationGateway> {
        self.generation_gateway.as_deref()
    }

    /// Wire the visual-search backend seam (E11-S10). Called by the host once the
    /// `palmier-search` coordinator + `ort` query-encoder are built; tests inject a mock.
    pub fn set_visual_search_gateway(
        &mut self,
        gateway: Box<dyn crate::search::VisualSearchGateway>,
    ) {
        self.visual_search_gateway = Some(gateway);
    }

    /// The wired visual-search gateway, if any (`search_media`'s visual scope routes
    /// through this; `None` ⇒ `visual_status: disabled` / empty hits).
    pub fn visual_search_gateway(&self) -> Option<&dyn crate::search::VisualSearchGateway> {
        self.visual_search_gateway.as_deref()
    }
}
