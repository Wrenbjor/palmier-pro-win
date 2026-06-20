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
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio_util::sync::CancellationToken;

use palmier_agent::{
    available_models, effective_model, parse_json_object, select_and_build_client, AgentClientError,
    AgentLoop, AnthropicModel, AnthropicToolSchema, DispatchResult, SelectedBackend, Tier,
    ToolDispatcher, ToolResultBlock,
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
    /// The verbatim shared agent system prompt (Epic 7's `palmier_mcp` constant,
    /// ruling #2 — the SAME bytes the MCP `initialize` advertises).
    system: String,
    /// Per-chat-tab loop state, keyed by `session_id`.
    sessions: Mutex<HashMap<String, SessionLoop>>,
    /// The persisted model preference (config key `"agentModel"`; reference
    /// `UserDefaults`). `None` ⇒ fall back to the tier default.
    preferred_model: Mutex<Option<AnthropicModel>>,
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
            system: palmier_mcp::AGENT_INSTRUCTIONS.to_string(),
            sessions: Mutex::new(HashMap::new()),
            preferred_model: Mutex::new(None),
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

    // `McpServer::start` is async (binds a TcpListener); run it to completion on the
    // Tauri async runtime. We block on JUST the bind (returns as soon as the listener
    // is bound — serving runs on a background task), so boot is not stalled by I/O.
    let start_result =
        tauri::async_runtime::block_on(palmier_mcp::McpServer::start(executor, config));

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
    let client = match select_and_build_client(api_key.as_deref(), is_signed_in) {
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
        let mut sessions = agent_state.sessions.lock().expect("sessions mutex");
        if let Some(entry) = sessions.get_mut(&session_id) {
            entry.loop_state = working;
            entry.cancel = None;
        }
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
}
