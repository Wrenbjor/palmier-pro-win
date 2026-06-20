//! Agent undo stack + the `undo` tool (E7-S12; reference `ToolExecutor.swift`
//! lines 33-36, 82-96, and `withUndoGroup(actionName:)`).
//!
//! ## The agent stack is distinct from the user stack
//! Every mutating tool (E7-S6/S7/S8/S10) wraps its work in **one** named agent
//! undo step via [`agent_edit`]. That registers a single before/after swap on the
//! [`History`]'s **agent** stack ([`History::push_agent`]) and records its name in
//! the unified recency log, so:
//!
//! - the user undo stack is untouched (Ctrl+Z still only sees user edits), and
//! - the agent-undo refusal rule can compare the *current* most-recent change
//!   (across both stacks) against the name the agent pushed.
//!
//! ## The refusal rule (carry-forward — `mcp-tools.md` §"Agent undo stack")
//! The `undo` tool ([`undo`]) pops one agent step and reverses **one** agent edit,
//! but **refuses** when:
//! - the agent has no edit this session ([`AgentUndoError::NoAgentEdit`]), or
//! - the most recent change is **not** the agent's own — i.e. a **user edit** (or
//!   any other change) interleaved, so the editor's current undo-action name no
//!   longer matches the name the agent pushed
//!   ([`AgentUndoError::NotAgentsEdit`]).
//!
//! This is implemented in [`palmier_history::History::agent_undo`], which we call
//! here and surface as a [`ToolResult`].
//!
//! ## How a mutating tool registers exactly ONE agent step
//! [`agent_edit`] is the single seam the edit tools use. It:
//! 1. snapshots the timeline `before`,
//! 2. runs the mutation against a **scratch** [`History`] (whose own entries are
//!    discarded — palmier-edit's orchestration commands register on the history
//!    they're handed; we don't want their *user*-stack entries), and
//! 3. if (and only if) the timeline changed, pushes **one** named [`StateSwap`]
//!    (`before` → `after`) onto the real history's **agent** stack.
//!
//! That makes every edit tool exactly one agent-undo step, named with the
//! reference's `…(Agent)` action name, reversible by the `undo` tool, and subject
//! to the interleaved-user-edit refusal — without re-implementing any edit math
//! (palmier-edit's validated, atomic orchestration is reused verbatim).

use palmier_history::{History, NamedAction, StateSwap};
use palmier_model::Timeline;

use crate::editor::EditorState;
use crate::result::ToolResult;

/// Run a mutating edit tool's `work` as **one** named agent-undo step.
///
/// `work` mutates `state.library.timeline` (using palmier-edit's pure engines /
/// orchestration against the supplied scratch [`History`]) and returns a
/// `Result<ToolResult, String>` — `Err(msg)` becomes the contract error shape and
/// **registers no undo step** (the caller is responsible for leaving the timeline
/// unchanged on the error paths it controls; palmier-edit's orchestration is
/// atomic — it refuses with the timeline byte-unchanged).
///
/// On a successful run that **changed** the timeline, exactly one [`StateSwap`]
/// named `action_name` is pushed onto the agent stack. A no-op edit (timeline
/// unchanged) registers nothing — matching the reference's
/// `editor.timeline != before` guard.
pub fn agent_edit(
    state: &mut EditorState,
    action_name: &str,
    work: impl FnOnce(&mut Timeline, &mut History<Timeline>) -> Result<ToolResult, String>,
) -> ToolResult {
    let before = state.library.timeline.clone();
    // palmier-edit's orchestration commands register on the History they're handed.
    // We hand them a throwaway History so their *user*-stack entries are discarded;
    // the single agent-stack entry is registered below from the before/after diff.
    let mut scratch: History<Timeline> = History::new();
    let result = match work(&mut state.library.timeline, &mut scratch) {
        Ok(r) => r,
        Err(msg) => {
            // The body refused or failed. Restore defensively to the pre-call state
            // so a partial mutation can never leak (palmier-edit is atomic, but a
            // multi-step body in this crate may have mutated before hitting an
            // error — restore guarantees the all-or-none contract).
            state.library.timeline = before;
            return ToolResult::error(msg);
        }
    };

    // Register exactly one agent-undo step iff the timeline actually changed
    // (reference `editor.timeline != before`). Errors never reach here.
    if !result.is_error && state.library.timeline != before {
        let after = state.library.timeline.clone();
        state.history.push_agent(NamedAction::new(
            action_name.to_string(),
            StateSwap::new(before, after),
        ));
    }
    result
}

/// The `undo` tool (reference `ToolExecutor.undo`). Pops one agent-stack entry and
/// reverses **one** agent edit, refusing if the most recent change came from the
/// user (or otherwise no longer matches the pushed name) or the stack is empty.
///
/// Mirrors the reference messages exactly via [`History::agent_undo`].
pub fn undo(state: &mut EditorState) -> ToolResult {
    match state.history.agent_undo(&mut state.library.timeline) {
        Ok(name) => ToolResult::ok(format!(
            "Undid: {name}. The timeline is restored to its state before that edit; \
             re-read with get_timeline or get_transcript before editing again."
        )),
        Err(e) => ToolResult::error(e.to_string()),
    }
}
