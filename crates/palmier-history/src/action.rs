//! Reversible actions and the named-action wrapper.
//!
//! An undo entry is a **named** reversible op over some opaque state `S`. The
//! macOS reference registers undo via self-re-registering closures
//! (`registerUndo(withTarget:)` that re-registers its inverse); in Rust we model
//! the same thing as a [`Reversible<S>`] that knows how to `apply` (redo) and
//! `revert` (undo) against `&mut S`. The crate stays generic: it never names a
//! concrete model type.

/// A reversible operation over state `S`.
///
/// * [`revert`](Reversible::revert) undoes the op (restores the prior state).
/// * [`apply`](Reversible::apply) re-does it (re-applies the change).
///
/// Both take `&mut S` and own whatever data they need (e.g. before/after
/// snapshots, or a clip id + before/after clip). Implementations must be exact
/// inverses so undo→redo→undo round-trips are stable.
pub trait Reversible<S> {
    /// Undo: mutate `state` back to how it was before this action.
    fn revert(&self, state: &mut S);
    /// Redo: re-apply this action to `state`.
    fn apply(&self, state: &mut S);
}

/// The simplest reversible: hold the whole-state `before`/`after` snapshots and
/// swap. This is the generic form of the reference's `registerTimelineSwap`
/// (whole-`Timeline` before/after) and is what
/// [`with_user_swap`](crate::History::with_user_swap) builds. Also serviceable
/// for the per-clip swaps when `S` is scoped to the affected clips.
///
/// Requires `S: Clone` so `revert`/`apply` can write the stored snapshot back.
pub struct StateSwap<S> {
    before: S,
    after: S,
}

impl<S> StateSwap<S> {
    /// A swap between the `before` and `after` states.
    pub fn new(before: S, after: S) -> Self {
        StateSwap { before, after }
    }
}

impl<S: Clone> Reversible<S> for StateSwap<S> {
    fn revert(&self, state: &mut S) {
        *state = self.before.clone();
    }
    fn apply(&self, state: &mut S) {
        *state = self.after.clone();
    }
}

/// A reversible built from two closures: `apply` (redo) and `revert` (undo).
/// Use this for the bidirectional per-clip swaps
/// (`register_clip_property_swap` / `register_clip_state_swap`) where you want to
/// touch only the affected clips rather than snapshot the whole state — e.g.
/// `ClosureSwap::new(move |t| set_clip(t, &after), move |t| set_clip(t, &before))`.
pub struct ClosureSwap<S> {
    apply: Box<dyn Fn(&mut S) + Send>,
    revert: Box<dyn Fn(&mut S) + Send>,
}

impl<S> ClosureSwap<S> {
    /// Build from an `apply` (redo) closure and a `revert` (undo) closure.
    ///
    /// The closures are `Send` so a `History` (and any `EditorState`/executor
    /// owning one) can be shared across threads behind a `Mutex` — required by the
    /// Epic 7 MCP server (axum's multi-threaded runtime serializes tool calls
    /// through one `Mutex<EditorState>`, which needs the guarded state to be
    /// `Send`).
    pub fn new(
        apply: impl Fn(&mut S) + Send + 'static,
        revert: impl Fn(&mut S) + Send + 'static,
    ) -> Self {
        ClosureSwap {
            apply: Box::new(apply),
            revert: Box::new(revert),
        }
    }
}

impl<S> Reversible<S> for ClosureSwap<S> {
    fn revert(&self, state: &mut S) {
        (self.revert)(state);
    }
    fn apply(&self, state: &mut S) {
        (self.apply)(state);
    }
}

/// A named undo entry: an action name (the reference's `setActionName`) plus one
/// or more reversible ops. Multiple ops appear only when actions were
/// **coalesced** into a single user-visible step (nested `withTimelineSwap`):
/// `revert` undoes them in reverse order, `apply` redoes them in forward order,
/// so the whole group is one atomic undo/redo.
pub struct NamedAction<S> {
    name: String,
    /// Ops in the order they were applied. Coalesced groups hold several; a plain
    /// action holds one. The boxed ops are `+ Send` so a `History` can live behind
    /// a `Mutex` shared across threads (the Epic 7 MCP server requirement).
    ops: Vec<Box<dyn Reversible<S> + Send>>,
}

impl<S> NamedAction<S> {
    /// A named action wrapping a single reversible op.
    pub fn new(name: impl Into<String>, op: impl Reversible<S> + Send + 'static) -> Self {
        NamedAction {
            name: name.into(),
            ops: vec![Box::new(op)],
        }
    }

    /// A named action from an already-boxed op (lets callers erase the type).
    pub fn from_boxed(name: impl Into<String>, op: Box<dyn Reversible<S> + Send>) -> Self {
        NamedAction {
            name: name.into(),
            ops: vec![op],
        }
    }

    /// This action's name (the value compared by the agent-undo refusal rule).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rename this action — used when a coalescing group relabels its composite
    /// with the group's own name (the outer `setActionName`).
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Fold `other`'s ops onto the end of this action's ops (coalescing). The
    /// name is left unchanged here; the group closing it relabels via
    /// [`set_name`](Self::set_name).
    pub fn coalesce(&mut self, other: NamedAction<S>) {
        self.ops.extend(other.ops);
    }

    /// Number of coalesced ops (test/introspection helper).
    pub fn op_count(&self) -> usize {
        self.ops.len()
    }
}

impl<S> Reversible<S> for NamedAction<S> {
    fn revert(&self, state: &mut S) {
        // Undo in reverse application order.
        for op in self.ops.iter().rev() {
            op.revert(state);
        }
    }
    fn apply(&self, state: &mut S) {
        // Redo in forward application order.
        for op in self.ops.iter() {
            op.apply(state);
        }
    }
}
