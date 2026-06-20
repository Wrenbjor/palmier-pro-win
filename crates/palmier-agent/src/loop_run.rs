//! The agentic **run loop** + tool dispatch + orphan-tool_use repair (E8-S4).
//!
//! Ports `AgentService.runLoop` / `kickOffStream` / `runPendingToolUses` /
//! `resolveOrphanToolUses` / `dropEmptyAssistantTurn` / `apiMessages`
//! (`agent-panel.md` §"Agentic loop", lines 96-127, 193-211). This is the
//! orchestrator that connects an [`AgentClient`] stream to the shared tool
//! dispatcher: it accumulates the assistant turn from [`StreamEvent`]s, and on a
//! `tool_use` stop dispatches every `tool_use` block, feeds the results back as a
//! new user message, and resumes — until the model ends its turn.
//!
//! ## The two seams (testable with no live API, no real editor)
//! - [`AgentClient`] — the transport. Tests drive a [`MockAgentClient`] /
//!   [`ScriptedAgentClient`] replaying a fixed [`StreamEvent`] script per turn.
//! - [`ToolDispatcher`] — the tool-execution seam. The real wiring adapts
//!   `palmier-tools`' `ToolDispatch::execute(name, args, ctx)` (a thin integration
//!   landed in a later story); tests use a [`MockDispatcher`]. Keeping the seam
//!   **here** (rather than depending on `palmier_tools::ToolContext` directly)
//!   lets the loop unit-test against a scripted dispatcher with no editor state.
//!
//! ## What the loop does NOT own (deferred)
//! - **Tauri integration + the real `palmier-tools` `ToolExecutor` wiring** — a
//!   thin adapter (`ToolDispatcher` over `palmier_tools::ToolDispatch` + a
//!   `ToolContext`) plus the command/event surface. Deferred to the integration
//!   story (E8-S9 / the Tauri wiring story).
//! - **Mentions / context-hints / image inlining** in [`AgentLoop::api_messages`]
//!   — E8-S5 enriches the user-turn projection. S4 ships the structural
//!   projection (blocks → wire objects, drop-empty-text, `tool_use` re-parse,
//!   `tool_result` shape) the loop needs to round-trip a tool turn.
//!
//! ## Cancellation (`agent-panel.md` lines 109-110, 210-211)
//! The loop takes a [`CancellationToken`]; it is threaded into the client via
//! [`AnthropicClient::stream_with_cancel`](crate::anthropic_client::AnthropicClient::stream_with_cancel)
//! (E8-S3) for the live path, and checked at every loop iteration + before
//! dispatching each tool. A cancel drops the in-flight assistant turn cleanly:
//! if the assistant message accumulated no blocks it is removed
//! ([`drop_empty_assistant_turn`]) — no half-written `tool_use` is committed. A
//! mid-tool cancel yields a `"Cancelled"` `is_error` `tool_result` (so the turn
//! stays well-formed) rather than aborting the message.

use crate::client::{AgentClient, AgentClientError, AgentRequest, AnthropicToolSchema, WireMessage};
use crate::event::{AnthropicModel, AnthropicStopReason, StreamEvent};
use crate::model::{AgentContentBlock, AgentMessage, Role, ToolResultBlock};
use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// The result of dispatching one tool call, in the agent's own block vocabulary.
///
/// Mirrors `palmier_tools::ToolResult` (content blocks + `is_error`) but lives in
/// `palmier-agent` so the loop's tool seam does not pull in `palmier-tools`'
/// `ToolContext`. The real adapter maps a `palmier_tools::ToolResult` into this
/// (text/image blocks pass through 1:1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchResult {
    /// The result content blocks (text and/or images), fed back to the model.
    pub content: Vec<ToolResultBlock>,
    /// Whether the tool errored (reference `ToolResult.isError`).
    pub is_error: bool,
}

impl DispatchResult {
    /// A successful single-text result.
    #[must_use]
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultBlock::text(text)],
            is_error: false,
        }
    }

    /// An error single-text result.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultBlock::text(text)],
            is_error: true,
        }
    }
}

/// The tool-execution seam the loop dispatches into (reference
/// `ToolExecutor.execute(name:args:)`).
///
/// The single real implementation adapts `palmier_tools::ToolDispatch` — the SAME
/// dispatcher the MCP server uses (no duplication; PRD §10, FOUNDATION §4) — over a
/// `ToolContext` carrying the editor handle. Tests use [`MockDispatcher`].
///
/// `execute` is synchronous: the reference runs every tool call on its
/// single-owner `@MainActor`; the port serializes through one owner
/// (`Mutex<EditorState>` / a command actor). The loop calls it inline within the
/// chat task, between stream turns — never concurrently with itself.
pub trait ToolDispatcher: Send + Sync {
    /// Dispatch tool `name` with the **raw, verbatim** `input_json` string
    /// (`agent-panel.md` lines 201-203: the loop re-parses `input_json` into an
    /// object here, the second of exactly two parse sites). Never panics; every
    /// error path returns a [`DispatchResult`] with `is_error = true`.
    fn execute(&self, name: &str, input_json: &str) -> DispatchResult;
}

/// The verbatim default `reason` injected for an orphaned `tool_use`
/// (reference `resolveOrphanToolUses(reason: "Cancelled")`).
pub const ORPHAN_REASON: &str = "Cancelled";

/// The user-text fallback appended when no [`ToolDispatcher`] is present
/// (reference `"Tool executor unavailable."`).
pub const TOOL_EXECUTOR_UNAVAILABLE: &str = "Tool executor unavailable.";

/// The agentic run loop over a conversation (reference `AgentService`'s loop
/// portion — `messages` + `runLoop`/`runPendingToolUses`/`resolveOrphanToolUses`).
///
/// Holds the conversation [`messages`](Self::messages) and the per-request inputs
/// (system prompt + tool catalogue + model). [`run_turn`](Self::run_turn) drives
/// one user-initiated turn to completion (through any number of tool rounds).
/// The session/tab orchestration, `send()` gating, and Tauri event emission live
/// outside this struct (E8-S7 / the integration story); the loop is a pure,
/// testable state machine over `(client, dispatcher, cancel)`.
#[derive(Debug, Clone)]
pub struct AgentLoop {
    /// The full conversation, mutated in place across tool rounds (reference
    /// `AgentService.messages`).
    pub messages: Vec<AgentMessage>,
    /// The verbatim shared agent system prompt (Epic 7 owns the constant).
    pub system: String,
    /// The tool catalogue advertised every turn.
    pub tools: Vec<AnthropicToolSchema>,
    /// The target model.
    pub model: AnthropicModel,
}

impl AgentLoop {
    /// A loop with an empty conversation for `model`, `system`, and `tools`.
    #[must_use]
    pub fn new(
        model: AnthropicModel,
        system: impl Into<String>,
        tools: Vec<AnthropicToolSchema>,
    ) -> Self {
        Self {
            messages: Vec::new(),
            system: system.into(),
            tools,
            model,
        }
    }

    /// Append a user turn (reference `send()` minus the gating/mention snapshot,
    /// which the caller owns). Runs orphan repair first, matching `send()`'s
    /// `resolveOrphanToolUses()` before appending.
    pub fn push_user_text(&mut self, text: impl Into<String>) {
        self.resolve_orphan_tool_uses(ORPHAN_REASON);
        self.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text(text)],
        ));
    }

    /// Drive one turn to completion: stream the assistant turn, and on a
    /// `tool_use` stop dispatch every pending tool, append the results as a user
    /// message, and resume — until the model ends its turn (reference `runLoop`).
    ///
    /// Returns the terminal [`AnthropicStopReason`] (`end_turn` on a normal finish,
    /// or whatever non-`tool_use` reason ended it), or an [`AgentClientError`] if
    /// the stream surfaced a terminal error. On cancellation it returns
    /// `Ok(`[`AnthropicStopReason::Other`]`)` after dropping the empty assistant
    /// turn (no error — a cancel is a clean stop, not a failure).
    ///
    /// `client` is driven via its [`AgentClient::stream`]; the live
    /// [`AnthropicClient`](crate::anthropic_client::AnthropicClient) honors the
    /// shared `cancel` token through `stream_with_cancel`. `cancel` is also checked
    /// directly at each iteration and before each tool dispatch so the loop drops
    /// the turn even when the transport cannot.
    ///
    /// # Errors
    /// Returns the [`AgentClientError`] carried by a terminal [`StreamEvent::Error`]
    /// (HTTP ≥ 400, stream error, etc.), after dropping the empty assistant turn.
    pub async fn run_turn(
        &mut self,
        client: &dyn AgentClient,
        dispatcher: Option<&dyn ToolDispatcher>,
        cancel: &CancellationToken,
    ) -> Result<AnthropicStopReason, AgentClientError> {
        loop {
            if cancel.is_cancelled() {
                return Ok(AnthropicStopReason::Other);
            }

            // (a) Orphan repair before EVERY iteration, then project + append the
            //     empty assistant turn we accumulate into.
            self.resolve_orphan_tool_uses(ORPHAN_REASON);
            let request = self.build_request();
            let assistant = AgentMessage::new(Role::Assistant, Vec::new());
            let assistant_id = assistant.id;
            self.messages.push(assistant);

            // (b) Stream the turn, accumulating blocks in place.
            let mut stop_reason = AnthropicStopReason::EndTurn;
            let mut stream = client.stream(request);
            let mut terminal_error: Option<AgentClientError> = None;

            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        // Cancellation: drop the (possibly empty) assistant turn and
                        // stop. No half-written tool_use is committed if it was empty.
                        drop(stream);
                        self.drop_empty_assistant_turn(assistant_id);
                        return Ok(AnthropicStopReason::Other);
                    }
                    event = stream.next() => {
                        match event {
                            Some(StreamEvent::TextDelta(chunk)) => {
                                self.append_text_delta(&chunk, assistant_id);
                            }
                            Some(StreamEvent::ToolUseComplete { id, name, json }) => {
                                self.append_tool_use(assistant_id, id, name, json);
                            }
                            Some(StreamEvent::MessageStop { reason }) => {
                                stop_reason = reason;
                            }
                            Some(StreamEvent::MessageStart { .. }) => {
                                // Usage is informational (DEBUG telemetry seam).
                            }
                            Some(StreamEvent::Error(msg)) => {
                                terminal_error = Some(AgentClientError::StreamError(msg));
                            }
                            None => break,
                        }
                    }
                }
            }

            // (c) Terminal transport error → drop empty turn, surface the error.
            if let Some(err) = terminal_error {
                self.drop_empty_assistant_turn(assistant_id);
                return Err(err);
            }

            // (d) tool_use → run the pending tools and resume; else end the turn.
            if stop_reason == AnthropicStopReason::ToolUse {
                self.run_pending_tool_uses(assistant_id, dispatcher, cancel);
                continue;
            }
            return Ok(stop_reason);
        }
    }

    /// Build the [`AgentRequest`] for the current turn (system + tools + the
    /// wire-projected conversation).
    fn build_request(&self) -> AgentRequest {
        AgentRequest {
            model: self.model,
            max_tokens: crate::client::DEFAULT_MAX_TOKENS,
            system: self.system.clone(),
            tools: self.tools.clone(),
            messages: self.api_messages(),
        }
    }

    /// The index of the assistant message with `id` (reference
    /// `assistantMessageIndex`).
    fn assistant_index(&self, id: uuid::Uuid) -> Option<usize> {
        self.messages
            .iter()
            .position(|m| m.id == id && m.role == Role::Assistant)
    }

    /// Remove the assistant message `id` iff it has no blocks (reference
    /// `dropEmptyAssistantTurn`). Used on cancellation / error so a turn that
    /// produced nothing leaves no trace.
    pub fn drop_empty_assistant_turn(&mut self, id: uuid::Uuid) {
        if let Some(index) = self.assistant_index(id)
            && self.messages[index].blocks.is_empty()
        {
            self.messages.remove(index);
        }
    }

    /// Append a text delta to the assistant turn, extending the last `.text` block
    /// **in place** if present (reference `appendTextDelta`).
    fn append_text_delta(&mut self, chunk: &str, assistant_id: uuid::Uuid) {
        let Some(index) = self.assistant_index(assistant_id) else {
            return;
        };
        if let Some(AgentContentBlock::Text { text }) = self.messages[index].blocks.last_mut() {
            text.push_str(chunk);
        } else {
            self.messages[index]
                .blocks
                .push(AgentContentBlock::text(chunk));
        }
    }

    /// Append a `.toolUse` block to the assistant turn (reference `appendToolUse`).
    fn append_tool_use(
        &mut self,
        assistant_id: uuid::Uuid,
        id: String,
        name: String,
        input_json: String,
    ) {
        let Some(index) = self.assistant_index(assistant_id) else {
            return;
        };
        self.messages[index]
            .blocks
            .push(AgentContentBlock::tool_use(id, name, input_json));
    }

    /// Dispatch every pending `.toolUse` of the assistant message and append a
    /// single user message of `.toolResult` blocks (reference
    /// `runPendingToolUses`).
    ///
    /// - Skips ids already resolved in the immediately-following user message.
    /// - A cancelled token (or a cancel arriving mid-loop) yields a `"Cancelled"`
    ///   `is_error` result for the remaining tools rather than aborting.
    /// - With no dispatcher, appends the `"Tool executor unavailable."` user text.
    fn run_pending_tool_uses(
        &mut self,
        assistant_id: uuid::Uuid,
        dispatcher: Option<&dyn ToolDispatcher>,
        cancel: &CancellationToken,
    ) {
        let Some(assistant_index) = self.assistant_index(assistant_id) else {
            return;
        };

        let Some(dispatcher) = dispatcher else {
            self.messages.push(AgentMessage::new(
                Role::User,
                vec![AgentContentBlock::text(TOOL_EXECUTOR_UNAVAILABLE)],
            ));
            return;
        };

        // Collect (id, name, input) for every tool_use block of the assistant turn.
        let tool_uses: Vec<(String, String, String)> = self.messages[assistant_index]
            .blocks
            .iter()
            .filter_map(|b| match b {
                AgentContentBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => Some((id.clone(), name.clone(), input_json.clone())),
                _ => None,
            })
            .collect();
        let already_resolved = self.resolved_tool_use_ids(assistant_index);

        let mut result_blocks: Vec<AgentContentBlock> = Vec::new();
        for (id, name, input) in tool_uses {
            if already_resolved.contains(&id) {
                continue;
            }
            if cancel.is_cancelled() {
                result_blocks.push(AgentContentBlock::tool_result(
                    id,
                    vec![ToolResultBlock::text(ORPHAN_REASON)],
                    true,
                ));
                continue;
            }
            let result = dispatcher.execute(&name, &input);
            result_blocks.push(AgentContentBlock::tool_result(
                id,
                result.content,
                result.is_error,
            ));
        }

        if !result_blocks.is_empty() {
            self.messages
                .push(AgentMessage::new(Role::User, result_blocks));
        }
    }

    /// The set of `tool_use` ids already answered by the user message immediately
    /// after `index` (reference `resolvedToolUseIds`).
    fn resolved_tool_use_ids(&self, index: usize) -> std::collections::HashSet<String> {
        let next = index + 1;
        if next >= self.messages.len() || self.messages[next].role != Role::User {
            return std::collections::HashSet::new();
        }
        self.messages[next]
            .blocks
            .iter()
            .filter_map(|b| match b {
                AgentContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// **Orphan-tool_use repair** (reference `resolveOrphanToolUses`): for every
    /// assistant message carrying a `tool_use` id with no matching `tool_result` in
    /// the next user message, inject a synthetic
    /// `tool_result(content:[text(reason)], is_error:true)`.
    ///
    /// **Load-bearing:** Anthropic rejects any `tool_use` without a matching
    /// `tool_result` in the next turn (`agent-panel.md` lines 115-120, 193-197).
    /// Runs before every send AND every loop iteration.
    ///
    /// The **prepend-vs-insert** branch (reference, verbatim): if the next message
    /// already exists and is a user message with at least one `tool_result`, the
    /// synthetic blocks are **prepended** into it; otherwise a **new** user message
    /// of synthetic results is **inserted** at `next`.
    pub fn resolve_orphan_tool_uses(&mut self, reason: &str) {
        let mut i = 0;
        while i < self.messages.len() {
            if self.messages[i].role != Role::Assistant {
                i += 1;
                continue;
            }
            let tool_use_ids: Vec<String> = self.messages[i]
                .blocks
                .iter()
                .filter_map(|b| match b {
                    AgentContentBlock::ToolUse { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .collect();
            if tool_use_ids.is_empty() {
                i += 1;
                continue;
            }

            let next = i + 1;
            let next_is_tool_result = next < self.messages.len()
                && self.messages[next].role == Role::User
                && self.messages[next]
                    .blocks
                    .iter()
                    .any(|b| matches!(b, AgentContentBlock::ToolResult { .. }));
            let resolved: std::collections::HashSet<String> = if next_is_tool_result {
                self.messages[next]
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        AgentContentBlock::ToolResult { tool_use_id, .. } => {
                            Some(tool_use_id.clone())
                        }
                        _ => None,
                    })
                    .collect()
            } else {
                std::collections::HashSet::new()
            };

            let orphans: Vec<String> = tool_use_ids
                .into_iter()
                .filter(|id| !resolved.contains(id))
                .collect();
            if orphans.is_empty() {
                i += 1;
                continue;
            }

            let synthetic: Vec<AgentContentBlock> = orphans
                .into_iter()
                .map(|id| {
                    AgentContentBlock::tool_result(
                        id,
                        vec![ToolResultBlock::text(reason)],
                        true,
                    )
                })
                .collect();

            if next_is_tool_result {
                // Prepend into the existing next user message (order: reference
                // `insert(contentsOf:at:0)`).
                let existing = std::mem::take(&mut self.messages[next].blocks);
                let mut merged = synthetic;
                merged.extend(existing);
                self.messages[next].blocks = merged;
            } else {
                self.messages
                    .insert(next, AgentMessage::new(Role::User, synthetic));
            }
            i += 1;
        }
    }

    /// Project the stored conversation to the wire [`WireMessage`] list (reference
    /// `apiMessages`).
    ///
    /// **S4 scope (structural projection):** each block maps via
    /// [`content_block_json`] (drops empty text); `tool_use` re-parses `input_json`
    /// into an object (the FIRST of the two parse sites — `agent-panel.md` lines
    /// 201-203); `tool_result` emits `{type:tool_result, tool_use_id, content,
    /// is_error}`. Messages whose content ends up empty are skipped.
    ///
    /// **Deferred to E8-S5:** for user messages with mentions, prepend the
    /// context-hint text block + inlined image blocks at index 0. S4 ships the
    /// projection without the mention enrichment so the tool round-trip is exact.
    #[must_use]
    pub fn api_messages(&self) -> Vec<WireMessage> {
        self.messages
            .iter()
            .filter_map(|msg| {
                let content: Vec<Value> =
                    msg.blocks.iter().filter_map(content_block_json).collect();
                if content.is_empty() {
                    return None;
                }
                Some(WireMessage {
                    role: match msg.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                    },
                    content,
                })
            })
            .collect()
    }
}

/// Parse a raw `input_json` string into a JSON object (reference
/// `parseJSONObject`). A non-object / unparseable string yields `{}` — the loop
/// never fails a turn on a malformed tool input; the tool body validates.
#[must_use]
pub fn parse_json_object(json: &str) -> Value {
    match serde_json::from_str::<Value>(json) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Project one [`AgentContentBlock`] to its Anthropic wire object (reference
/// `contentBlockJSON`). Empty text → `None` (dropped).
fn content_block_json(block: &AgentContentBlock) -> Option<Value> {
    match block {
        AgentContentBlock::Text { text } => {
            if text.is_empty() {
                None
            } else {
                Some(serde_json::json!({ "type": "text", "text": text }))
            }
        }
        AgentContentBlock::ToolUse {
            id,
            name,
            input_json,
        } => Some(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": parse_json_object(input_json),
        })),
        AgentContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let content_json: Vec<Value> = content
                .iter()
                .map(|b| match b {
                    ToolResultBlock::Text { text } => {
                        serde_json::json!({ "type": "text", "text": text })
                    }
                    ToolResultBlock::Image { base64, media_type } => serde_json::json!({
                        "type": "image",
                        "source": { "type": "base64", "media_type": media_type, "data": base64 },
                    }),
                })
                .collect();
            Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content_json,
                "is_error": is_error,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::AgentClient;
    use futures_core::stream::BoxStream;
    use std::sync::Mutex;

    /// A client that replays a DIFFERENT script per `stream` call — one entry per
    /// turn — so a multi-round loop (tool_use → resume → end_turn) is driven by a
    /// list of scripts. Exhausting the list yields an `end_turn`-only stream (the
    /// loop must have stopped before then in a well-formed test).
    struct ScriptedAgentClient {
        scripts: Mutex<std::collections::VecDeque<Vec<StreamEvent>>>,
    }

    impl ScriptedAgentClient {
        fn new(scripts: Vec<Vec<StreamEvent>>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into_iter().collect()),
            }
        }
    }

    impl AgentClient for ScriptedAgentClient {
        fn stream<'a>(&'a self, _request: AgentRequest) -> BoxStream<'a, StreamEvent> {
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    vec![StreamEvent::MessageStop {
                        reason: AnthropicStopReason::EndTurn,
                    }]
                });
            Box::pin(futures_util::stream::iter(script))
        }
    }

    /// A dispatcher that records every call and echoes an `ok` result naming the
    /// tool + its verbatim input (so tests can assert both the call log and that
    /// the loop forwarded the raw `input_json`).
    #[derive(Default)]
    struct MockDispatcher {
        calls: Mutex<Vec<(String, String)>>,
    }

    impl MockDispatcher {
        fn echo() -> Self {
            Self::default()
        }
        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ToolDispatcher for MockDispatcher {
        fn execute(&self, name: &str, input_json: &str) -> DispatchResult {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), input_json.to_string()));
            DispatchResult::ok(format!("ran {name} with {input_json}"))
        }
    }

    fn loop_with_user(text: &str) -> AgentLoop {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "you are the editor", Vec::new());
        l.push_user_text(text);
        l
    }

    fn tool_use(id: &str, name: &str, json: &str) -> StreamEvent {
        StreamEvent::tool_use_complete(id, name, json)
    }

    fn stop(reason: AnthropicStopReason) -> StreamEvent {
        StreamEvent::MessageStop { reason }
    }

    // ── (b) text-only turn → end_turn ───────────────────────────────────────

    #[tokio::test]
    async fn text_only_turn_ends_with_end_turn() {
        let mut l = loop_with_user("hello");
        let client = ScriptedAgentClient::new(vec![vec![
            StreamEvent::TextDelta("Hi ".to_string()),
            StreamEvent::TextDelta("there".to_string()),
            stop(AnthropicStopReason::EndTurn),
        ]]);
        let cancel = CancellationToken::new();
        let reason = l.run_turn(&client, None, &cancel).await.unwrap();
        assert_eq!(reason, AnthropicStopReason::EndTurn);
        // user + assistant; assistant text accumulated in place into ONE block.
        assert_eq!(l.messages.len(), 2);
        assert_eq!(l.messages[1].role, Role::Assistant);
        assert_eq!(
            l.messages[1].blocks,
            vec![AgentContentBlock::text("Hi there")]
        );
    }

    // ── (b) single tool_use turn → dispatch → result → resume → end_turn ─────

    #[tokio::test]
    async fn single_tool_use_round_trips_then_ends() {
        let mut l = loop_with_user("cut the intro");
        let client = ScriptedAgentClient::new(vec![
            // Turn 1: emit a tool_use, stop with tool_use.
            vec![
                tool_use("toolu_1", "get_timeline", r#"{"page":1}"#),
                stop(AnthropicStopReason::ToolUse),
            ],
            // Turn 2 (resume after results): end_turn with a closing text.
            vec![
                StreamEvent::TextDelta("done".to_string()),
                stop(AnthropicStopReason::EndTurn),
            ],
        ]);
        let dispatcher = MockDispatcher::echo();
        let cancel = CancellationToken::new();
        let reason = l.run_turn(&client, Some(&dispatcher), &cancel).await.unwrap();
        assert_eq!(reason, AnthropicStopReason::EndTurn);

        // The dispatcher saw exactly one call with the verbatim input.
        assert_eq!(
            dispatcher.calls(),
            vec![("get_timeline".to_string(), r#"{"page":1}"#.to_string())]
        );

        // Message sequence: user, assistant(tool_use), user(tool_result), assistant(text).
        assert_eq!(l.messages.len(), 4);
        assert_eq!(l.messages[0].role, Role::User);
        assert_eq!(l.messages[1].role, Role::Assistant);
        assert!(matches!(
            l.messages[1].blocks[0],
            AgentContentBlock::ToolUse { .. }
        ));
        assert_eq!(l.messages[2].role, Role::User);
        match &l.messages[2].blocks[0] {
            AgentContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_1");
                assert!(!is_error);
            }
            _ => panic!("expected tool_result"),
        }
        assert_eq!(l.messages[3].blocks, vec![AgentContentBlock::text("done")]);
    }

    // ── (b) multi tool_use in one turn → one user message of all results ─────

    #[tokio::test]
    async fn multi_tool_use_turn_collects_all_results_in_one_user_message() {
        let mut l = loop_with_user("inspect and split");
        let client = ScriptedAgentClient::new(vec![
            vec![
                tool_use("toolu_a", "get_timeline", "{}"),
                tool_use("toolu_b", "split_clip", r#"{"frame":120}"#),
                stop(AnthropicStopReason::ToolUse),
            ],
            vec![stop(AnthropicStopReason::EndTurn)],
        ]);
        let dispatcher = MockDispatcher::echo();
        let cancel = CancellationToken::new();
        l.run_turn(&client, Some(&dispatcher), &cancel).await.unwrap();

        assert_eq!(dispatcher.calls().len(), 2);
        // The results message carries BOTH tool_results in order.
        let results = &l.messages[2];
        assert_eq!(results.role, Role::User);
        assert_eq!(results.blocks.len(), 2);
        let ids: Vec<&str> = results
            .blocks
            .iter()
            .filter_map(|b| match b {
                AgentContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["toolu_a", "toolu_b"]);
    }

    // ── orphan repair: insert branch (no following user msg) ─────────────────

    #[test]
    fn orphan_repair_inserts_new_user_message_for_dangling_tool_use() {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "sys", Vec::new());
        l.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text("go")],
        ));
        // An assistant turn with a tool_use and NO following user message.
        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![AgentContentBlock::tool_use("toolu_x", "get_timeline", "{}")],
        ));

        l.resolve_orphan_tool_uses(ORPHAN_REASON);

        // A new user message of one synthetic Cancelled result is inserted after.
        assert_eq!(l.messages.len(), 3);
        assert_eq!(l.messages[2].role, Role::User);
        assert_eq!(l.messages[2].blocks.len(), 1);
        match &l.messages[2].blocks[0] {
            AgentContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_x");
                assert!(is_error);
                assert_eq!(content, &vec![ToolResultBlock::text("Cancelled")]);
            }
            _ => panic!("expected synthetic tool_result"),
        }
    }

    // ── orphan repair: prepend branch (following user msg has SOME results) ──

    #[test]
    fn orphan_repair_prepends_into_existing_results_message() {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "sys", Vec::new());
        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![
                AgentContentBlock::tool_use("toolu_1", "a", "{}"),
                AgentContentBlock::tool_use("toolu_2", "b", "{}"),
            ],
        ));
        // The following user message already answers toolu_2 but NOT toolu_1.
        l.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::tool_result(
                "toolu_2",
                vec![ToolResultBlock::text("ok")],
                false,
            )],
        ));

        l.resolve_orphan_tool_uses(ORPHAN_REASON);

        // Still two messages (prepended, not inserted).
        assert_eq!(l.messages.len(), 2);
        let blocks = &l.messages[1].blocks;
        assert_eq!(blocks.len(), 2);
        // The synthetic Cancelled for toolu_1 is PREPENDED (index 0); the real
        // result for toolu_2 follows.
        match &blocks[0] {
            AgentContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_1");
                assert!(is_error);
            }
            _ => panic!("expected synthetic result first"),
        }
        match &blocks[1] {
            AgentContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_2");
                assert!(!is_error);
            }
            _ => panic!("expected the real result second"),
        }
    }

    #[test]
    fn orphan_repair_is_noop_when_all_resolved() {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "sys", Vec::new());
        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![AgentContentBlock::tool_use("toolu_1", "a", "{}")],
        ));
        l.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::tool_result(
                "toolu_1",
                vec![ToolResultBlock::text("ok")],
                false,
            )],
        ));
        let before = l.messages.clone();
        l.resolve_orphan_tool_uses(ORPHAN_REASON);
        assert_eq!(l.messages, before, "fully-resolved turn must be untouched");
    }

    // ── cancellation: mid-tool cancel yields a "Cancelled" is_error result ───

    #[test]
    fn cancel_mid_tool_yields_cancelled_result_not_aborted_message() {
        // Drive run_pending_tool_uses directly with an already-cancelled token:
        // the dispatcher must NOT run and a "Cancelled" is_error result is appended
        // (reference: mid-tool cancel yields a result, not an aborted message).
        let mut l = loop_with_user("cut");
        let dispatcher = MockDispatcher::echo();
        let cancel = CancellationToken::new();

        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![AgentContentBlock::tool_use("toolu_1", "get_timeline", "{}")],
        ));
        let assistant_id = l.messages.last().unwrap().id;
        cancel.cancel();
        l.run_pending_tool_uses(assistant_id, Some(&dispatcher), &cancel);

        assert!(
            dispatcher.calls().is_empty(),
            "dispatcher must not run when cancelled"
        );
        let results = l.messages.last().unwrap();
        assert_eq!(results.role, Role::User);
        match &results.blocks[0] {
            AgentContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_1");
                assert!(is_error);
                assert_eq!(content, &vec![ToolResultBlock::text("Cancelled")]);
            }
            _ => panic!("expected a Cancelled tool_result"),
        }
    }

    #[tokio::test]
    async fn cancel_drops_empty_assistant_turn_cleanly() {
        let mut l = loop_with_user("hello");
        // A stream that never completes — the cancel must drop the turn.
        struct PendingClient;
        impl AgentClient for PendingClient {
            fn stream<'a>(&'a self, _request: AgentRequest) -> BoxStream<'a, StreamEvent> {
                Box::pin(futures_util::stream::pending())
            }
        }
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        // Cancel shortly after starting.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            token.cancel();
        });
        let reason = l.run_turn(&PendingClient, None, &cancel).await.unwrap();
        assert_eq!(reason, AnthropicStopReason::Other);
        // The empty assistant turn was dropped: only the original user message left.
        assert_eq!(l.messages.len(), 1);
        assert_eq!(l.messages[0].role, Role::User);
    }

    #[tokio::test]
    async fn already_cancelled_token_returns_without_streaming() {
        let mut l = loop_with_user("hello");
        let client = ScriptedAgentClient::new(vec![vec![
            StreamEvent::TextDelta("should not appear".to_string()),
            stop(AnthropicStopReason::EndTurn),
        ]]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let reason = l.run_turn(&client, None, &cancel).await.unwrap();
        assert_eq!(reason, AnthropicStopReason::Other);
        // No assistant message was appended (loop bailed before streaming).
        assert_eq!(l.messages.len(), 1);
    }

    // ── terminal stream error → drop empty turn, surface error ───────────────

    #[tokio::test]
    async fn terminal_stream_error_is_surfaced_and_empty_turn_dropped() {
        let mut l = loop_with_user("hello");
        let client = ScriptedAgentClient::new(vec![vec![StreamEvent::Error(
            "Anthropic API error (429): rate limited".to_string(),
        )]]);
        let cancel = CancellationToken::new();
        let err = l.run_turn(&client, None, &cancel).await.unwrap_err();
        match err {
            AgentClientError::StreamError(msg) => assert!(msg.contains("429")),
            other => panic!("expected StreamError, got {other:?}"),
        }
        // The empty assistant turn was dropped.
        assert_eq!(l.messages.len(), 1);
    }

    // ── no dispatcher → "Tool executor unavailable." user text ───────────────

    #[tokio::test]
    async fn tool_use_without_dispatcher_appends_unavailable_then_resumes() {
        let mut l = loop_with_user("cut");
        let client = ScriptedAgentClient::new(vec![
            vec![
                tool_use("toolu_1", "get_timeline", "{}"),
                stop(AnthropicStopReason::ToolUse),
            ],
            vec![stop(AnthropicStopReason::EndTurn)],
        ]);
        let cancel = CancellationToken::new();
        l.run_turn(&client, None, &cancel).await.unwrap();
        // The "unavailable" user text was appended (reference behavior). Because it
        // is NOT a tool_result, orphan repair injects a Cancelled result before the
        // next iteration so the tool_use is never left dangling.
        let has_unavailable = l.messages.iter().any(|m| {
            m.blocks
                .iter()
                .any(|b| matches!(b, AgentContentBlock::Text { text } if text == TOOL_EXECUTOR_UNAVAILABLE))
        });
        assert!(has_unavailable, "expected the unavailable user text");
        // And the dangling tool_use was repaired (a synthetic Cancelled exists).
        let has_synthetic = l.messages.iter().any(|m| {
            m.blocks.iter().any(|b| matches!(
                b,
                AgentContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "toolu_1"
            ))
        });
        assert!(has_synthetic, "dangling tool_use must be repaired");
    }

    // ── api_messages structural projection ───────────────────────────────────

    #[test]
    fn api_messages_projects_tool_round_trip_shapes_and_drops_empty_text() {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "sys", Vec::new());
        l.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text("cut the intro")],
        ));
        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![
                AgentContentBlock::text(""), // empty text → dropped
                AgentContentBlock::tool_use("toolu_1", "get_timeline", r#"{"page":2}"#),
            ],
        ));
        l.messages.push(AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::tool_result(
                "toolu_1",
                vec![ToolResultBlock::text("timeline json")],
                false,
            )],
        ));

        let wire = l.api_messages();
        assert_eq!(wire.len(), 3);

        // user text
        assert_eq!(wire[0].role, "user");
        assert_eq!(wire[0].content[0]["type"], "text");

        // assistant: empty text dropped, tool_use input re-parsed into an OBJECT.
        assert_eq!(wire[1].role, "assistant");
        assert_eq!(wire[1].content.len(), 1);
        assert_eq!(wire[1].content[0]["type"], "tool_use");
        assert_eq!(wire[1].content[0]["input"], serde_json::json!({ "page": 2 }));

        // tool_result wire shape.
        assert_eq!(wire[2].content[0]["type"], "tool_result");
        assert_eq!(wire[2].content[0]["tool_use_id"], "toolu_1");
        assert_eq!(wire[2].content[0]["is_error"], false);
        assert_eq!(wire[2].content[0]["content"][0]["type"], "text");
    }

    #[test]
    fn api_messages_skips_fully_empty_messages() {
        let mut l = AgentLoop::new(AnthropicModel::Haiku45, "sys", Vec::new());
        l.messages.push(AgentMessage::new(
            Role::Assistant,
            vec![AgentContentBlock::text("")], // only empty text → message dropped
        ));
        assert!(l.api_messages().is_empty());
    }

    #[test]
    fn parse_json_object_defaults_malformed_to_empty_object() {
        assert_eq!(parse_json_object("{}"), serde_json::json!({}));
        assert_eq!(parse_json_object(r#"{"a":1}"#), serde_json::json!({ "a": 1 }));
        // Non-object / malformed → {}.
        assert_eq!(parse_json_object("[1,2]"), serde_json::json!({}));
        assert_eq!(parse_json_object("not json"), serde_json::json!({}));
        assert_eq!(parse_json_object(""), serde_json::json!({}));
    }
}
