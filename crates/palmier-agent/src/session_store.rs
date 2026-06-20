//! Session persistence seam: read/write [`ChatSession`] to/from
//! `<project>/chat/<uuid>.json`.
//!
//! Ports `ChatSessionStore` (`Agent/ChatSessionStore.swift`) — the JSON
//! load/encode half. **Ruling #3** (phase0-reconciliation): the session
//! directory is **`chat/`** (NOT FOUNDATION's `chatsessions/`). Each session is
//! one `<uuid>.json` file.
//!
//! Encoding matches the reference exactly: **ISO-8601 dates + pretty-print +
//! sorted keys** (`agent-panel.md` lines 158-159). serde_json (without the
//! `preserve_order` feature) is `BTreeMap`-backed, so `to_string_pretty` already
//! emits sorted keys deterministically; `ChatSession::updated_at` routes through
//! `serde_date::iso8601`.
//!
//! ## Scaffold scope
//! This story lands the file read/write + filename helpers and the
//! load-filter/sort/encode rules as pure functions. The **save trigger** (on
//! document save, ruling #4 — sessions are NOT written eagerly) and the
//! tab/sync orchestration are E8-S7, wired into the project save lifecycle.

use crate::model::{to_canonical_json, ChatSession};
use std::io;
use std::path::{Path, PathBuf};

/// The session directory name under a project (ruling #3: `chat/`, not
/// `chatsessions/`).
pub const CHAT_DIR_NAME: &str = "chat";

/// The `chat/` directory under a project root.
#[must_use]
pub fn chat_dir(project_root: &Path) -> PathBuf {
    project_root.join(CHAT_DIR_NAME)
}

/// The on-disk path for a session: `<project>/chat/<uuid>.json`.
#[must_use]
pub fn session_path(project_root: &Path, session: &ChatSession) -> PathBuf {
    chat_dir(project_root).join(format!("{}.json", session.id))
}

/// Encode a session to its canonical bytes: **pretty-printed + sorted keys +
/// ISO-8601 dates** (reference `ChatSessionStore.encoder`).
///
/// # Errors
/// Propagates a `serde_json` error (only on a non-serializable value, which the
/// model precludes).
pub fn encode_session(session: &ChatSession) -> Result<String, serde_json::Error> {
    to_canonical_json(session)
}

/// Write one **non-empty** session to `<project>/chat/<uuid>.json`, creating the
/// `chat/` dir if needed (reference `captureSaveSnapshot` writes non-empty
/// sessions only — `agent-panel.md` line 158).
///
/// Empty-message sessions are skipped (returns `Ok(None)`); a written session
/// returns `Ok(Some(path))`.
///
/// # Errors
/// I/O errors creating the directory or writing the file; encode errors.
pub fn write_session(project_root: &Path, session: &ChatSession) -> io::Result<Option<PathBuf>> {
    if session.is_empty() {
        // Filtered on save (matches the load-side filter) — no file emitted.
        return Ok(None);
    }
    let dir = chat_dir(project_root);
    std::fs::create_dir_all(&dir)?;
    let path = session_path(project_root, session);
    let bytes = encode_session(session).map_err(io::Error::other)?;
    std::fs::write(&path, bytes)?;
    Ok(Some(path))
}

/// Write a whole set of sessions (the save snapshot), skipping empty ones.
/// Returns the paths actually written (reference `captureSaveSnapshot`).
///
/// # Errors
/// First I/O / encode error aborts and propagates.
pub fn write_sessions(project_root: &Path, sessions: &[ChatSession]) -> io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for s in sessions {
        if let Some(path) = write_session(project_root, s)? {
            written.push(path);
        }
    }
    Ok(written)
}

/// Read + decode every `<project>/chat/*.json`, **dropping empty-message
/// sessions**, sorted by `updated_at` **descending** (reference `loadSessions`,
/// minus the "prepend a fresh open session as current" step, which is tab
/// orchestration owned by E8-S7).
///
/// A missing `chat/` dir yields an empty list (a brand-new project). Files that
/// fail to decode are skipped (reference `compactMap` / `try?`), so one corrupt
/// session never breaks loading the rest.
#[must_use]
pub fn load_sessions(project_root: &Path) -> Vec<ChatSession> {
    let dir = chat_dir(project_root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<ChatSession> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|s| serde_json::from_str::<ChatSession>(&s).ok())
        .filter(|s| !s.is_empty())
        .collect();
    // Most-recently-updated first (reference sort `updatedAt` desc).
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentContentBlock, AgentMessage, ChatSession, Role};
    use time::macros::datetime;
    use uuid::Uuid;

    fn session_with_text(id: Uuid, when: time::OffsetDateTime, text: &str) -> ChatSession {
        ChatSession {
            id,
            title: text.to_string(),
            updated_at: when,
            messages: vec![AgentMessage::new(
                Role::User,
                vec![AgentContentBlock::text(text)],
            )],
            is_open: false,
        }
    }

    fn temp_project() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("palmier-agent-test-{}", Uuid::new_v4()));
        p
    }

    #[test]
    fn paths_use_chat_dir_and_uuid() {
        let root = Path::new("C:/projects/demo");
        let s = ChatSession::new();
        assert_eq!(chat_dir(root), Path::new("C:/projects/demo/chat"));
        let path = session_path(root, &s);
        assert!(path.ends_with(format!("{}.json", s.id)));
        assert!(path.to_string_lossy().contains("chat"));
    }

    #[test]
    fn write_then_load_round_trips_content_stable() {
        let root = temp_project();
        let id = uuid::uuid!("55555555-5555-5555-5555-555555555555");
        let mut s = session_with_text(id, datetime!(2026-06-20 10:00:00 UTC), "trim the intro");
        s.messages[0].id = uuid::uuid!("66666666-6666-6666-6666-666666666666");

        let written = write_session(&root, &s).unwrap();
        assert!(written.is_some());

        let loaded = load_sessions(&root);
        assert_eq!(loaded.len(), 1);
        // Content-stable: same id/title/messages/updated_at survive the round-trip.
        assert_eq!(loaded[0].id, s.id);
        assert_eq!(loaded[0].title, s.title);
        assert_eq!(loaded[0].messages, s.messages);
        assert_eq!(loaded[0].updated_at, s.updated_at);

        // Re-encode is byte-identical to the first encode.
        assert_eq!(encode_session(&loaded[0]).unwrap(), encode_session(&s).unwrap());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_drops_empty_sessions_and_sorts_desc() {
        let root = temp_project();
        let older = session_with_text(
            uuid::uuid!("77777777-7777-7777-7777-777777777777"),
            datetime!(2026-06-19 10:00:00 UTC),
            "older",
        );
        let newer = session_with_text(
            uuid::uuid!("88888888-8888-8888-8888-888888888888"),
            datetime!(2026-06-20 10:00:00 UTC),
            "newer",
        );
        // An empty session that must be dropped on load.
        let mut empty = ChatSession::new();
        empty.id = uuid::uuid!("99999999-9999-9999-9999-999999999999");
        empty.messages.clear();

        write_session(&root, &older).unwrap();
        write_session(&root, &newer).unwrap();
        // Force-write the empty one bypassing the save-side filter to prove the
        // LOAD-side filter also drops it.
        std::fs::create_dir_all(chat_dir(&root)).unwrap();
        std::fs::write(session_path(&root, &empty), encode_session(&empty).unwrap()).unwrap();

        let loaded = load_sessions(&root);
        // Empty dropped; two remain, newest first.
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].title, "newer");
        assert_eq!(loaded[1].title, "older");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_emits_no_file_for_empty_session() {
        let root = temp_project();
        let mut empty = ChatSession::new();
        empty.messages.clear();
        let written = write_session(&root, &empty).unwrap();
        assert!(written.is_none());
        // Nothing was created.
        assert!(load_sessions(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_missing_dir_is_empty() {
        let root = temp_project(); // never created
        assert!(load_sessions(&root).is_empty());
    }

    #[test]
    fn write_sessions_skips_empty_and_returns_written_paths() {
        let root = temp_project();
        let s1 = session_with_text(
            uuid::uuid!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            datetime!(2026-06-20 10:00:00 UTC),
            "one",
        );
        let mut empty = ChatSession::new();
        empty.messages.clear();
        let written = write_sessions(&root, &[s1.clone(), empty]).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(load_sessions(&root).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
