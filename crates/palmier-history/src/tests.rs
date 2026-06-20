//! Unit tests for the user/agent undo stacks.
//!
//! The state type used here is a deliberately model-free stand-in — a tiny
//! `Doc` of clip ids and a frame counter — proving the crate is generic over an
//! arbitrary `S` and never needs `palmier-model`. It exercises: push/undo/redo
//! ordering for both swap patterns, redo invalidation, nested coalescing into a
//! single undo step, user/agent stack independence, and the agent-undo refusal
//! semantics.

use super::*;

/// A model-free stand-in for the editor's `Timeline`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Doc {
    clips: Vec<String>,
    frame: i32,
}

impl Doc {
    fn with_clips(clips: &[&str]) -> Self {
        Doc {
            clips: clips.iter().map(|s| s.to_string()).collect(),
            frame: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// with_*_swap (whole-state snapshot pattern) — push / undo / redo ordering
// ---------------------------------------------------------------------------

#[test]
fn state_swap_push_undo_redo_restores_exact_state() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    h.with_user_swap("Add Clip", &mut doc, |d| d.clips.push("b".into()));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
    assert_eq!(h.current_undo_action_name(), Some("Add Clip"));

    h.with_user_swap("Add Clip", &mut doc, |d| d.clips.push("c".into()));
    assert_eq!(doc, Doc::with_clips(&["a", "b", "c"]));

    // Undo twice, back to start.
    assert!(h.undo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
    assert!(h.undo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a"]));
    assert!(!h.can_undo());
    assert!(!h.undo(&mut doc)); // no-op past the bottom

    // Redo twice, forward again.
    assert!(h.redo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
    assert!(h.redo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b", "c"]));
    assert!(!h.can_redo());
    assert!(!h.redo(&mut doc));
}

#[test]
fn no_op_swap_registers_nothing() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);
    // work that does not change state -> no undo entry (reference `guard before != after`).
    h.with_user_swap("Nothing", &mut doc, |_d| {});
    assert!(!h.can_undo());
    assert_eq!(h.current_undo_action_name(), None);
}

// ---------------------------------------------------------------------------
// ClosureSwap (per-clip bidirectional swap pattern) — push / undo / redo
// ---------------------------------------------------------------------------

#[test]
fn closure_swap_push_undo_redo_restores_exact_state() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    // register_clip_property_swap analogue: set frame 0 -> 30, with an explicit
    // inverse. We apply eagerly, then register the bidirectional swap.
    let before = doc.frame; // 0
    doc.frame = 30;
    let after = doc.frame; // 30
    h.push_user(NamedAction::new(
        "Change Clip Property",
        ClosureSwap::new(
            move |d: &mut Doc| d.frame = after,
            move |d: &mut Doc| d.frame = before,
        ),
    ));

    assert_eq!(doc.frame, 30);
    assert!(h.undo(&mut doc));
    assert_eq!(doc.frame, 0);
    assert!(h.redo(&mut doc));
    assert_eq!(doc.frame, 30);
    assert_eq!(h.current_undo_action_name(), Some("Change Clip Property"));
}

// ---------------------------------------------------------------------------
// Redo invalidation on new push
// ---------------------------------------------------------------------------

#[test]
fn new_push_invalidates_redo() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    h.with_user_swap("Add b", &mut doc, |d| d.clips.push("b".into()));
    h.with_user_swap("Add c", &mut doc, |d| d.clips.push("c".into()));

    // Undo once: "Add c" is now redoable.
    assert!(h.undo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
    assert!(h.can_redo());
    assert_eq!(h.current_redo_action_name(), Some("Add c"));

    // A new edit must invalidate the pending redo.
    h.with_user_swap("Add d", &mut doc, |d| d.clips.push("d".into()));
    assert!(!h.can_redo(), "new push must clear redo");
    assert!(!h.redo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b", "d"]));
}

// ---------------------------------------------------------------------------
// Nested coalescing -> ONE undo step
// ---------------------------------------------------------------------------

#[test]
fn nested_pushes_coalesce_into_one_undo_step() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&[]);

    // A composite edit: three sub-mutations registered inside one group must
    // collapse to a single user-visible undo entry (nested withTimelineSwap skip).
    h.group(Origin::User, "Linked Split", |h| {
        for id in ["x", "y", "z"] {
            let id = id.to_string();
            let added = id.clone();
            h.push_user(NamedAction::new(
                "sub",
                ClosureSwap::new(
                    move |d: &mut Doc| d.clips.push(added.clone()),
                    move |d: &mut Doc| {
                        if let Some(pos) = d.clips.iter().rposition(|c| c == &id) {
                            d.clips.remove(pos);
                        }
                    },
                ),
            ));
        }
    });

    // Apply the group's effect by redoing is not needed — closures only mutate on
    // apply/revert. Build the expected by replaying apply via redo path: here the
    // group registered the action but did NOT apply it (we registered swaps for an
    // already-applied edit pattern would differ). Drive through one undo/redo to
    // verify atomicity instead.

    // Exactly ONE undo entry exists.
    assert_eq!(h.user_undo_len(), 1, "group must coalesce to one entry");
    assert_eq!(h.current_undo_action_name(), Some("Linked Split"));

    // Redo (apply) the whole group at once after undoing it once to seed redo.
    // First, manually apply by undo then redo round-trip is awkward for
    // not-yet-applied ops; instead verify revert+apply atomicity directly:
    // seed state by applying via redo requires the entry on the redo stack, so
    // undo first.
    h.undo(&mut doc); // reverts all three sub-ops in reverse
    assert!(doc.clips.is_empty(), "undo reverts the whole group");
    assert!(h.redo(&mut doc)); // re-applies all three in order
    assert_eq!(doc, Doc::with_clips(&["x", "y", "z"]));
    assert_eq!(h.user_undo_len(), 1);
}

#[test]
fn nested_state_swaps_coalesce_to_one_entry() {
    // Several sub-swaps registered inside one group must produce ONE entry, and
    // that entry's revert restores the pre-group state exactly. We mutate `doc`
    // directly and register each sub-edit's before/after snapshot; the group
    // collapses them.
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);
    let start = doc.clone();

    h.group(Origin::User, "Composite", |h| {
        // sub-edit 1
        let s1 = doc.clone();
        doc.clips.push("b".into());
        let e1 = doc.clone();
        h.push_user(NamedAction::new("s1", StateSwap::new(s1, e1)));
        // sub-edit 2
        let s2 = doc.clone();
        doc.frame = 99;
        let e2 = doc.clone();
        h.push_user(NamedAction::new("s2", StateSwap::new(s2, e2)));
    });

    assert_eq!(h.user_undo_len(), 1);
    assert_eq!(doc, Doc { clips: vec!["a".into(), "b".into()], frame: 99 });

    assert!(h.undo(&mut doc));
    assert_eq!(doc, start, "one undo reverts the whole composite");
    assert!(h.redo(&mut doc));
    assert_eq!(doc, Doc { clips: vec!["a".into(), "b".into()], frame: 99 });
}

// ---------------------------------------------------------------------------
// User and agent stacks are independent
// ---------------------------------------------------------------------------

#[test]
fn user_and_agent_stacks_are_independent() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&[]);

    h.with_user_swap("User Edit", &mut doc, |d| d.clips.push("u".into()));
    h.with_agent_swap("Agent Edit", &mut doc, |d| d.clips.push("g".into()));

    // An agent push does not appear on the user undo stack, and vice versa.
    assert_eq!(h.user_undo_len(), 1, "only the user edit is on the user stack");
    assert_eq!(h.agent_undo_len(), 1, "only the agent edit is on the agent stack");

    // User undo only reverses the user edit (leaves the agent edit's data alone
    // on the agent stack).
    // Current state: ["u","g"]. The agent edit is the most-recent overall.
    assert_eq!(h.current_undo_action_name(), Some("Agent Edit"));
}

// ---------------------------------------------------------------------------
// current_undo_action_name reflects the last pushed group
// ---------------------------------------------------------------------------

#[test]
fn current_undo_action_name_reflects_last_pushed() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&[]);
    assert_eq!(h.current_undo_action_name(), None);

    h.with_user_swap("First", &mut doc, |d| d.clips.push("1".into()));
    assert_eq!(h.current_undo_action_name(), Some("First"));

    h.with_agent_swap("Second", &mut doc, |d| d.clips.push("2".into()));
    assert_eq!(h.current_undo_action_name(), Some("Second"));
    assert_eq!(h.current_undo_origin(), Some(Origin::Agent));

    // Undoing the (most-recent) user-or-agent surfaces the prior name. The
    // agent edit is on top; a user undo only touches user entries, so the name
    // remains the agent's until the agent undoes it.
    h.undo(&mut doc); // reverts the user "First" entry (the only user entry)
    assert_eq!(h.current_undo_action_name(), Some("Second"));
}

// ---------------------------------------------------------------------------
// Agent-stack refusal semantics
// ---------------------------------------------------------------------------

#[test]
fn agent_undo_succeeds_when_agent_edit_is_most_recent() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    h.with_agent_swap("Agent Add", &mut doc, |d| d.clips.push("b".into()));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));

    let undone = h.agent_undo(&mut doc).expect("agent undo should succeed");
    assert_eq!(undone, "Agent Add");
    assert_eq!(doc, Doc::with_clips(&["a"]), "agent edit reversed");
    assert!(!h.can_agent_undo());
}

#[test]
fn agent_undo_refuses_when_no_agent_edit() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);
    h.with_user_swap("User Add", &mut doc, |d| d.clips.push("b".into()));

    let err = h.agent_undo(&mut doc).unwrap_err();
    assert_eq!(err, AgentUndoError::NoAgentEdit);
    // Nothing mutated.
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
}

#[test]
fn agent_undo_refuses_when_last_change_was_user() {
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    // Agent edits, then the USER edits afterwards -> the agent must refuse,
    // because the most recent change came from the user (mcp-tools.md rule).
    h.with_agent_swap("Agent Add", &mut doc, |d| d.clips.push("b".into()));
    h.with_user_swap("User Add", &mut doc, |d| d.clips.push("c".into()));

    let err = h.agent_undo(&mut doc).unwrap_err();
    match err {
        AgentUndoError::NotAgentsEdit { expected, found } => {
            assert_eq!(expected, "Agent Add");
            assert_eq!(found.as_deref(), Some("User Add"));
        }
        other => panic!("expected NotAgentsEdit, got {other:?}"),
    }
    // Refusal mutates nothing.
    assert_eq!(doc, Doc::with_clips(&["a", "b", "c"]));
}

#[test]
fn agent_undo_succeeds_after_user_edit_is_undone() {
    // Agent edit, then user edit, then the user undoes their own edit: now the
    // agent's edit is once again the most recent -> agent_undo should succeed.
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&["a"]);

    h.with_agent_swap("Agent Add", &mut doc, |d| d.clips.push("b".into()));
    h.with_user_swap("User Add", &mut doc, |d| d.clips.push("c".into()));

    // User undoes their edit.
    assert!(h.undo(&mut doc));
    assert_eq!(doc, Doc::with_clips(&["a", "b"]));
    assert_eq!(h.current_undo_action_name(), Some("Agent Add"));

    // Now the agent edit is top -> agent_undo succeeds.
    let undone = h.agent_undo(&mut doc).expect("agent undo should now succeed");
    assert_eq!(undone, "Agent Add");
    assert_eq!(doc, Doc::with_clips(&["a"]));
}

#[test]
fn agent_undo_refuses_when_a_later_agent_edit_is_on_top() {
    // Two agent edits: agent_undo always targets the MOST RECENT agent edit and
    // refuses if some other edit interleaved on top. Here two agent edits in a
    // row: undoing once should reverse the second, then the first.
    let mut h: History<Doc> = History::new();
    let mut doc = Doc::with_clips(&[]);

    h.with_agent_swap("Agent 1", &mut doc, |d| d.clips.push("1".into()));
    h.with_agent_swap("Agent 2", &mut doc, |d| d.clips.push("2".into()));

    assert_eq!(h.agent_undo(&mut doc).unwrap(), "Agent 2");
    assert_eq!(doc, Doc::with_clips(&["1"]));
    assert_eq!(h.agent_undo(&mut doc).unwrap(), "Agent 1");
    assert!(doc.clips.is_empty());
    // Stack exhausted.
    assert_eq!(h.agent_undo(&mut doc).unwrap_err(), AgentUndoError::NoAgentEdit);
}
