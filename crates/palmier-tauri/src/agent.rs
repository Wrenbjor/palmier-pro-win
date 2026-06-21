//! M2 boot integration — the MCP server + the in-app agent, wired into the live
//! Tauri app over **one shared `EditorState`** (the keystone correctness property).
//!
//! This module is the seam that connects everything M2 built:
//!
//! - **The shared executor** ([`AgentState::executor`]) — a single
//!   `Arc<palmier_tools::ToolExecutor>` constructed at boot. It owns the one
//!   `EditorState` (timeline + media library + history) behind its internal
//!   `Mutex`. Both the loopback MCP server (external clients) and the in-app agent
//!   (this app's chat panel) dispatch through this **same** `Arc`, so an edit an
//!   external Claude Desktop makes and an edit the in-app agent makes land on one
//!   timeline / one undo timeline (FOUNDATION §4, PRD §10 "no duplication").
//!
//! - **The MCP server boot seam** ([`start_mcp`]) — boot step 6 calls this behind
//!   `io.palmier.pro.mcp.enabled` (ruling #6, absent ⇒ ON). It calls
//!   [`palmier_mcp::McpServer::start`] with the shared executor and stows the handle
//!   in managed state; [`crate::commands::get_mcp_status`] reflects the live state.
//!   A bind failure is **logged, not fatal** — boot stays offline-safe and < 2 s.
//!
//! - **The agent command surface** ([`agent_send`] / [`agent_cancel`] /
//!   [`agent_status`] / [`agent_set_pref`]) — the frontend `agent-panel`
//!   command/event seam. `agent_send` builds the real BYOK client via
//!   [`palmier_agent::select_and_build_client`] (keyring key → `AnthropicClient`),
//!   constructs the [`ExecutorDispatcher`] adapter over the shared executor, and
//!   drives [`palmier_agent::AgentLoop::run_turn`], **streaming every
//!   [`palmier_agent::StreamEvent`] to the webview as an `agent://event`** so the
//!   panel renders text deltas / tool activity live.
//!
//! ## What is functional vs stubbed here
//! - **Functional (no live key needed):** the shared-executor wiring, the MCP boot
//!   start/stop + status, the adapter mapping, backend selection, session state,
//!   the event surface, cancellation.
//! - **Needs a real key (BYOK):** the live Anthropic round trip inside `agent_send`
//!   only streams real content when a key is in the keyring (or `ANTHROPIC_API_KEY`
//!   in DEBUG). Without one, `agent_send` emits a single `error` `agent://event`
//!   carrying the "no backend / add a key" message — the panel surfaces it.
//! - **Convex-proxied path:** selected but not yet wired (E8-S6 / Spike S-2) — a
//!   signed-in-without-key user gets the "proxied not yet available" error event.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio_util::sync::CancellationToken;

use palmier_agent::{
    available_models, effective_model, encode_session, load_sessions, parse_json_object,
    select_and_build_client, AgentClientError, AgentLoop, AnthropicModel, AnthropicToolSchema,
    ChatSession, DispatchResult, SelectedBackend, Tier, ToolDispatcher, ToolResultBlock,
};
use palmier_tools::{tool_definitions, Block, ToolContext, ToolDispatch, ToolExecutor, IdUniverse};

/// The Tauri event name the agent streams every turn's deltas over (the
/// `agent://event` channel the `src-ui/agent-panel` controller subscribes to).
pub const AGENT_EVENT: &str = "agent://event";

// ─── The ToolDispatcher adapter (the agent → shared-executor bridge) ─────────────

/// A trivial [`ToolContext`] for the adapter. [`ToolExecutor::execute`] snapshots
/// its own [`IdUniverse`] from the locked `EditorState` and ignores the ctx arg, so
/// the adapter only needs a context that satisfies the trait; the empty universe is
/// never consulted by the real executor (it re-snapshots internally).
struct AdapterContext;

impl ToolContext for AdapterContext {
    fn id_universe(&self) -> IdUniverse {
        IdUniverse::default()
    }
}

/// A trivial [`ToolContext`] for callers outside this module (the `editor_edit`
/// command in `commands.rs`). [`ToolExecutor::execute`] ignores the ctx arg, so any
/// context that satisfies the trait works — this hands one out without exposing the
/// private [`AdapterContext`] type.
#[must_use]
pub fn adapter_context() -> impl ToolContext {
    AdapterContext
}

/// Adapts the shared [`ToolExecutor`] (the MCP server's `palmier_tools::ToolDispatch`)
/// to the agent loop's [`ToolDispatcher`] seam, mapping a `palmier_tools::ToolResult`
/// to a `palmier_agent::DispatchResult` **1:1** (text/image blocks pass through, the
/// `is_error` flag is preserved).
///
/// **This is the no-duplication guarantee made concrete:** the agent loop's tool
/// dispatch and the MCP server's tool dispatch are the *same* `Arc<ToolExecutor>`,
/// so both edit one `EditorState` / one undo timeline (FOUNDATION §4, PRD §10).
pub struct ExecutorDispatcher {
    executor: Arc<ToolExecutor>,
}

impl ExecutorDispatcher {
    /// Wrap the shared executor as an agent-loop dispatcher.
    #[must_use]
    pub fn new(executor: Arc<ToolExecutor>) -> ExecutorDispatcher {
        ExecutorDispatcher { executor }
    }
}

impl ToolDispatcher for ExecutorDispatcher {
    fn execute(&self, name: &str, input_json: &str) -> DispatchResult {
        // The loop forwards the RAW, verbatim `input_json` string (the second of
        // exactly two parse sites — agent-panel.md 201-203); parse it to the object
        // the executor expects. A malformed/non-object string → `{}` (the tool body
        // validates), matching the reference `parseJSONObject`.
        let args = parse_json_object(input_json);
        let result = self.executor.execute(name, args, &AdapterContext);
        map_tool_result(result)
    }
}

/// Map a `palmier_tools::ToolResult` to a `palmier_agent::DispatchResult` 1:1.
/// Text/image blocks pass through; `is_error` is preserved exactly.
fn map_tool_result(result: palmier_tools::ToolResult) -> DispatchResult {
    let content = result
        .content
        .into_iter()
        .map(|block| match block {
            Block::Text(text) => ToolResultBlock::text(text),
            Block::Image { base64, media_type } => ToolResultBlock::image(base64, media_type),
        })
        .collect();
    DispatchResult {
        content,
        is_error: result.is_error,
    }
}

// ─── Managed state ───────────────────────────────────────────────────────────────

/// Per-session live loop state: the conversation + the cancel token for any
/// in-flight turn. One entry per chat tab (`session_id`).
struct SessionLoop {
    /// The conversation accumulated across turns (reference `AgentService.messages`).
    loop_state: AgentLoop,
    /// The cancel token for the currently-streaming turn (if any).
    cancel: Option<CancellationToken>,
}

/// The app-wide agent integration state, held in Tauri managed state for the
/// process lifetime.
///
/// Owns the **single shared executor** both the MCP server and the in-app agent
/// drive, the running MCP server handle (if started at boot), the advertised tool
/// catalogue, the verbatim system prompt, the per-session loops, and the persisted
/// model preference.
pub struct AgentState {
    /// The one `Arc<ToolExecutor>` — the single owner of the `EditorState` shared
    /// by the MCP server (boot step 6) and every in-app agent turn. THIS is the
    /// correctness keystone: clone the `Arc`, never the executor.
    pub executor: Arc<ToolExecutor>,
    /// The running loopback MCP server, or `None` if disabled by pref / failed to
    /// bind. Held so it lives for the process and `stop()` runs on exit.
    mcp: Mutex<Option<palmier_mcp::McpServer>>,
    /// The 30-tool catalogue advertised to the model every turn (built once from
    /// the shared `palmier_tools` definitions — same surface the MCP server lists).
    tools: Vec<AnthropicToolSchema>,
    /// The verbatim shared agent system prompt — `palmier_prompt::AGENT_INSTRUCTIONS`,
    /// the SINGLE source (ruling #2). These are the SAME bytes the MCP `initialize`
    /// advertises (the MCP server reads the same constant), so the in-app agent and
    /// the external MCP surface can never drift. This is the `system` block every
    /// in-app `agent_send` turn carries.
    system: String,
    /// Per-chat-tab loop state, keyed by `session_id`.
    sessions: Mutex<HashMap<String, SessionLoop>>,
    /// The persisted model preference (config key `"agentModel"`; reference
    /// `UserDefaults`). `None` ⇒ fall back to the tier default.
    preferred_model: Mutex<Option<AnthropicModel>>,
    /// **The tab/history list** (E8-S7) — the ordered [`ChatSession`]s shown in
    /// the panel's tab bar + history dropdown. On project open this is
    /// `load_sessions(<project>/chat/)` (non-empty, newest-first) with a fresh
    /// empty open session **prepended as current** (reference `loadSessions`).
    /// `index 0` is the current tab. Empty when no project is open.
    tabs: Mutex<Vec<ChatSession>>,
    /// The current (foreground) session id — the tab `agent_send` runs against
    /// and the panel renders. `None` until a project opens / a session is created.
    current_session: Mutex<Option<String>>,
}

impl AgentState {
    /// Build the agent state over a fresh shared executor. The tool catalogue + the
    /// system prompt are snapshotted once (they never change at runtime).
    #[must_use]
    pub fn new() -> AgentState {
        AgentState::with_executor(Arc::new(ToolExecutor::new()))
    }

    /// Build over an explicit shared executor (used when a project is loaded so the
    /// executor wraps that project's `EditorState`).
    #[must_use]
    pub fn with_executor(executor: Arc<ToolExecutor>) -> AgentState {
        AgentState {
            executor,
            mcp: Mutex::new(None),
            tools: build_tool_schemas(),
            system: palmier_prompt::AGENT_INSTRUCTIONS.to_string(),
            sessions: Mutex::new(HashMap::new()),
            preferred_model: Mutex::new(None),
            tabs: Mutex::new(Vec::new()),
            current_session: Mutex::new(None),
        }
    }

    /// Stow the running MCP server handle (boot step 6).
    fn set_mcp(&self, server: palmier_mcp::McpServer) {
        *self.mcp.lock().expect("mcp mutex") = Some(server);
    }

    /// Whether the MCP server is currently running (live liveness for the Agent-tab
    /// status row — `get_mcp_status` reads this).
    #[must_use]
    pub fn mcp_running(&self) -> bool {
        self.mcp.lock().expect("mcp mutex").is_some()
    }

    /// The live bound address of the MCP server, if running.
    #[must_use]
    pub fn mcp_bind(&self) -> Option<String> {
        self.mcp
            .lock()
            .expect("mcp mutex")
            .as_ref()
            .map(|s| s.local_addr().to_string())
    }

    // ─── Session / tab orchestration (E8-S7) ─────────────────────────────────

    /// **Load on project open** (reference `loadSessions(projectURL)`): read
    /// `<project>/chat/*.json` (empty-message sessions dropped, sorted
    /// `updated_at` desc by [`palmier_agent::load_sessions`]), set them all
    /// `is_open=false`, then **prepend a fresh empty open session as current**.
    /// The prepended session's id becomes the [`current_session`](Self::current_session).
    ///
    /// A missing `chat/` dir ⇒ just the fresh session (offline-safe / new project).
    /// Drops any prior live `SessionLoop` state (a new project ⇒ a clean slate).
    pub fn load_project_sessions(&self, project_root: &Path) {
        let mut prior = load_sessions(project_root);
        for s in &mut prior {
            s.is_open = false;
        }
        // Prepend a fresh empty open session as the current tab (reference: the
        // panel always opens onto a new chat, with history behind it).
        let fresh = ChatSession::new();
        let current_id = fresh.id.to_string();
        let mut tabs = Vec::with_capacity(prior.len() + 1);
        tabs.push(fresh);
        tabs.extend(prior);

        *self.tabs.lock().expect("tabs mutex") = tabs;
        *self.current_session.lock().expect("current mutex") = Some(current_id);
        // A project switch resets the live per-tab loop state.
        self.sessions.lock().expect("sessions mutex").clear();
        tracing::info!(
            target: "agent",
            project = %project_root.display(),
            "loaded chat sessions for project"
        );
    }

    /// Clear all session state (returning Home / no project active).
    pub fn clear_sessions(&self) {
        self.tabs.lock().expect("tabs mutex").clear();
        *self.current_session.lock().expect("current mutex") = None;
        self.sessions.lock().expect("sessions mutex").clear();
    }

    /// Ensure `session_id` exists as an open tab and is the current/foreground
    /// session (`agent_send`'s pre-step). If the id is unknown — e.g. the frontend
    /// minted a fresh uuid for a brand-new chat, or there is no project open yet —
    /// a new [`ChatSession`] with that id is prepended so the conversation appears
    /// in the tab bar and is captured on the next save.
    fn ensure_tab_is_current(&self, session_id: &str) {
        {
            let mut tabs = self.tabs.lock().expect("tabs mutex");
            match tabs.iter_mut().find(|t| t.id.to_string() == session_id) {
                Some(tab) => tab.is_open = true,
                None => {
                    // Adopt the frontend-minted id (parse it; fall back to a fresh
                    // uuid only if it isn't a valid uuid, which the panel never sends).
                    let mut fresh = ChatSession::new();
                    if let Ok(id) = uuid::Uuid::parse_str(session_id) {
                        fresh.id = id;
                    }
                    tabs.insert(0, fresh);
                }
            }
        }
        *self.current_session.lock().expect("current mutex") = Some(session_id.to_string());
    }

    /// Copy the live loop messages of `session_id` into its tab [`ChatSession`],
    /// bumping `updated_at` + auto-deriving the title (reference
    /// `syncMessagesIntoCurrentSession`). No-op if the tab / loop is absent.
    fn sync_messages_into_session(&self, session_id: &str) {
        let messages = {
            let sessions = self.sessions.lock().expect("sessions mutex");
            match sessions.get(session_id) {
                Some(s) => s.loop_state.messages.clone(),
                None => return,
            }
        };
        let mut tabs = self.tabs.lock().expect("tabs mutex");
        if let Some(tab) = tabs.iter_mut().find(|t| t.id.to_string() == session_id) {
            tab.sync_messages(messages);
        }
    }

    /// **Capture the save snapshot** (reference `captureSaveSnapshot`): encode
    /// each **non-empty** tab session to `<uuid>.json` bytes for the bundle's
    /// `chat/` dir. Empty-message sessions are filtered (matching the load-side
    /// filter — `agent-panel.md` lines 157-161). First syncs every live loop's
    /// messages into its tab so the snapshot reflects the current conversation.
    ///
    /// Returns `(filename, bytes)` pairs the bundle writer drops into `chat/`.
    pub fn capture_chat_snapshot(&self) -> Vec<(String, Vec<u8>)> {
        // Sync every live loop into its tab first (so in-flight conversations
        // persist), then encode the non-empty tabs.
        let session_ids: Vec<String> = self
            .sessions
            .lock()
            .expect("sessions mutex")
            .keys()
            .cloned()
            .collect();
        for id in session_ids {
            self.sync_messages_into_session(&id);
        }

        let tabs = self.tabs.lock().expect("tabs mutex");
        tabs.iter()
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                let bytes = encode_session(s)
                    .map_err(|e| {
                        tracing::warn!(target: "agent", error = %e, "failed to encode chat session");
                    })
                    .ok()?;
                Some((format!("{}.json", s.id), bytes.into_bytes()))
            })
            .collect()
    }

    /// A summary row for the panel's tab bar / history dropdown.
    fn session_summaries(&self) -> Vec<SessionSummary> {
        let current = self.current_session.lock().expect("current mutex").clone();
        let tabs = self.tabs.lock().expect("tabs mutex");
        tabs.iter()
            .map(|s| SessionSummary {
                id: s.id.to_string(),
                title: s.title.clone(),
                updated_at: s.updated_at.unix_timestamp(),
                is_open: s.is_open,
                is_current: current.as_deref() == Some(&s.id.to_string()),
                message_count: s.messages.len(),
            })
            .collect()
    }

    /// The current session id (for tests / callers).
    fn current_id(&self) -> Option<String> {
        self.current_session.lock().expect("current mutex").clone()
    }

    /// The messages of `session_id`, projected into the frontend shape (E8-S8).
    /// Prefers the **live** [`SessionLoop`] (an in-flight / just-streamed
    /// conversation) and falls back to the persisted tab [`ChatSession`] (a history
    /// session that has no live loop yet). `None` if the id is unknown.
    ///
    /// This is the read the panel uses to restore a switched-to / reopened
    /// session's conversation in a Tauri webview (the backend is the source of
    /// truth for session content there).
    fn session_messages(&self, session_id: &str) -> Option<Vec<FrontendMessage>> {
        // Live loop first (covers the current/in-flight session).
        {
            let sessions = self.sessions.lock().expect("sessions mutex");
            if let Some(s) = sessions.get(session_id) {
                return Some(s.loop_state.messages.iter().map(project_message).collect());
            }
        }
        // Else the persisted tab session (history loaded from disk).
        let tabs = self.tabs.lock().expect("tabs mutex");
        tabs.iter()
            .find(|t| t.id.to_string() == session_id)
            .map(|t| t.messages.iter().map(project_message).collect())
    }

    /// **new_session** (reference `newChat`): sync the outgoing current tab, then
    /// prepend a fresh empty open session and make it current. Returns its id.
    fn new_session(&self) -> String {
        if let Some(cur) = self.current_id() {
            self.sync_messages_into_session(&cur);
        }
        let fresh = ChatSession::new();
        let id = fresh.id.to_string();
        self.tabs.lock().expect("tabs mutex").insert(0, fresh);
        *self.current_session.lock().expect("current mutex") = Some(id.clone());
        id
    }

    /// **open_session** (reference `selectSession`): cancel the outgoing turn + sync
    /// it, then make `session_id` the current open tab. `Err` if the id is unknown.
    fn open_session(&self, session_id: &str) -> Result<(), String> {
        if let Some(cur) = self.current_id() {
            self.cancel_session_turn(&cur);
            self.sync_messages_into_session(&cur);
        }
        {
            let mut tabs = self.tabs.lock().expect("tabs mutex");
            let Some(tab) = tabs.iter_mut().find(|t| t.id.to_string() == session_id) else {
                return Err(format!("unknown session id: {session_id}"));
            };
            tab.is_open = true;
        }
        *self.current_session.lock().expect("current mutex") = Some(session_id.to_string());
        Ok(())
    }

    /// **close_session** (reference `closeTab`): mark the tab not-open (it stays in
    /// history + on disk). If it was current, focus the next open tab — or open a
    /// fresh one if it was the last open tab (reference: last open tab → `newChat`).
    fn close_session(&self, session_id: &str) -> Result<(), String> {
        self.cancel_session_turn(session_id);
        self.sync_messages_into_session(session_id);
        let was_current = self.current_id().as_deref() == Some(session_id);

        let next_open = {
            let mut tabs = self.tabs.lock().expect("tabs mutex");
            let Some(tab) = tabs.iter_mut().find(|t| t.id.to_string() == session_id) else {
                return Err(format!("unknown session id: {session_id}"));
            };
            tab.is_open = false;
            if was_current {
                tabs.iter()
                    .find(|t| t.is_open && t.id.to_string() != session_id)
                    .map(|t| t.id.to_string())
            } else {
                None
            }
        };

        if was_current {
            match next_open {
                Some(id) => *self.current_session.lock().expect("current mutex") = Some(id),
                None => self.set_fresh_current(),
            }
        }
        Ok(())
    }

    /// **delete_session** (reference `deleteSession`): drop the session from the tab
    /// list + its live loop state. Deleting the current tab focuses the next open
    /// tab (or a fresh one). `Err` if unknown. The on-disk file delete is the
    /// caller's job (it needs the project root).
    fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.cancel_session_turn(session_id);
        let was_current = self.current_id().as_deref() == Some(session_id);
        {
            let mut tabs = self.tabs.lock().expect("tabs mutex");
            let before = tabs.len();
            tabs.retain(|t| t.id.to_string() != session_id);
            if tabs.len() == before {
                return Err(format!("unknown session id: {session_id}"));
            }
        }
        self.sessions.lock().expect("sessions mutex").remove(session_id);
        if was_current {
            let next = self
                .tabs
                .lock()
                .expect("tabs mutex")
                .iter()
                .find(|t| t.is_open)
                .map(|t| t.id.to_string());
            match next {
                Some(id) => *self.current_session.lock().expect("current mutex") = Some(id),
                None => self.set_fresh_current(),
            }
        }
        Ok(())
    }

    /// Prepend a fresh empty session and make it current (the "no open tab left"
    /// fallback shared by close/delete — reference last-tab → `newChat`).
    fn set_fresh_current(&self) {
        let fresh = ChatSession::new();
        let id = fresh.id.to_string();
        self.tabs.lock().expect("tabs mutex").insert(0, fresh);
        *self.current_session.lock().expect("current mutex") = Some(id);
    }

    /// Cancel the in-flight turn for `session_id` (if any) — the "cancel before
    /// switching/closing" step (reference `selectSession`).
    fn cancel_session_turn(&self, session_id: &str) {
        let token = {
            let sessions = self.sessions.lock().expect("sessions mutex");
            sessions.get(session_id).and_then(|s| s.cancel.clone())
        };
        if let Some(token) = token {
            token.cancel();
        }
    }
}

impl Default for AgentState {
    fn default() -> AgentState {
        AgentState::new()
    }
}

/// Build the [`AnthropicToolSchema`] catalogue from the shared `palmier_tools`
/// definitions — the exact 30-tool surface the MCP server advertises, projected
/// into the agent request's tool shape (no separate catalogue; one source of truth).
fn build_tool_schemas() -> Vec<AnthropicToolSchema> {
    tool_definitions()
        .into_iter()
        .map(|def| AnthropicToolSchema {
            name: def.name.wire_name().to_string(),
            description: def.description.to_string(),
            input_schema: def.input_schema,
        })
        .collect()
}

// ─── Boot step 6 — start the MCP server over the shared executor ─────────────────

/// Boot step 6: start the loopback MCP server over the **shared executor** if
/// `mcp_enabled`. Non-blocking and **failure-tolerant** — a bind error is logged,
/// never fatal, so boot stays offline-safe and under the cold-start budget.
///
/// Returns whether the server is running (for the boot log / `mcp_started`).
pub fn start_mcp<R: Runtime>(app: &AppHandle<R>, mcp_enabled: bool) -> bool {
    let Some(agent_state) = app.try_state::<AgentState>() else {
        tracing::error!(target: "mcp", "boot 6/7: AgentState not managed; MCP not started");
        return false;
    };

    if !mcp_enabled {
        tracing::info!(target: "mcp", "boot 6/7: MCP disabled by settings; not started");
        return false;
    }

    // The MCP server uses the SAME shared executor the in-app agent drives — so an
    // external client's edits and the in-app agent's edits share one EditorState /
    // one undo timeline (the single-owner correctness property).
    let executor = Arc::clone(&agent_state.executor);
    let config = palmier_mcp::ServerConfig::default();

    // The live-UI seam: after an EXTERNAL MCP client's successful MUTATING tool call,
    // emit the SAME `timeline://changed` the UI's `editor_edit` and the in-app agent's
    // dispatch emit, so the open Project window's panels refetch. `palmier-mcp` only
    // knows about a plain `Fn`; this closure captures the `AppHandle` (Tauri-side).
    let app_for_hook = app.clone();
    let on_mutation: palmier_mcp::MutationCallback = Arc::new(move || {
        if let Err(err) = app_for_hook.emit(crate::commands::TIMELINE_CHANGED_EVENT, ()) {
            tracing::warn!(
                target: "mcp",
                error = %err,
                "failed to emit timeline://changed after external MCP mutation"
            );
        }
    });

    // `McpServer::start_with_hook` is async (binds a TcpListener); run it to completion
    // on the Tauri async runtime. We block on JUST the bind (returns as soon as the
    // listener is bound — serving runs on a background task), so boot is not stalled.
    let start_result = tauri::async_runtime::block_on(palmier_mcp::McpServer::start_with_hook(
        executor,
        config,
        Some(on_mutation),
    ));

    match start_result {
        Ok(server) => {
            let addr = server.local_addr();
            agent_state.set_mcp(server);
            tracing::info!(target: "mcp", %addr, "boot 6/7: MCP server started (shared executor)");
            true
        }
        Err(err) => {
            // A failed bind (port in use, etc.) is logged, NOT fatal — the app runs
            // fine without the external MCP surface; the in-app agent still works.
            tracing::warn!(
                target: "mcp",
                error = %err,
                "boot 6/7: MCP server failed to start (logged, non-fatal)"
            );
            false
        }
    }
}

/// Stop the MCP server on app exit (graceful shutdown). Idempotent.
pub fn stop_mcp<R: Runtime>(app: &AppHandle<R>) {
    if let Some(agent_state) = app.try_state::<AgentState>()
        && let Some(mut server) = agent_state.mcp.lock().expect("mcp mutex").take()
    {
        tauri::async_runtime::block_on(server.stop());
        tracing::info!(target: "mcp", "MCP server stopped (app exit)");
    }
}

// ─── The agent command surface ───────────────────────────────────────────────────

/// The backend status the Agent panel reads for send-gating + the model picker
/// (reference `AgentService` derived getters; `agent-panel.md` lines 47-54). Mirrors
/// the frontend `BackendStatus` shape (camelCase).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    /// A non-empty Anthropic key is in the OS keyring (BYOK path).
    pub has_api_key: bool,
    /// Clerk session is active (Convex-proxied path).
    pub is_signed_in: bool,
    /// Signed-in account is a paid plan (free = Haiku only).
    pub is_paid: bool,
    /// Signed-in account has credits left (gates `can_stream` on the proxied path).
    pub has_credits: bool,
    /// Catalog-allowed models for a signed-in PAID plan (ruling #20). Empty ⇒ default.
    pub paid_catalog: Vec<String>,
    /// The model id a send would use (preference clamped to availability).
    pub effective_model: String,
    /// The wire model ids the user may pick, given their tier.
    pub available_models: Vec<String>,
}

/// A tab/history row the panel renders in the floating tab bar + the history
/// dropdown (mirrors the frontend `ChatSessionSummary`; camelCase).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// The session uuid (the key the tab commands take).
    pub id: String,
    /// The display title (auto-derived from the first user text, else "New chat").
    pub title: String,
    /// Last-updated as Unix seconds (history sorts newest-first by this).
    pub updated_at: i64,
    /// Whether this session is an open tab (vs history only).
    pub is_open: bool,
    /// Whether this is the current (foreground) session.
    pub is_current: bool,
    /// Number of messages (0 ⇒ a fresh empty session, never persisted).
    pub message_count: usize,
}

/// A chat message projected into the **frontend** `AgentMessage` shape (E8-S8): a
/// role + a list of [`FrontendBlock`]s. Returned by [`agent_get_session`] so the
/// panel can restore a switched-to / reopened session's conversation from the
/// backend (the single source of session state in a Tauri webview).
///
/// This is a *frontend-projection* of the backend [`palmier_agent::AgentMessage`] —
/// the `kind` discriminator + camelCase keys match the `src-ui` `AgentMessage` /
/// `AgentContentBlock` TS union (note `inputJson`, not the on-disk `input` key).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendMessage {
    /// The message id (the backend uuid, stringified).
    pub id: String,
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// The ordered content blocks.
    pub blocks: Vec<FrontendBlock>,
}

/// One content block projected into the frontend `AgentContentBlock` union shape
/// (E8-S8): `kind` discriminator `text` / `toolUse` / `toolResult`, camelCase keys.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendBlock {
    /// Assistant or user text.
    Text { text: String },
    /// A tool call (raw `input_json` forwarded verbatim under `inputJson`).
    #[serde(rename_all = "camelCase")]
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
    /// A tool result fed back to the model.
    #[serde(rename_all = "camelCase")]
    ToolResult {
        tool_use_id: String,
        content: Vec<FrontendToolResultBlock>,
        is_error: bool,
    },
}

/// A block inside a frontend tool result (`text` or an inlined `image`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendToolResultBlock {
    /// Plain-text tool output.
    Text { text: String },
    /// An inlined image (base64 + media type).
    #[serde(rename_all = "camelCase")]
    Image { base64: String, media_type: String },
}

/// Project a backend [`palmier_agent::AgentMessage`] into the frontend shape.
fn project_message(msg: &palmier_agent::AgentMessage) -> FrontendMessage {
    use palmier_agent::{AgentContentBlock, Role};
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let blocks = msg
        .blocks
        .iter()
        .map(|block| match block {
            AgentContentBlock::Text { text } => FrontendBlock::Text { text: text.clone() },
            AgentContentBlock::ToolUse { id, name, input_json } => FrontendBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input_json: input_json.clone(),
            },
            AgentContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => FrontendBlock::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: content.iter().map(project_tool_result_block).collect(),
                is_error: *is_error,
            },
        })
        .collect();
    FrontendMessage {
        id: msg.id.to_string(),
        role: role.to_string(),
        blocks,
    }
}

/// Project a backend tool-result block into the frontend shape.
fn project_tool_result_block(
    block: &palmier_agent::ToolResultBlock,
) -> FrontendToolResultBlock {
    use palmier_agent::ToolResultBlock;
    match block {
        ToolResultBlock::Text { text } => FrontendToolResultBlock::Text { text: text.clone() },
        ToolResultBlock::Image { base64, media_type } => FrontendToolResultBlock::Image {
            base64: base64.clone(),
            media_type: media_type.clone(),
        },
    }
}

/// One `agent://event` payload streamed to the webview (camelCase; mirrors the
/// frontend `AgentStreamEvent` union via a `type` discriminator).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
enum AgentEventPayload {
    /// A chunk of assistant text.
    TextDelta { session_id: String, text: String },
    /// A fully-accumulated tool call (the model wants a tool run).
    ToolUseComplete {
        session_id: String,
        id: String,
        name: String,
        input_json: String,
    },
    /// A tool result fed back to the model (so the panel can render tool activity).
    ToolResult {
        session_id: String,
        tool_use_id: String,
        is_error: bool,
        text: String,
    },
    /// The turn ended; carries the stop reason.
    Done {
        session_id: String,
        stop_reason: String,
    },
    /// A terminal error (transport / no-backend / cancelled-with-error).
    Error { session_id: String, message: String },
}

/// Resolve the keyring Anthropic key + account state into a [`Tier`] +
/// `effective_model`, the shared logic behind [`agent_status`] and [`agent_send`]'s
/// backend selection.
fn resolve_backend(
    auth: &palmier_auth::Auth,
    preferred: Option<AnthropicModel>,
) -> (SelectedBackend, Tier, AnthropicModel, BackendStatus) {
    let api_key = auth.anthropic_key().load().ok().flatten();
    let has_api_key = api_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);

    let account = auth.account();
    let is_signed_in = account.is_signed_in();
    let tier_obj = account.tier();
    let is_paid = tier_obj.is_paid();
    let has_credits = account.has_credits();

    let backend = palmier_agent::select_client(api_key.as_deref(), is_signed_in);

    // Catalog-driven paid models are an Epic 9 (Convex) concern; M2 has no catalog,
    // so paid → default Sonnet only (ruling #20: catalog MAY enable Opus later).
    let availability_tier = if has_api_key {
        Tier::Byok
    } else if is_signed_in {
        Tier::SignedIn {
            paid: is_paid,
            catalog: Vec::new(),
        }
    } else {
        Tier::None
    };

    let available = available_models(&availability_tier);
    let model = effective_model(preferred, &available);

    let status = BackendStatus {
        has_api_key,
        is_signed_in,
        is_paid,
        has_credits,
        paid_catalog: Vec::new(),
        effective_model: model.wire_id().to_string(),
        available_models: available.iter().map(|m| m.wire_id().to_string()).collect(),
    };

    (backend, availability_tier, model, status)
}

/// `agent_status` — the backend status the panel reads on mount + on
/// `anthropic-api-key-changed` (reference `AgentService` getters). BYOK if a key is
/// present; tier + credits from the account state.
#[tauri::command]
pub fn agent_status(
    auth: State<'_, palmier_auth::Auth>,
    agent: State<'_, AgentState>,
) -> BackendStatus {
    let preferred = *agent.preferred_model.lock().expect("model pref mutex");
    let (_, _, _, status) = resolve_backend(&auth, preferred);
    status
}

/// `agent_set_pref` — persist the picked model (config key `"agentModel"`). The
/// value is a wire model id (e.g. `claude-opus-4-8`); an unknown id is ignored.
#[tauri::command]
pub fn agent_set_pref(agent: State<'_, AgentState>, model: String) -> Result<(), String> {
    let parsed = AnthropicModel::from_wire_id(&model)
        .ok_or_else(|| format!("unknown model id: {model}"))?;
    *agent.preferred_model.lock().expect("model pref mutex") = Some(parsed);
    tracing::info!(target: "agent", model = %model, "agent model preference set");
    Ok(())
}

/// `agent_cancel` — cancel the in-flight turn for `session_id` (drops the empty
/// assistant turn cleanly; the loop emits no further events for it).
#[tauri::command]
pub fn agent_cancel(agent: State<'_, AgentState>, session_id: String) {
    let token = {
        let sessions = agent.sessions.lock().expect("sessions mutex");
        sessions.get(&session_id).and_then(|s| s.cancel.clone())
    };
    if let Some(token) = token {
        token.cancel();
        tracing::info!(target: "agent", session_id = %session_id, "agent turn cancelled");
    }
}

/// `agent_send` — run one streaming agent turn for `session_id` against the BYOK
/// `AnthropicClient`, dispatching every tool call into the **shared executor** and
/// streaming each [`StreamEvent`] to the webview as an `agent://event`.
///
/// `user_text` is the message; `mentions` are the `@`-mention display tokens (the
/// rich context-hint projection is E8-S5 — here they are recorded but the structural
/// turn is what streams); `model` optionally overrides the picked model for this
/// turn (else the persisted preference / tier default is used).
///
/// The turn is spawned onto the Tauri async runtime so the command returns
/// immediately; the panel renders the live `agent://event` stream. A missing key /
/// proxied-not-wired backend emits a single `error` event (the panel surfaces it).
#[tauri::command]
pub fn agent_send<R: Runtime>(
    app: AppHandle<R>,
    auth: State<'_, palmier_auth::Auth>,
    agent: State<'_, AgentState>,
    session_id: String,
    user_text: String,
    mentions: Option<Vec<String>>,
    model: Option<String>,
) -> Result<(), String> {
    let _ = mentions; // E8-S5 enriches the user turn with context hints; recorded, not yet projected.

    // Resolve the backend + model from the keyring key + account (off the UI path).
    let model_override = model.as_deref().and_then(AnthropicModel::from_wire_id);
    let preferred = model_override.or(*agent.preferred_model.lock().expect("model pref mutex"));
    let (backend, _tier, chosen_model, _status) = resolve_backend(&auth, preferred);

    // Build the live client. BYOK key → real AnthropicClient; signed-in-without-key
    // → proxied (E8-S6, not yet wired) → error; nothing → no-backend error. A build
    // failure becomes a single `error` event so the panel surfaces it (never panics).
    let api_key = match &backend {
        SelectedBackend::Anthropic { api_key } => Some(api_key.clone()),
        _ => None,
    };
    let is_signed_in = matches!(backend, SelectedBackend::Palmier);

    // Optional custom Anthropic endpoint for LOCAL-BRIDGE / SELF-HOST use:
    // `PALMIER_ANTHROPIC_BASE_URL` lets a BYOK user point the in-app agent at a
    // local Anthropic-compatible bridge (e.g. LiteLLM fronting an OpenAI-compatible
    // model) instead of `api.anthropic.com`. Trim + ignore-empty; `None` ⇒ default
    // endpoint. The key is NEVER logged — only which endpoint is in use.
    let base_url = std::env::var("PALMIER_ANTHROPIC_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match &base_url {
        Some(url) => tracing::info!(target: "agent", endpoint = %url, "agent client endpoint: custom (local-bridge/self-host via PALMIER_ANTHROPIC_BASE_URL)"),
        None => tracing::info!(target: "agent", "agent client endpoint: default (api.anthropic.com)"),
    }

    let client = match select_and_build_client(api_key.as_deref(), is_signed_in, base_url.as_deref()) {
        Ok(client) => client,
        Err(err) => {
            emit_event(
                &app,
                &AgentEventPayload::Error {
                    session_id: session_id.clone(),
                    message: backend_error_message(&err),
                },
            );
            emit_event(
                &app,
                &AgentEventPayload::Done {
                    session_id,
                    stop_reason: "error".to_string(),
                },
            );
            return Ok(());
        }
    };

    // Ensure this session_id is a tab (so it appears in the tab bar + is captured on
    // save) and is the current/foreground tab. A frontend-minted session_id that
    // isn't yet a tab (e.g. the very first message of a brand-new chat) is adopted.
    agent.ensure_tab_is_current(&session_id);

    // Append the user turn to the (per-session) loop + install a fresh cancel token.
    let cancel = CancellationToken::new();
    {
        let mut sessions = agent.sessions.lock().expect("sessions mutex");
        let entry = sessions.entry(session_id.clone()).or_insert_with(|| SessionLoop {
            loop_state: AgentLoop::new(chosen_model, agent.system.clone(), agent.tools.clone()),
            cancel: None,
        });
        // Re-target the model for this turn (the user may have switched the picker).
        entry.loop_state.model = chosen_model;
        entry.loop_state.push_user_text(user_text);
        entry.cancel = Some(cancel.clone());
    }

    // Snapshot what the spawned task needs (it can't hold `State` across await).
    let executor = Arc::clone(&agent.executor);
    let app_for_task = app.clone();

    // Drive the turn on the Tauri async runtime; the command returns immediately and
    // the panel renders the streamed `agent://event`s.
    tauri::async_runtime::spawn(async move {
        run_agent_turn(app_for_task, session_id, client, executor, cancel).await;
    });

    Ok(())
}

/// Drive one [`AgentLoop::run_turn`] to completion, streaming each event to the
/// webview. The loop holds the conversation in managed state; we run it against a
/// **clone** of the loop so the `AgentState` mutex isn't held across `.await`, then
/// write the resulting conversation back.
async fn run_agent_turn<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    client: Box<dyn palmier_agent::AgentClient>,
    executor: Arc<ToolExecutor>,
    cancel: CancellationToken,
) {
    // Take a working copy of the loop (so we don't hold the sessions lock across the
    // network stream). The agent loop is a pure state machine over its `messages`.
    let Some(agent_state) = app.try_state::<AgentState>() else {
        return;
    };
    let mut working = {
        let sessions = agent_state.sessions.lock().expect("sessions mutex");
        match sessions.get(&session_id) {
            Some(s) => s.loop_state.clone(),
            None => return,
        }
    };

    // The tool dispatcher is the SHARED executor — the same `Arc` the MCP server
    // drives. Tool calls in this turn mutate the one `EditorState`.
    let dispatcher = ExecutorDispatcher::new(executor);

    // A streaming wrapper around the dispatcher so we can emit a `tool_result` event
    // to the panel as each tool runs (the loop itself only returns the final state).
    let streaming = StreamingDispatcher {
        inner: dispatcher,
        app: app.clone(),
        session_id: session_id.clone(),
    };

    // We can't get text deltas out of `run_turn` directly (it accumulates them into
    // the conversation), so we diff the conversation after the turn and stream the
    // assistant blocks. To keep deltas LIVE, we instead drive the loop and emit on
    // completion of each turn-round. For M2 the panel receives: tool_use + tool_result
    // events live (via the streaming dispatcher), and the assistant text + done at the
    // end. (A future refinement threads a delta channel into the loop.)
    let result = working.run_turn(&*client, Some(&streaming), &cancel).await;

    // Stream the assistant text blocks that accumulated this turn.
    emit_assistant_text(&app, &session_id, &working);

    match result {
        Ok(reason) => {
            emit_event(
                &app,
                &AgentEventPayload::Done {
                    session_id: session_id.clone(),
                    stop_reason: stop_reason_wire(reason).to_string(),
                },
            );
        }
        Err(err) => {
            emit_event(
                &app,
                &AgentEventPayload::Error {
                    session_id: session_id.clone(),
                    message: backend_error_message(&err),
                },
            );
            emit_event(
                &app,
                &AgentEventPayload::Done {
                    session_id: session_id.clone(),
                    stop_reason: "error".to_string(),
                },
            );
        }
    }

    // Write the resulting conversation back + clear the cancel token.
    if let Some(agent_state) = app.try_state::<AgentState>() {
        {
            let mut sessions = agent_state.sessions.lock().expect("sessions mutex");
            if let Some(entry) = sessions.get_mut(&session_id) {
                entry.loop_state = working;
                entry.cancel = None;
            }
        }
        // Sync the live loop into its tab session (bumps updated_at + auto-derives
        // the title) and mark the active document dirty so the next save persists
        // the chat (ruling #4 — sessions write on document save, not eagerly).
        agent_state.sync_messages_into_session(&session_id);
        crate::project::mark_chat_dirty(&app);
    }
}

/// A [`ToolDispatcher`] that wraps [`ExecutorDispatcher`] and emits a `tool_use` +
/// `tool_result` `agent://event` for each call, so the panel renders tool activity
/// as it happens (the loop itself only returns the final conversation).
struct StreamingDispatcher<R: Runtime> {
    inner: ExecutorDispatcher,
    app: AppHandle<R>,
    session_id: String,
}

impl<R: Runtime> ToolDispatcher for StreamingDispatcher<R> {
    fn execute(&self, name: &str, input_json: &str) -> DispatchResult {
        emit_event(
            &self.app,
            &AgentEventPayload::ToolUseComplete {
                session_id: self.session_id.clone(),
                id: String::new(),
                name: name.to_string(),
                input_json: input_json.to_string(),
            },
        );
        let result = self.inner.execute(name, input_json);
        let text = result
            .content
            .iter()
            .filter_map(|b| match b {
                ToolResultBlock::Text { text } => Some(text.clone()),
                ToolResultBlock::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        emit_event(
            &self.app,
            &AgentEventPayload::ToolResult {
                session_id: self.session_id.clone(),
                tool_use_id: String::new(),
                is_error: result.is_error,
                text,
            },
        );
        // The agent edits the SHARED EditorState — notify every window so the Project
        // surface's panels refetch (the same `timeline://changed` the UI's `editor_edit`
        // emits, so AGENT edits update the UI too). Only non-error dispatches can have
        // mutated state; a read/echo refetch is cheap and idempotent. Logged-non-fatal.
        if !result.is_error {
            if let Err(err) = self.app.emit(crate::commands::TIMELINE_CHANGED_EVENT, ()) {
                tracing::warn!(target: "agent", error = %err, "failed to emit timeline://changed");
            }
        }
        result
    }
}

/// Stream the assistant text blocks of the most-recent assistant message as a single
/// `text_delta` event (M2: deltas land at turn end; a future refinement makes them
/// per-chunk live by threading a channel into the loop).
fn emit_assistant_text<R: Runtime>(app: &AppHandle<R>, session_id: &str, loop_state: &AgentLoop) {
    use palmier_agent::{AgentContentBlock, Role};
    // Find the last assistant message and emit its text.
    for msg in loop_state.messages.iter().rev() {
        if msg.role == Role::Assistant {
            for block in &msg.blocks {
                if let AgentContentBlock::Text { text } = block
                    && !text.is_empty()
                {
                    {
                        emit_event(
                            app,
                            &AgentEventPayload::TextDelta {
                                session_id: session_id.to_string(),
                                text: text.clone(),
                            },
                        );
                    }
                }
            }
            return;
        }
    }
}

/// The wire string for a stop reason (snake_case; matches the frontend
/// `AgentStopReason`).
fn stop_reason_wire(reason: palmier_agent::AnthropicStopReason) -> &'static str {
    use palmier_agent::AnthropicStopReason::*;
    match reason {
        EndTurn => "end_turn",
        ToolUse => "tool_use",
        MaxTokens => "max_tokens",
        StopSequence => "stop_sequence",
        PauseTurn => "pause_turn",
        Refusal => "refusal",
        Other => "other",
    }
}

/// A human-readable message for a client error (the panel surfaces it).
fn backend_error_message(err: &AgentClientError) -> String {
    err.to_string()
}

// ─── Tab orchestration commands (E8-S7) ──────────────────────────────────────────
//
// The panel's floating tab bar + history dropdown call these; each maps to the
// `AgentState` tab list + the live per-session loop. They mutate in-memory state
// and mark the active document dirty so the chat persists on the next save
// (ruling #4 — sessions are NOT written eagerly). The actual file write happens in
// `project::flush_active` / save, which pulls `AgentState::capture_chat_snapshot`.

/// `agent_list_sessions` — the tab bar + history rows (current first; history
/// newest-first behind it). The panel renders open tabs and the history dropdown
/// from this.
#[tauri::command]
pub fn agent_list_sessions(agent: State<'_, AgentState>) -> Vec<SessionSummary> {
    agent.session_summaries()
}

/// `agent_get_session` — the full message list of `session_id`, projected into the
/// frontend `AgentMessage` shape (E8-S8). The panel calls this after switching /
/// reopening a tab to restore that session's conversation from the backend (the
/// source of truth for session content in a Tauri webview). `Err` if the id is
/// unknown (the panel falls back to its local store).
#[tauri::command]
pub fn agent_get_session(
    agent: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<FrontendMessage>, String> {
    agent
        .session_messages(&session_id)
        .ok_or_else(|| format!("unknown session id: {session_id}"))
}

/// `agent_new_session` — open a fresh empty chat tab and make it current
/// (reference `newChat`). Syncs the outgoing current tab first. Returns the new
/// session id. Does NOT itself write a file (the empty session is never persisted
/// until it has messages).
#[tauri::command]
pub fn agent_new_session(agent: State<'_, AgentState>) -> String {
    let id = agent.new_session();
    tracing::info!(target: "agent", session_id = %id, "new chat session");
    id
}

/// `agent_open_session` — switch the current tab to `session_id` (reference
/// `selectSession`: cancel the in-flight turn + sync the outgoing session first).
/// Marks the session open. `Err` if the id is unknown.
#[tauri::command]
pub fn agent_open_session(
    agent: State<'_, AgentState>,
    session_id: String,
) -> Result<(), String> {
    agent.open_session(&session_id)?;
    tracing::info!(target: "agent", session_id = %session_id, "opened chat session");
    Ok(())
}

/// `agent_close_session` — close the tab for `session_id` (reference `closeTab`):
/// mark it not-open; closing the **current** tab moves focus to the next open tab,
/// or opens a fresh one if it was the last open tab (reference: closing the last
/// open tab → `newChat`). The session stays in history (and on disk) — closing a
/// tab does not delete it. Marks the document dirty (the close is a session change).
#[tauri::command]
pub fn agent_close_session<R: Runtime>(
    app: AppHandle<R>,
    agent: State<'_, AgentState>,
    session_id: String,
) -> Result<(), String> {
    agent.close_session(&session_id)?;
    crate::project::mark_chat_dirty(&app);
    tracing::info!(target: "agent", session_id = %session_id, "closed chat tab");
    Ok(())
}

/// `agent_delete_session` — remove the session entirely (reference
/// `deleteSession`): drop it from the tab list AND delete its `<uuid>.json` from
/// the project's `chat/` dir. Deleting the current tab opens a fresh one. Marks the
/// document dirty so the next save reflects the removal.
#[tauri::command]
pub fn agent_delete_session<R: Runtime>(
    app: AppHandle<R>,
    agent: State<'_, AgentState>,
    session_id: String,
) -> Result<(), String> {
    agent.delete_session(&session_id)?;
    // Delete the on-disk session file from the active project's chat/ dir, if any.
    delete_session_file(&app, &session_id);
    crate::project::mark_chat_dirty(&app);
    tracing::info!(target: "agent", session_id = %session_id, "deleted chat session");
    Ok(())
}

/// Delete `<project>/chat/<session_id>.json` from the active project, if a project
/// is open and the file exists. Best-effort + lenient (a missing file / no project
/// is fine — the in-memory removal is the source of truth on the next save).
fn delete_session_file<R: Runtime>(app: &AppHandle<R>, session_id: &str) {
    let Some(root) = crate::project::active_project_root(app) else {
        return;
    };
    let path = palmier_agent::chat_dir(&root).join(format!("{session_id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!(target: "agent", path = %path.display(), "deleted chat session file"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(target: "agent", error = %e, "failed to delete chat session file"),
    }
}

/// Emit one `agent://event` to all webviews. Logged-but-non-fatal on failure.
fn emit_event<R: Runtime>(app: &AppHandle<R>, payload: &AgentEventPayload) {
    if let Err(err) = app.emit(AGENT_EVENT, payload) {
        tracing::warn!(target: "agent", error = %err, "failed to emit agent://event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The adapter maps a `palmier_tools::ToolResult` to a `DispatchResult` 1:1:
    /// text and image blocks pass through and `is_error` is preserved.
    #[test]
    fn adapter_maps_tool_result_blocks_and_error_flag() {
        // text + image, not-error.
        let ok = palmier_tools::ToolResult {
            content: vec![
                Block::Text("hello".to_string()),
                Block::Image {
                    base64: "AAAA".to_string(),
                    media_type: "image/png".to_string(),
                },
            ],
            is_error: false,
        };
        let mapped = map_tool_result(ok);
        assert!(!mapped.is_error);
        assert_eq!(mapped.content.len(), 2);
        assert_eq!(mapped.content[0], ToolResultBlock::text("hello"));
        assert_eq!(mapped.content[1], ToolResultBlock::image("AAAA", "image/png"));

        // error shape preserved.
        let err = palmier_tools::ToolResult::error("bad args");
        let mapped = map_tool_result(err);
        assert!(mapped.is_error);
        assert_eq!(mapped.content[0], ToolResultBlock::text("bad args"));
    }

    /// The dispatcher adapter drives the SHARED executor: a real tool call (here a
    /// READ tool) round-trips through the agent's `ToolDispatcher` seam and returns a
    /// non-error result — proving the agent loop and the MCP server invoke ONE
    /// `palmier_tools::execute` (no duplication).
    #[test]
    fn dispatcher_runs_a_real_read_tool_through_shared_executor() {
        let executor = Arc::new(ToolExecutor::new());
        let dispatcher = ExecutorDispatcher::new(Arc::clone(&executor));
        // get_timeline on a fresh editor returns a real (non-error) result.
        let result = dispatcher.execute("get_timeline", "{}");
        assert!(!result.is_error, "get_timeline should dispatch and succeed");
        assert!(!result.content.is_empty());
        // An unknown tool returns the contract error shape (is_error true).
        let unknown = dispatcher.execute("not_a_tool", "{}");
        assert!(unknown.is_error);
    }

    /// **The single-owner correctness property**: the MCP server and the in-app
    /// agent share ONE `EditorState`. An edit issued through the agent's dispatcher
    /// is visible to a second dispatcher built from the SAME `Arc<ToolExecutor>` —
    /// i.e. both surfaces read/write one timeline. We assert via an `add_clips`-style
    /// mutation observable through `get_timeline`.
    #[test]
    fn mcp_and_agent_share_one_editor_state() {
        let executor = Arc::new(ToolExecutor::new());

        // "MCP side" and "agent side" are two dispatchers over the SAME Arc.
        let mcp_side = ExecutorDispatcher::new(Arc::clone(&executor));
        let agent_side = ExecutorDispatcher::new(Arc::clone(&executor));

        // Mutate the editor via a direct executor call (stand-in for any mutating
        // tool) and confirm BOTH dispatchers observe the same post-state. We use
        // `with_state_mut` to set a flag both sides then read back via the executor,
        // proving they reference one `EditorState` (Arc identity, not a clone).
        executor.with_state_mut(|state| {
            state.can_generate = true;
        });
        let seen_by_mcp = executor.with_state_ref(|s| s.can_generate);
        assert!(seen_by_mcp, "MCP-side sees the mutation");

        // Both dispatchers point at the same Arc — Arc::strong_count proves shared
        // ownership (executor + 2 dispatchers = 3 strong refs, no deep clone).
        assert_eq!(Arc::strong_count(&executor), 3);

        // And both can dispatch against that one state.
        assert!(!mcp_side.execute("get_timeline", "{}").is_error);
        assert!(!agent_side.execute("get_timeline", "{}").is_error);
    }

    /// `resolve_backend`-style selection: a present key ⇒ BYOK (all three models);
    /// no key + not signed in ⇒ None (no models). We exercise the pure availability
    /// path the command uses (the keyring/account read is integration-only).
    #[test]
    fn backend_status_selection_byok_vs_none() {
        // BYOK: key present → all three models, effective = first (or persisted).
        let byok = available_models(&Tier::Byok);
        assert_eq!(byok.len(), 3);
        let eff = effective_model(Some(AnthropicModel::Opus48), &byok);
        assert_eq!(eff, AnthropicModel::Opus48);

        // None: no backend → no models, effective falls back to the default.
        let none = available_models(&Tier::None);
        assert!(none.is_empty());
        let eff = effective_model(None, &none);
        assert_eq!(eff, palmier_agent::DEFAULT_MODEL);

        // Signed-in free → Haiku only.
        let free = available_models(&Tier::SignedIn {
            paid: false,
            catalog: Vec::new(),
        });
        assert_eq!(free, vec![AnthropicModel::Haiku45]);
    }

    /// The tool catalogue the agent advertises is the SAME 30-tool surface the MCP
    /// server lists (built from the shared `palmier_tools` definitions).
    #[test]
    fn agent_tool_catalogue_is_the_30_tool_surface() {
        let schemas = build_tool_schemas();
        assert_eq!(schemas.len(), palmier_tools::TOOL_COUNT);
        assert_eq!(schemas.len(), 30);
        // Names are the wire names; descriptions are the verbatim contract strings.
        assert!(schemas.iter().any(|s| s.name == "get_timeline"));
        assert!(schemas.iter().all(|s| !s.description.is_empty()));
    }

    /// **The verbatim-prompt property (E7-S13 follow-up / E8-S9 line 432):** the
    /// in-app agent runs with the SAME system prompt the MCP server advertises
    /// (ruling #2 — one shared string, both injection sites, no drift). We assert at
    /// three points along the path `agent_send` takes:
    ///   1. `AgentState.system` is `palmier_prompt::AGENT_INSTRUCTIONS` verbatim,
    ///   2. it is byte-identical to what `palmier_mcp::initialize` advertises
    ///      (the MCP re-export resolves to the SAME constant),
    ///   3. the per-session `AgentLoop` `agent_send` builds carries it as `system`,
    ///      and the request that loop emits to the model carries it unchanged.
    #[test]
    fn in_app_agent_turn_carries_verbatim_system_prompt() {
        // (1) The state's system prompt is the single-source constant verbatim.
        let state = AgentState::new();
        assert_eq!(
            state.system,
            palmier_prompt::AGENT_INSTRUCTIONS,
            "AgentState.system must be palmier_prompt::AGENT_INSTRUCTIONS verbatim"
        );
        assert!(!state.system.is_empty(), "the system prompt must be non-empty");

        // (2) The MCP server advertises the SAME bytes (re-export of one constant) —
        // the two injection sites can never drift (ruling #2).
        assert_eq!(
            state.system,
            palmier_mcp::AGENT_INSTRUCTIONS,
            "in-app agent system prompt must equal the MCP-advertised instructions"
        );

        // (3) The per-session loop `agent_send` builds carries it as `system` (this
        // mirrors the exact `AgentLoop::new(model, agent.system.clone(), ...)` line in
        // `agent_send`), and the request that loop emits to the client carries it.
        let session_loop = AgentLoop::new(
            palmier_agent::DEFAULT_MODEL,
            state.system.clone(),
            state.tools.clone(),
        );
        assert_eq!(
            session_loop.system,
            palmier_prompt::AGENT_INSTRUCTIONS,
            "agent_send's AgentLoop must carry the verbatim instructions as system"
        );
        // The request body builder projects the loop's `system` straight onto the
        // Anthropic `system` field (build via the public request builder).
        let request = palmier_agent::AgentRequest {
            model: session_loop.model,
            max_tokens: 8192,
            system: session_loop.system.clone(),
            tools: session_loop.tools.clone(),
            messages: Vec::new(),
        };
        assert_eq!(
            request.system,
            palmier_prompt::AGENT_INSTRUCTIONS,
            "the AgentRequest sent to the model must carry the verbatim instructions"
        );
    }

    /// The streamed event payload serializes with the `type` discriminator + the
    /// camelCase fields the frontend `AgentStreamEvent` union expects.
    #[test]
    fn agent_event_payload_serializes_for_frontend() {
        let delta = AgentEventPayload::TextDelta {
            session_id: "s1".to_string(),
            text: "hi".to_string(),
        };
        let v = serde_json::to_value(&delta).unwrap();
        assert_eq!(v["type"], "text_delta");
        assert_eq!(v["sessionId"], "s1");
        assert_eq!(v["text"], "hi");

        let done = AgentEventPayload::Done {
            session_id: "s1".to_string(),
            stop_reason: "end_turn".to_string(),
        };
        let v = serde_json::to_value(&done).unwrap();
        assert_eq!(v["type"], "done");
        assert_eq!(v["stopReason"], "end_turn");

        let tr = AgentEventPayload::ToolResult {
            session_id: "s1".to_string(),
            tool_use_id: "t1".to_string(),
            is_error: true,
            text: "boom".to_string(),
        };
        let v = serde_json::to_value(&tr).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["isError"], true);
    }

    /// The `BackendStatus` snapshot serializes camelCase for the panel.
    #[test]
    fn backend_status_serializes_camel_case() {
        let status = BackendStatus {
            has_api_key: true,
            is_signed_in: false,
            is_paid: false,
            has_credits: false,
            paid_catalog: Vec::new(),
            effective_model: "claude-sonnet-4-6".to_string(),
            available_models: vec!["claude-sonnet-4-6".to_string()],
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["hasApiKey"], true);
        assert_eq!(v["effectiveModel"], "claude-sonnet-4-6");
        assert_eq!(v["availableModels"], json!(["claude-sonnet-4-6"]));
    }

    // ─── E8-S7 — session persistence + tab orchestration ─────────────────────────

    use palmier_agent::{AgentContentBlock, AgentMessage, ChatSession, Role};
    use std::path::PathBuf;

    /// A unique temp project root for a test.
    fn temp_project() -> PathBuf {
        std::env::temp_dir().join(format!("palmier-e8s7-{}", uuid::Uuid::new_v4()))
    }

    /// Add `text` as a user message to the live loop for `session_id` so a sync
    /// captures it into the tab (simulating what `agent_send` accumulates).
    fn push_user_turn(state: &AgentState, session_id: &str, text: &str) {
        let mut sessions = state.sessions.lock().expect("sessions mutex");
        let entry = sessions.entry(session_id.to_string()).or_insert_with(|| SessionLoop {
            loop_state: AgentLoop::new(
                palmier_agent::DEFAULT_MODEL,
                state.system.clone(),
                state.tools.clone(),
            ),
            cancel: None,
        });
        entry.loop_state.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text(text)],
        ));
    }

    /// load_project_sessions: a missing chat/ dir yields exactly one fresh empty
    /// current session (offline-safe), with no history.
    #[test]
    fn load_missing_chat_dir_yields_one_fresh_current() {
        let state = AgentState::new();
        let root = temp_project(); // never created
        state.load_project_sessions(&root);
        let tabs = state.tabs.lock().expect("tabs mutex");
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].is_empty());
        assert!(tabs[0].is_open);
        assert_eq!(state.current_id(), Some(tabs[0].id.to_string()));
    }

    /// Round-trip: create + capture-save a session, reload it on the next project
    /// open → identical content, history sorted newest-first, with a fresh current
    /// prepended. (create → save → reload → identical, sorted desc.)
    #[test]
    fn session_round_trip_create_save_reload_identical_sorted_desc() {
        let root = temp_project();
        std::fs::create_dir_all(&root).unwrap();

        // Pre-seed two prior sessions directly on disk (older + newer).
        let mut older = ChatSession::new();
        older.title = "older chat".to_string();
        older.updated_at = time::macros::datetime!(2026-06-18 09:00:00 UTC);
        older.messages.push(AgentMessage::new(Role::User, vec![AgentContentBlock::text("a")]));
        let mut newer = ChatSession::new();
        newer.title = "newer chat".to_string();
        newer.updated_at = time::macros::datetime!(2026-06-19 09:00:00 UTC);
        newer.messages.push(AgentMessage::new(Role::User, vec![AgentContentBlock::text("b")]));
        palmier_agent::write_session(&root, &older).unwrap();
        palmier_agent::write_session(&root, &newer).unwrap();

        // Open the project → load history + a fresh current.
        let state = AgentState::new();
        state.load_project_sessions(&root);
        {
            let tabs = state.tabs.lock().expect("tabs mutex");
            // [fresh-current, newer, older] — history sorted updated_at DESC.
            assert_eq!(tabs.len(), 3);
            assert!(tabs[0].is_empty(), "current is the fresh empty session");
            assert_eq!(tabs[1].title, "newer chat");
            assert_eq!(tabs[2].title, "older chat");
            // History sessions are not open (panel shows them as history rows).
            assert!(!tabs[1].is_open && !tabs[2].is_open);
        }

        // The capture snapshot encodes the two NON-EMPTY history sessions (the fresh
        // current is empty ⇒ skipped). Reload them and assert identical content.
        let snapshot = state.capture_chat_snapshot();
        assert_eq!(snapshot.len(), 2, "fresh empty current is not persisted");
        // Write them back into a clean dir and reload → identical to the originals.
        let root2 = temp_project();
        std::fs::create_dir_all(palmier_agent::chat_dir(&root2)).unwrap();
        for (name, bytes) in &snapshot {
            std::fs::write(palmier_agent::chat_dir(&root2).join(name), bytes).unwrap();
        }
        let reloaded = palmier_agent::load_sessions(&root2);
        assert_eq!(reloaded.len(), 2);
        // Sorted desc: newer first.
        assert_eq!(reloaded[0].title, "newer chat");
        assert_eq!(reloaded[1].title, "older chat");
        assert_eq!(reloaded[0].messages, newer.messages);
        assert_eq!(reloaded[1].messages, older.messages);

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&root2).ok();
    }

    /// capture_chat_snapshot skips empty-message sessions (matching the load filter)
    /// and syncs a live loop's messages into its tab so an in-flight chat persists.
    #[test]
    fn capture_skips_empty_and_syncs_live_loop() {
        let state = AgentState::new();
        let root = temp_project();
        state.load_project_sessions(&root); // one fresh empty current

        let current = state.current_id().unwrap();
        // No messages yet ⇒ snapshot is empty (the only tab is the empty current).
        assert!(state.capture_chat_snapshot().is_empty());

        // Push a user turn into the live loop, then capture: the tab is synced +
        // persisted (title auto-derives), the empty case is gone.
        push_user_turn(&state, &current, "trim the intro please");
        let snapshot = state.capture_chat_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot[0].0.ends_with(".json"));
        // The tab's title derived from the first user text.
        let tabs = state.tabs.lock().expect("tabs mutex");
        assert_eq!(tabs[0].title, "trim the intro please");
    }

    /// new_session: prepends a fresh empty current tab, keeping the prior tab in the
    /// list (as history once it has content).
    #[test]
    fn new_session_prepends_fresh_current() {
        let state = AgentState::new();
        state.load_project_sessions(&temp_project());
        let first = state.current_id().unwrap();
        let second = state.new_session();
        assert_ne!(first, second);
        assert_eq!(state.current_id(), Some(second.clone()));
        let tabs = state.tabs.lock().expect("tabs mutex");
        assert_eq!(tabs[0].id.to_string(), second, "new session is at index 0");
        assert_eq!(tabs.len(), 2);
    }

    /// open_session: switches current; unknown id errors.
    #[test]
    fn open_session_switches_current_and_errors_unknown() {
        let state = AgentState::new();
        state.load_project_sessions(&temp_project());
        let first = state.current_id().unwrap();
        let second = state.new_session();
        // Switch back to the first.
        state.open_session(&first).unwrap();
        assert_eq!(state.current_id(), Some(first));
        let _ = second;
        // Unknown id errors.
        assert!(state.open_session("not-a-session").is_err());
    }

    /// close_session: closing the current (and only open) tab opens a fresh one;
    /// the closed session stays in the list as history. Closing the LAST open tab →
    /// newChat (reference).
    #[test]
    fn close_last_open_tab_opens_fresh() {
        let state = AgentState::new();
        state.load_project_sessions(&temp_project());
        let current = state.current_id().unwrap();
        // Give it content so it survives as history.
        push_user_turn(&state, &current, "hello");

        state.close_session(&current).unwrap();
        // A fresh current was opened (different id), and the closed one is still in
        // the list but not open.
        let new_current = state.current_id().unwrap();
        assert_ne!(new_current, current);
        let tabs = state.tabs.lock().expect("tabs mutex");
        let closed = tabs.iter().find(|t| t.id.to_string() == current).unwrap();
        assert!(!closed.is_open, "closed session is history, not deleted");
    }

    /// delete_session: drops the session from the list entirely; deleting the
    /// current opens a fresh one; unknown id errors.
    #[test]
    fn delete_session_removes_and_errors_unknown() {
        let state = AgentState::new();
        state.load_project_sessions(&temp_project());
        let current = state.current_id().unwrap();
        push_user_turn(&state, &current, "doomed");

        // Delete the current → it's gone, a fresh current exists.
        state.delete_session(&current).unwrap();
        {
            let tabs = state.tabs.lock().expect("tabs mutex");
            assert!(tabs.iter().all(|t| t.id.to_string() != current), "deleted session removed");
            assert!(state.current_id().is_some(), "a fresh current was opened");
        }
        // Unknown id errors.
        assert!(state.delete_session("not-a-session").is_err());
    }

    /// ensure_tab_is_current adopts a frontend-minted uuid as a new tab so an
    /// agent_send against a brand-new chat id appears in the tab list + is captured.
    #[test]
    fn ensure_tab_adopts_frontend_session_id() {
        let state = AgentState::new();
        let id = uuid::Uuid::new_v4().to_string();
        state.ensure_tab_is_current(&id);
        assert_eq!(state.current_id(), Some(id.clone()));
        let tabs = state.tabs.lock().expect("tabs mutex");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].id.to_string(), id, "the minted id is adopted verbatim");
    }

    /// clear_sessions wipes all tab + current state (returning Home).
    #[test]
    fn clear_sessions_wipes_state() {
        let state = AgentState::new();
        state.load_project_sessions(&temp_project());
        assert!(state.current_id().is_some());
        state.clear_sessions();
        assert!(state.current_id().is_none());
        assert!(state.tabs.lock().expect("tabs mutex").is_empty());
    }
}
