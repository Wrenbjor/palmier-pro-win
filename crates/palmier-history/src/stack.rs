//! A single undo/redo stack of [`NamedAction`]s, with redo-invalidation on push.
//!
//! [`History`](crate::History) owns two of these (user + agent). The stack itself
//! is origin-agnostic; the separation and the agent refusal rule live in
//! `History`.

use crate::action::{NamedAction, Reversible};

/// A LIFO undo stack plus its redo stack, over state `S`.
///
/// * [`push`](Self::push) registers a new action and **clears the redo stack**
///   (a fresh edit invalidates any pending redo — matching the reference, where
///   `registerUndo` after the undo stack diverges drops the redo branch).
/// * [`undo`](Self::undo) pops the top action, reverts it against `state`, and
///   moves it to the redo stack.
/// * [`redo`](Self::redo) pops the top redo action, re-applies it, and moves it
///   back to the undo stack.
pub struct UndoStack<S> {
    undo: Vec<NamedAction<S>>,
    redo: Vec<NamedAction<S>>,
}

impl<S> Default for UndoStack<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> UndoStack<S> {
    /// A fresh, empty stack.
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Register a new action on the undo stack and **invalidate redo**.
    pub fn push(&mut self, action: NamedAction<S>) {
        self.undo.push(action);
        self.redo.clear();
    }

    /// The action name `undo` would currently reverse (top of the undo stack).
    pub fn top_undo_name(&self) -> Option<&str> {
        self.undo.last().map(NamedAction::name)
    }

    /// The action name `redo` would reapply (top of the redo stack).
    pub fn top_redo_name(&self) -> Option<&str> {
        self.redo.last().map(NamedAction::name)
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of undoable entries.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Number of redoable entries.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Undo the top action against `state`, moving it to the redo stack. Returns
    /// `false` (no-op) if the undo stack is empty.
    pub fn undo(&mut self, state: &mut S) -> bool {
        match self.undo.pop() {
            Some(action) => {
                action.revert(state);
                self.redo.push(action);
                true
            }
            None => false,
        }
    }

    /// Redo the top undone action against `state`, moving it back to the undo
    /// stack. Returns `false` (no-op) if the redo stack is empty.
    pub fn redo(&mut self, state: &mut S) -> bool {
        match self.redo.pop() {
            Some(action) => {
                action.apply(state);
                self.undo.push(action);
                true
            }
            None => false,
        }
    }
}
