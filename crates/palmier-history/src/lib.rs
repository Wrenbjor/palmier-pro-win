//! # palmier-history
//!
//! A generic named-action undo/redo system with **two separate stacks** — a
//! **user** stack and an **agent** stack — ported from the macOS reference's
//! `UndoManager` usage (`EditorViewModel+ClipMutations.swift` `withTimelineSwap`
//! / `registerClipStateSwap` / `registerClipPropertySwap`, and
//! `Agent/Tools/ToolExecutor.swift`'s `agentUndoStack`). See
//! `docs/reference/edit-engines.md` (UndoManager → palmier-history) and
//! `docs/reference/mcp-tools.md` (agent-undo refusal rule).
//!
//! ## Generic over state
//!
//! This crate is **generic over the state type `S`** and carries **no dependency
//! on `palmier-model`**. The concrete `Timeline`/`Clip` types live in
//! `palmier-model`; the orchestration layer (E3-S6) supplies them as the type
//! parameter `S` (for whole-timeline swaps) or inside boxed [`Reversible<S>`]
//! ops (for per-clip swaps). Nothing here knows what a clip *is* — an undo entry
//! is just a **named** pair of `apply` (redo) / `revert` (undo) operations over
//! some opaque state. This is what lets the crate build and be tested standalone
//! while `palmier-model` is still a skeleton on this branch.
//!
//! ## The two reference registration patterns
//!
//! Both map onto the same [`Reversible<S>`] abstraction:
//!
//! * **`with_timeline_swap`** — whole-state before/after snapshot, atomic; nested
//!   calls do **not** re-register (they coalesce into one user-visible step).
//!   Implemented by [`History::with_user_swap`] / [`History::with_agent_swap`]
//!   plus [`StateSwap`].
//! * **`register_clip_property_swap` / `register_clip_state_swap`** — bidirectional
//!   per-clip swaps. Implemented by pushing any custom [`Reversible<S>`] (e.g.
//!   [`ClosureSwap`] or [`StateSwap`] scoped to the clips) via [`History::push_user`]
//!   / [`History::push_agent`], optionally inside a [`History::group`] for
//!   nested-action coalescing.
//!
//! ## Agent-stack refusal rule
//!
//! The agent `undo` MCP tool (Epic 7 / SM-4) must refuse to undo a change that
//! the user made. We replicate the reference exactly: every undo group carries an
//! **action name**, the agent records the name it pushed, and
//! [`History::agent_undo`] refuses unless the **current** user-undo action name
//! ([`History::current_undo_action_name`]) still equals the name the agent pushed
//! — i.e. the most recent change must be the agent's own, not an interleaved user
//! edit. See [`AgentUndoError`].

mod action;
mod stack;

pub use action::{ClosureSwap, NamedAction, Reversible, StateSwap};
pub use stack::UndoStack;

/// Who originated an edit / which stack it lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A hand-edit by the user. Driven by Ctrl+Z / Ctrl+Shift+Z.
    User,
    /// An edit made by the assistant through the MCP tools.
    Agent,
}

/// Reasons the agent `undo` tool refuses, mirroring the reference's
/// `ToolExecutor.undo` error messages
/// (`Agent/Tools/ToolExecutor.swift:82-96`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUndoError {
    /// `agentUndoStack` was empty — the assistant made no undoable edit this
    /// session. The user's own edits are theirs to undo.
    NoAgentEdit,
    /// There is nothing on the user undo stack to undo at all.
    NothingToUndo,
    /// The most recent change was **not** the agent's: the current undo action
    /// name no longer matches the name the agent pushed (a user edit, or another
    /// agent edit, interleaved). Carries `(expected, found)` for the toast.
    NotAgentsEdit {
        /// The action name the agent expected to be on top.
        expected: String,
        /// The action name actually on top of the user undo stack (if any).
        found: Option<String>,
    },
}

impl core::fmt::Display for AgentUndoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AgentUndoError::NoAgentEdit => write!(
                f,
                "No assistant edit to undo this session. The user's own edits are theirs to undo."
            ),
            AgentUndoError::NothingToUndo => write!(f, "Nothing to undo."),
            AgentUndoError::NotAgentsEdit { found, .. } => {
                let name = found.as_deref().unwrap_or("a user edit");
                write!(
                    f,
                    "The most recent change ('{name}') wasn't made by the assistant — not undoing it."
                )
            }
        }
    }
}

impl std::error::Error for AgentUndoError {}

/// The complete undo/redo history for one document: a **user** stack and a
/// separate **agent** stack over the same state type `S`.
///
/// Both stacks operate on the *same* live state `S` (the document's timeline),
/// but their entries never mix: an agent edit never appears on the user stack
/// and vice versa. Ctrl+Z / Ctrl+Shift+Z drive the **user** stack only
/// ([`undo`](Self::undo) / [`redo`](Self::redo)); the agent tools drive the
/// agent stack ([`agent_undo`](Self::agent_undo)).
///
/// Both `undo`/`redo` mutate the state in place (the reversible ops own the
/// before/after data), so callers pass `&mut S`.
pub struct History<S> {
    user: UndoStack<S>,
    agent: UndoStack<S>,
    /// Open coalescing group: depth + the entry being accumulated. Mirrors
    /// `withTimelineSwap`'s nested-suppression — only the outermost group
    /// registers. `origin` records which stack the group lands on.
    group: Option<OpenGroup<S>>,
    /// The conceptual *shared* undo-name stack — the order edits actually
    /// happened across BOTH stacks, top = most recent change overall. This is
    /// the reference's `undoManager.undoActionName`: a single value reflecting
    /// the most recent change regardless of who made it. `current_undo_action_name`
    /// reads its top, and `agent_undo`'s refusal rule compares against it (so an
    /// interleaved USER edit, which lands here on top, makes the agent refuse).
    ///
    /// Storage of the reversible ops stays split across `user`/`agent` (so the
    /// two stacks are genuinely independent), but the *name* ordering is unified
    /// here to faithfully model the single shared `undoActionName`.
    recent: Vec<RecentMark>,
}

/// One entry in the unified recency log: which stack it lives on + its name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentMark {
    origin: Origin,
    name: String,
}

struct OpenGroup<S> {
    origin: Origin,
    name: String,
    depth: usize,
    action: Option<NamedAction<S>>,
}

impl<S> Default for History<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> History<S> {
    /// A fresh, empty history.
    pub fn new() -> Self {
        History {
            user: UndoStack::new(),
            agent: UndoStack::new(),
            group: None,
            recent: Vec::new(),
        }
    }

    // ---- introspection -----------------------------------------------------

    /// The action name of the **most recent change overall** (across both the
    /// user and agent stacks) — the single value the reference exposes as
    /// `undoManager.undoActionName`. `None` if nothing has been done yet.
    ///
    /// Exposed so Epic 7 / SM-4 can enforce the refuse-after-user-edit rule: the
    /// agent compares the name it pushed against this; if a user edit interleaved,
    /// this now names that user edit and the agent refuses.
    pub fn current_undo_action_name(&self) -> Option<&str> {
        self.recent.last().map(|m| m.name.as_str())
    }

    /// The action name the **user** `redo` would reapply, if any.
    pub fn current_redo_action_name(&self) -> Option<&str> {
        self.user.top_redo_name()
    }

    /// The [`Origin`] of the most recent change overall, if any.
    pub fn current_undo_origin(&self) -> Option<Origin> {
        self.recent.last().map(|m| m.origin)
    }

    /// Whether the user stack has anything to undo.
    pub fn can_undo(&self) -> bool {
        self.user.can_undo()
    }

    /// Whether the user stack has anything to redo.
    pub fn can_redo(&self) -> bool {
        self.user.can_redo()
    }

    /// Whether the agent has an edit it could attempt to undo this session.
    pub fn can_agent_undo(&self) -> bool {
        self.agent.can_undo()
    }

    /// Number of entries on the user undo stack (test/introspection helper).
    pub fn user_undo_len(&self) -> usize {
        self.user.undo_len()
    }

    /// Number of entries on the agent undo stack (test/introspection helper).
    pub fn agent_undo_len(&self) -> usize {
        self.agent.undo_len()
    }

    // ---- push --------------------------------------------------------------

    /// Push an already-built named action onto the **user** undo stack. Clears
    /// the user redo stack (a new edit invalidates redo — the reference's
    /// `registerUndo` does the same). If a coalescing group is open, the action
    /// is folded into it instead (see [`group`](Self::group)).
    pub fn push_user(&mut self, action: NamedAction<S>) {
        self.push(Origin::User, action);
    }

    /// Push an already-built named action onto the **agent** undo stack and
    /// record its name for the agent-undo refusal rule. Clears the agent redo
    /// stack.
    pub fn push_agent(&mut self, action: NamedAction<S>) {
        self.push(Origin::Agent, action);
    }

    fn push(&mut self, origin: Origin, action: NamedAction<S>) {
        if let Some(group) = self.group.as_mut() {
            // Nested registration: fold into the open group rather than
            // registering a separate undo step. Mirrors `withTimelineSwap`
            // skipping registration while an outer swap is still suppressing.
            debug_assert_eq!(
                group.origin, origin,
                "cannot mix user and agent edits inside one coalescing group"
            );
            match group.action.as_mut() {
                None => group.action = Some(action),
                Some(existing) => existing.coalesce(action),
            }
            return;
        }
        self.push_resolved(origin, action);
    }

    // ---- coalescing groups (nested withTimelineSwap) -----------------------

    /// Run `work` as **one** coalesced undo step. Nested calls do not register a
    /// separate entry — every [`push_user`](Self::push_user) /
    /// [`push_agent`](Self::push_agent) (and every swap helper) made inside
    /// `work`, at any nesting depth, folds into a single named action that
    /// becomes one user-visible undo step. This is the generic form of the
    /// reference's nested-`withTimelineSwap` skip
    /// (`edit-engines.md` lines 248-249).
    ///
    /// If `work` produces no change (nothing pushed), nothing is registered.
    pub fn group<R>(
        &mut self,
        origin: Origin,
        name: impl Into<String>,
        work: impl FnOnce(&mut Self) -> R,
    ) -> R {
        match self.group.as_mut() {
            Some(group) => {
                // Already inside a group: just deepen — do not open a new one,
                // do not change the outer group's name. The outer registration
                // captures everything.
                debug_assert_eq!(
                    group.origin, origin,
                    "nested group origin must match the enclosing group"
                );
                group.depth += 1;
                let out = work(self);
                if let Some(group) = self.group.as_mut() {
                    group.depth -= 1;
                }
                out
            }
            None => {
                self.group = Some(OpenGroup {
                    origin,
                    name: name.into(),
                    depth: 0,
                    action: None,
                });
                let out = work(self);
                // Close the outermost group and register the accumulated action.
                if let Some(group) = self.group.take() {
                    if let Some(mut action) = group.action {
                        // The group's name overrides individual push names so the
                        // composite reads as one labelled step (matches the
                        // reference's outer `setActionName`).
                        action.set_name(group.name);
                        self.push_resolved(origin, action);
                    }
                }
                out
            }
        }
    }

    /// Push that always registers (never folds) — used to land a closed group's
    /// accumulated action on the right stack, and record its name in the unified
    /// recency log. A new edit invalidates the matching redo stack (done inside
    /// [`UndoStack::push`]).
    fn push_resolved(&mut self, origin: Origin, action: NamedAction<S>) {
        let name = action.name().to_owned();
        match origin {
            Origin::User => self.user.push(action),
            Origin::Agent => self.agent.push(action),
        }
        self.recent.push(RecentMark { origin, name });
    }

    /// Remove the topmost recency mark of `origin` (the one a stack's `undo` just
    /// reversed), preserving the rest of the order.
    fn pop_recent(&mut self, origin: Origin) {
        if let Some(pos) = self.recent.iter().rposition(|m| m.origin == origin) {
            self.recent.remove(pos);
        }
    }

    // ---- undo / redo (user stack) ------------------------------------------

    /// Undo the most recent **user** edit, mutating `state` back to its prior
    /// value. The undone entry moves to the user redo stack. No-op (returns
    /// `false`) if there is nothing to undo.
    pub fn undo(&mut self, state: &mut S) -> bool {
        if self.user.undo(state) {
            self.pop_recent(Origin::User);
            true
        } else {
            false
        }
    }

    /// Redo the most recently undone **user** edit. No-op if the redo stack is
    /// empty. Re-records the redone edit at the top of the unified recency log
    /// (it is once again the change the user's `undo` would reverse).
    pub fn redo(&mut self, state: &mut S) -> bool {
        let name = self.user.top_redo_name().map(str::to_owned);
        if self.user.redo(state) {
            if let Some(name) = name {
                self.recent.push(RecentMark {
                    origin: Origin::User,
                    name,
                });
            }
            true
        } else {
            false
        }
    }

    // ---- agent undo --------------------------------------------------------

    /// The agent `undo` tool: reverse the agent's most recent edit, but **refuse
    /// if the most recent change came from the user** (or otherwise no longer
    /// matches the name the agent pushed). Mirrors
    /// `ToolExecutor.undo` (`mcp-tools.md`: "refuses unless the editor's current
    /// `undoActionName` equals the pushed name; refuses if the most recent change
    /// came from the user").
    ///
    /// On success the edit is reversed on `state`, popped from the agent stack
    /// and the unified recency log. Returns the action name that was undone.
    pub fn agent_undo(&mut self, state: &mut S) -> Result<String, AgentUndoError> {
        // The agent must have an edit it could undo this session (the reference's
        // `agentUndoStack.last`). The name it would expect on top is the agent
        // stack's own top-of-undo name.
        let expected = self
            .agent
            .top_undo_name()
            .map(str::to_owned)
            .ok_or(AgentUndoError::NoAgentEdit)?;

        // Nothing on the user-facing (shared) stack to undo at all.
        if self.recent.is_empty() {
            return Err(AgentUndoError::NothingToUndo);
        }

        // The refusal rule: the *current* most-recent change overall must still
        // be the agent's own edit. If a user edit (or anything) interleaved, the
        // top of `recent` is no longer this agent edit -> refuse, mutate nothing.
        match self.recent.last() {
            Some(mark) if mark.origin == Origin::Agent && mark.name == expected => {}
            other => {
                return Err(AgentUndoError::NotAgentsEdit {
                    expected,
                    found: other.map(|m| m.name.clone()),
                });
            }
        }

        // Reverse the agent's edit and drop its recency mark.
        self.agent.undo(state);
        self.pop_recent(Origin::Agent);
        Ok(expected)
    }

    // ---- whole-state swap convenience (with_timeline_swap) -----------------

    /// The generic `with_timeline_swap`: snapshot `state`, run `work`, and if the
    /// state changed, register **one** undo entry (named `name`) that swaps the
    /// whole state between before/after. Nested calls coalesce — an inner
    /// `with_user_swap` inside an outer one does not register its own entry.
    ///
    /// Requires `S: Clone + PartialEq` so it can snapshot and detect no-ops
    /// exactly like the reference (`guard before != after else { return }`).
    pub fn with_user_swap<R>(
        &mut self,
        name: impl Into<String>,
        state: &mut S,
        work: impl FnOnce(&mut S) -> R,
    ) -> R
    where
        S: Clone + PartialEq + Send + 'static,
    {
        self.with_swap(Origin::User, name, state, work)
    }

    /// The agent-stack variant of [`with_user_swap`](Self::with_user_swap):
    /// registers the resulting entry on the agent stack (and records its name for
    /// the agent-undo rule).
    pub fn with_agent_swap<R>(
        &mut self,
        name: impl Into<String>,
        state: &mut S,
        work: impl FnOnce(&mut S) -> R,
    ) -> R
    where
        S: Clone + PartialEq + Send + 'static,
    {
        self.with_swap(Origin::Agent, name, state, work)
    }

    fn with_swap<R>(
        &mut self,
        origin: Origin,
        name: impl Into<String>,
        state: &mut S,
        work: impl FnOnce(&mut S) -> R,
    ) -> R
    where
        S: Clone + PartialEq + Send + 'static,
    {
        let name = name.into();
        // Nested: just run the work; the open group will register one entry from
        // the outer swap's before/after snapshot. We still snapshot at the outer
        // level only.
        if self.group.is_some() {
            return work(state);
        }
        let before = state.clone();
        let out = work(state);
        if *state == before {
            // No change — register nothing (reference `guard before != after`).
            return out;
        }
        let after = state.clone();
        let action = NamedAction::new(name, StateSwap::new(before, after));
        self.push_resolved(origin, action);
        out
    }
}

#[cfg(test)]
mod tests;
