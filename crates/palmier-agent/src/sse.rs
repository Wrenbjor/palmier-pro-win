//! The shared **Anthropic SSE parser** (`AnthropicSSE::parse`).
//!
//! Ports `AnthropicSSE.parse` from `Agent/Clients/AgentClientTypes.swift`. Consumes
//! the `text/event-stream` byte/line stream from `POST /v1/messages` (stream=true)
//! and emits the [`StreamEvent`] sequence the agentic loop (E8-S4) drives — one
//! parser for **both** transports (BYOK `AnthropicClient`, proxied `PalmierClient`),
//! so they consume identical event streams (`agent-panel.md` §"SSE parser", lines
//! 82-94, 207-209; reconciliation "implement one SSE parser, two transports").
//!
//! ## Event mapping (`data:`-prefixed line → switch on `type`)
//! - `message_start` → read `message.usage` → [`StreamEvent::MessageStart`] (the
//!   reference logged this via `AgentUsageLog` DEBUG-only; the Rust port surfaces it
//!   as an event the loop/telemetry seam can log — see [`crate::event`] docs).
//! - `content_block_start` with `content_block.type == "tool_use"` → record
//!   `pending_tools[index] = (id, name, "")` (no event yet).
//! - `content_block_delta`:
//!   - `text_delta` with non-empty `text` → [`StreamEvent::TextDelta`].
//!   - `input_json_delta` → append `partial_json` to `pending_tools[index].json`.
//! - `content_block_stop` → pop `pending_tools[index]`; **empty json → `"{}"`** →
//!   [`StreamEvent::ToolUseComplete`].
//! - `message_delta` → read `delta.stop_reason` → [`StreamEvent::MessageStop`].
//! - `error` → read `error.message` → [`StreamEvent::Error`] (terminal; the
//!   reference `continuation.finish(throwing:)`).
//! - `ping`, text `content_block_start`, `message_stop`, and anything unrecognized
//!   are ignored.
//!
//! ## Transport-agnostic & partial-line safe
//! Two entry points share the same per-line core:
//! - [`parse_lines`] — drives a complete line iterator → `Vec<StreamEvent>`
//!   (unit-testable against recorded fixtures).
//! - [`SseParser`] — a stateful struct the live transport feeds **raw byte chunks**
//!   into via [`SseParser::feed`]; it buffers an incomplete trailing line across
//!   chunk boundaries so a delta split mid-line is not lost. [`SseParser::finish`]
//!   flushes any complete buffered line at end-of-stream.
//!
//! The HTTP byte stream itself (reqwest) and cancellation (`Task.checkCancellation`
//! per line) are the transport's job in **E8-S3** — this parser stays pure so it is
//! testable against recorded SSE fixtures with no network.

use crate::event::{AnthropicStopReason, StreamEvent, Usage};
use std::collections::BTreeMap;

/// The `data:` line prefix in the SSE wire format. Lines without it (the `event:`
/// label lines, blank separators) are ignored, matching the reference
/// `guard line.hasPrefix("data:")`.
const DATA_PREFIX: &str = "data:";

/// A streaming-tool block in flight: its id, name, and the accumulated raw
/// `input_json` string (concatenated `partial_json` chunks), keyed by block index.
#[derive(Debug, Default)]
struct PendingTool {
    id: String,
    name: String,
    json: String,
}

/// Stateful, partial-line-safe SSE parser the live transport feeds byte chunks
/// into (reference `AnthropicSSE.parse` over `bytes.lines`, made incremental for
/// the reqwest byte stream).
///
/// Drive it with [`SseParser::feed`] per received chunk and [`SseParser::finish`]
/// at end-of-stream. For a complete recorded stream, [`parse_lines`] /
/// [`parse_str`] are simpler.
#[derive(Debug, Default)]
pub struct SseParser {
    /// `block_index → (id, name, json_accumulator)` for streaming tool_use blocks.
    pending_tools: BTreeMap<usize, PendingTool>,
    /// Bytes of an incomplete trailing line carried across `feed` calls.
    line_buf: String,
}

impl SseParser {
    /// A fresh parser with no buffered line and no pending tools.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a raw UTF-8 byte chunk from the transport. Splits on `\n`, processes
    /// every **complete** line, and retains any incomplete trailing line for the
    /// next call (partial-line buffering). Returns the events produced by this
    /// chunk, in order.
    ///
    /// Non-UTF-8 bytes are lossily decoded (`from_utf8_lossy`) — Anthropic SSE is
    /// always UTF-8, so this only guards against a mid-codepoint chunk split, which
    /// the line buffer would otherwise already keep together at a `\n` boundary.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<StreamEvent> {
        self.line_buf.push_str(&String::from_utf8_lossy(chunk));
        let mut events = Vec::new();
        // Drain every complete line (terminated by '\n'); keep the remainder.
        while let Some(nl) = self.line_buf.find('\n') {
            let line: String = self.line_buf.drain(..=nl).collect();
            // `line` still ends in '\n' (and maybe '\r'); process_line trims.
            self.process_line(line.trim_end_matches(['\r', '\n']), &mut events);
        }
        events
    }

    /// Flush any complete buffered line at end-of-stream. A well-formed stream ends
    /// on a newline so the buffer is usually empty, but a server that drops the
    /// final newline still gets its last line processed.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if !self.line_buf.is_empty() {
            let line = std::mem::take(&mut self.line_buf);
            self.process_line(line.trim_end_matches(['\r', '\n']), &mut events);
        }
        events
    }

    /// Process one already-delimited line, pushing any resulting events. This is the
    /// single switch-on-`type` core shared by [`feed`](Self::feed) and
    /// [`parse_lines`].
    fn process_line(&mut self, line: &str, out: &mut Vec<StreamEvent>) {
        let Some(rest) = line.strip_prefix(DATA_PREFIX) else {
            return; // not a `data:` line (event label, blank line, comment)
        };
        let payload = rest.trim();
        if payload.is_empty() {
            return;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(payload) else {
            return; // malformed JSON line — skip, matching the reference `try?`
        };
        let Some(ty) = event.get("type").and_then(|t| t.as_str()) else {
            return;
        };

        match ty {
            "message_start" => {
                if let Some(usage_val) = event.get("message").and_then(|m| m.get("usage")) {
                    let usage: Usage =
                        serde_json::from_value(usage_val.clone()).unwrap_or_default();
                    out.push(StreamEvent::MessageStart { usage });
                }
            }
            "content_block_start" => {
                // Only `tool_use` blocks are recorded; text `content_block_start`
                // is ignored (reference default).
                if let Some(index) = event.get("index").and_then(serde_json::Value::as_u64)
                    && let Some(block) = event.get("content_block")
                    && block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.pending_tools.insert(
                        index as usize,
                        PendingTool {
                            id,
                            name,
                            json: String::new(),
                        },
                    );
                }
            }
            "content_block_delta" => {
                let Some(index) = event.get("index").and_then(serde_json::Value::as_u64) else {
                    return;
                };
                let Some(delta) = event.get("delta") else {
                    return;
                };
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str())
                            && !text.is_empty()
                        {
                            out.push(StreamEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str())
                            && let Some(pending) = self.pending_tools.get_mut(&(index as usize))
                        {
                            pending.json.push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(index) = event.get("index").and_then(serde_json::Value::as_u64)
                    && let Some(pending) = self.pending_tools.remove(&(index as usize))
                {
                    // Empty accumulated json defaults to "{}" (reference rule).
                    out.push(StreamEvent::tool_use_complete(
                        pending.id,
                        pending.name,
                        pending.json,
                    ));
                }
            }
            "message_delta" => {
                if let Some(raw) = event
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|v| v.as_str())
                {
                    out.push(StreamEvent::MessageStop {
                        reason: AnthropicStopReason::from_wire(raw),
                    });
                }
            }
            "error" => {
                let msg = event
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown stream error")
                    .to_string();
                out.push(StreamEvent::Error(msg));
            }
            // ping / message_stop / unrecognized → ignored.
            _ => {}
        }
    }
}

/// Parse a complete iterator of already-split lines into the ordered
/// [`StreamEvent`] sequence (reference `AnthropicSSE.parse` over `bytes.lines`).
///
/// Use this for recorded fixtures and tests; the live transport (E8-S3) drives
/// [`SseParser::feed`] over raw byte chunks instead, to handle partial lines.
pub fn parse_lines<I, S>(lines: I) -> Vec<StreamEvent>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    for line in lines {
        parser.process_line(line.as_ref().trim_end_matches(['\r', '\n']), &mut events);
    }
    events
}

/// Parse a complete SSE payload string (the full `text/event-stream` body) into the
/// ordered [`StreamEvent`] sequence. Splits on `\n`; convenience over
/// [`parse_lines`] for recorded `.sse` fixtures.
#[must_use]
pub fn parse_str(body: &str) -> Vec<StreamEvent> {
    parse_lines(body.split('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text-only stream: two text deltas then end_turn (with a leading
    /// message_start carrying usage and the SSE `event:` label lines + blank
    /// separators the parser must ignore).
    const TEXT_STREAM: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":90,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" there\"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    /// A tool_use stream: text, then a streamed tool_use whose input_json arrives in
    /// THREE chunked `input_json_delta`s, then end via tool_use stop_reason.
    const TOOL_STREAM: &str = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me check.\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_42\",\"name\":\"get_timeline\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pa\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ge\\\":\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"2}\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n";

    #[test]
    fn text_stream_yields_message_start_text_deltas_then_stop() {
        let events = parse_str(TEXT_STREAM);
        assert_eq!(
            events,
            vec![
                StreamEvent::MessageStart {
                    usage: Usage {
                        input_tokens: 10,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 90,
                        output_tokens: 1,
                    }
                },
                StreamEvent::TextDelta("Hello".to_string()),
                StreamEvent::TextDelta(" there".to_string()),
                // The empty text_delta yields nothing (reference !text.isEmpty).
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn tool_stream_accumulates_chunked_input_json_into_one_complete() {
        let events = parse_str(TOOL_STREAM);
        assert_eq!(
            events,
            vec![
                StreamEvent::MessageStart {
                    usage: Usage {
                        input_tokens: 5,
                        ..Default::default()
                    }
                },
                StreamEvent::TextDelta("Let me check.".to_string()),
                StreamEvent::ToolUseComplete {
                    id: "toolu_42".to_string(),
                    name: "get_timeline".to_string(),
                    // The three partial_json chunks concatenate to exact bytes.
                    json: "{\"page\":2}".to_string(),
                },
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::ToolUse
                },
            ]
        );
    }

    #[test]
    fn empty_tool_input_defaults_to_braces() {
        // tool_use with NO input_json_delta → content_block_stop → "{}".
        let stream = "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_x\",\"name\":\"noop\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n";
        let events = parse_str(stream);
        assert_eq!(
            events,
            vec![StreamEvent::ToolUseComplete {
                id: "toolu_x".to_string(),
                name: "noop".to_string(),
                json: "{}".to_string(),
            }]
        );
    }

    #[test]
    fn error_event_yields_terminal_error() {
        let stream = "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n";
        let events = parse_str(stream);
        assert_eq!(events, vec![StreamEvent::Error("Overloaded".to_string())]);
    }

    #[test]
    fn ping_and_unrecognized_events_are_ignored() {
        let stream = "data: {\"type\":\"ping\"}\n\
data: {\"type\":\"some_future_event\",\"foo\":1}\n\
data: {\"type\":\"message_stop\"}\n";
        assert!(parse_str(stream).is_empty());
    }

    #[test]
    fn non_data_lines_and_blank_lines_are_skipped() {
        let stream = "event: content_block_delta\n\
\n\
: this is an sse comment\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}\n";
        assert_eq!(parse_str(stream), vec![StreamEvent::TextDelta("x".to_string())]);
    }

    #[test]
    fn malformed_json_line_is_skipped_not_fatal() {
        let stream = "data: {not valid json\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n";
        assert_eq!(parse_str(stream), vec![StreamEvent::TextDelta("ok".to_string())]);
    }

    #[test]
    fn feed_handles_partial_lines_split_mid_line() {
        // Split the TEXT_STREAM bytes at arbitrary, line-crossing boundaries and
        // feed them in pieces. The event sequence must match the whole-stream parse.
        let bytes = TEXT_STREAM.as_bytes();
        let expected = parse_str(TEXT_STREAM);

        // A set of awkward split points, several mid-line.
        let splits = [7usize, 23, 50, 99, 140, 200, 260, 333, 400];
        let mut parser = SseParser::new();
        let mut got = Vec::new();
        let mut start = 0;
        for &end in splits.iter().chain(std::iter::once(&bytes.len())) {
            let end = end.min(bytes.len());
            if start >= end {
                continue;
            }
            got.extend(parser.feed(&bytes[start..end]));
            start = end;
        }
        got.extend(parser.finish());
        assert_eq!(got, expected, "chunked feed must match whole-stream parse");
    }

    #[test]
    fn feed_byte_by_byte_matches_whole_stream() {
        // The most adversarial split: one byte at a time.
        let expected = parse_str(TOOL_STREAM);
        let mut parser = SseParser::new();
        let mut got = Vec::new();
        for b in TOOL_STREAM.as_bytes() {
            got.extend(parser.feed(&[*b]));
        }
        got.extend(parser.finish());
        assert_eq!(got, expected);
    }

    #[test]
    fn finish_flushes_a_final_line_without_trailing_newline() {
        // Server drops the final '\n' on the last data line.
        let mut parser = SseParser::new();
        let mut events =
            parser.feed(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}");
        // No newline yet → nothing emitted from feed.
        assert!(events.is_empty());
        events.extend(parser.finish());
        assert_eq!(
            events,
            vec![StreamEvent::MessageStop {
                reason: AnthropicStopReason::EndTurn
            }]
        );
    }

    #[test]
    fn unknown_stop_reason_maps_to_other() {
        let stream =
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"brand_new\"}}\n";
        assert_eq!(
            parse_str(stream),
            vec![StreamEvent::MessageStop {
                reason: AnthropicStopReason::Other
            }]
        );
    }

    #[test]
    fn parse_lines_over_recorded_fixture() {
        // Drive the parser over a committed recorded SSE fixture (line iterator),
        // exercising the same path the transport will, and assert the full sequence.
        let fixture = include_str!("../tests/fixtures/tool_use_stream.sse");
        let events = parse_str(fixture);
        // Fixture: usage, a text delta, a chunked tool_use, then tool_use stop.
        assert!(matches!(events.first(), Some(StreamEvent::MessageStart { .. })));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::TextDelta(_))));
        let tool = events.iter().find_map(|e| match e {
            StreamEvent::ToolUseComplete { id, name, json } => Some((id, name, json)),
            _ => None,
        });
        let (id, name, json) = tool.expect("fixture must contain a ToolUseComplete");
        assert_eq!(id, "toolu_fixture");
        assert_eq!(name, "split_clip");
        assert_eq!(json, "{\"clipId\":\"c1\",\"frame\":120}");
        assert_eq!(
            events.last(),
            Some(&StreamEvent::MessageStop {
                reason: AnthropicStopReason::ToolUse
            })
        );
    }
}
