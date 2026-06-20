//! E2-S8 R-6 round-trip regression gate (PRD §11 / FINDINGS §4).
//!
//! Decodes the committed `spikes/s1b-convex-date/sample-payload.json` Date
//! fields through the E2-S8 codecs, re-encodes, and asserts **semantic**
//! equality (decode A → encode → decode B → `A == B`). Per FINDINGS §3a, raw
//! byte-identity of the Apple-epoch f64 is NOT guaranteed (serde_json's Ryu vs
//! Swift's `JSONEncoder` formatting), so the gate is semantic plus a known
//! vector pinning the offset/direction.
//!
//! The full `MediaManifest` / `GenerationLog` shapes land in E2-S7; this test
//! uses minimal local mirror structs that wire the *same* `#[serde(with = …)]`
//! codecs E2-S7 will apply, proving the codec re-encodes the captured payload
//! identically in semantics.
//!
//! When a real `/v1/samples/resolve` payload lands (S-2 window), replace/augment
//! the fixture and re-run; green retires R-6.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::macros::datetime;
use time::OffsetDateTime;

/// Minimal mirror of the `generationInput` sub-object (E2-S7 `GenerationInput`),
/// only the Date field wired through the apple-epoch codec.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct GenerationInputMirror {
    prompt: String,
    model: String,
    #[serde(
        rename = "createdAt",
        with = "palmier_model::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    created_at: Option<OffsetDateTime>,
}

/// Minimal mirror of a `media.json` entry's Date-bearing fields (E2-S7
/// `MediaManifestEntry`).
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct ManifestEntryMirror {
    id: String,
    name: String,
    #[serde(
        rename = "cachedRemoteURLExpiresAt",
        with = "palmier_model::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    cached_remote_url_expires_at: Option<OffsetDateTime>,
    #[serde(rename = "generationInput", default, skip_serializing_if = "Option::is_none")]
    generation_input: Option<GenerationInputMirror>,
}

/// Minimal mirror of a `generation-log.json` entry (E2-S7 `GenerationLogEntry`).
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct LogEntryMirror {
    id: String,
    model: String,
    #[serde(
        rename = "createdAt",
        with = "palmier_model::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    created_at: Option<OffsetDateTime>,
}

/// Minimal mirror of a `chat/<uuid>.json` blob (E2-S7/S8 `ChatSession`), only
/// the ISO-8601 Date field.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct ChatSessionMirror {
    id: String,
    title: String,
    #[serde(rename = "updatedAt", with = "palmier_model::serde_date::iso8601")]
    updated_at: OffsetDateTime,
}

fn load_fixture() -> Value {
    // Test runs with CWD = crate dir (crates/palmier-model); the fixture lives at
    // the repo root under spikes/. Walk up two levels.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spikes/s1b-convex-date/sample-payload.json"
    );
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("could not read S-1b fixture at {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture is valid JSON")
}

#[test]
fn manifest_entry_date_semantic_round_trip() {
    let fixture = load_fixture();
    let entry_json = &fixture["manifest"]["entries"][0];

    let a: ManifestEntryMirror =
        serde_json::from_value(entry_json.clone()).expect("decode manifest entry");
    // Decoded Date fields match the fixture's NUMBERS (the apple-ref doubles).
    // 757_382_400.0 → 2025-01-01Z (the spike's correct 2025 vector).
    assert_eq!(
        a.cached_remote_url_expires_at,
        Some(datetime!(2025-01-01 00:00:00 UTC)),
        "cachedRemoteURLExpiresAt (757382400.0) decodes to 2025-01-01Z"
    );
    // The fixture's createdAt is 725_846_400.0. That number is the spike's
    // arithmetically-wrong "2024-01-01" vector (off by +86_400 s); it actually
    // represents 2024-01-02Z. We assert what the NUMBER decodes to so the gate is
    // honest about the fixture; the semantic round-trip below is unaffected. The
    // fixture/FINDINGS number should be corrected to 725_760_000.0 (out of this
    // story's crates/palmier-model scope — see the result summary).
    assert_eq!(
        a.generation_input.as_ref().unwrap().created_at,
        Some(datetime!(2024-01-02 00:00:00 UTC)),
        "createdAt (725846400.0) decodes to 2024-01-02Z (fixture's 2024 vector is off by one day)"
    );

    // SEMANTIC round-trip: encode → decode → equal.
    let encoded = serde_json::to_string(&a).unwrap();
    let b: ManifestEntryMirror = serde_json::from_str(&encoded).unwrap();
    assert_eq!(a, b, "manifest entry must semantically round-trip");

    // Cross-contamination guard: the Date re-encodes as a JSON NUMBER, not string.
    let reencoded: Value = serde_json::from_str(&encoded).unwrap();
    assert!(
        reencoded["cachedRemoteURLExpiresAt"].is_number(),
        "apple-epoch Date must serialize as a JSON number"
    );
}

#[test]
fn log_entry_date_semantic_round_trip() {
    let fixture = load_fixture();
    let entry_json = &fixture["generationLog"]["entries"][0];

    let a: LogEntryMirror = serde_json::from_value(entry_json.clone()).expect("decode log entry");
    // Same fixture number (725846400.0) → 2024-01-02Z (see manifest test note).
    assert_eq!(
        a.created_at,
        Some(datetime!(2024-01-02 00:00:00 UTC)),
        "log createdAt (725846400.0) decodes to 2024-01-02Z (fixture vector off by one day)"
    );

    let encoded = serde_json::to_string(&a).unwrap();
    let b: LogEntryMirror = serde_json::from_str(&encoded).unwrap();
    assert_eq!(a, b);

    let reencoded: Value = serde_json::from_str(&encoded).unwrap();
    assert!(reencoded["createdAt"].is_number());
}

#[test]
fn chat_session_iso8601_round_trip() {
    let fixture = load_fixture();
    // The fixture stores the chat blob example under this synthetic key
    // (note the `.json` suffix on the key).
    let chat_json = &fixture["_chat_file_example_11111111-1111-1111-1111-111111111111.json"];

    let a: ChatSessionMirror =
        serde_json::from_value(chat_json.clone()).expect("decode chat session");
    assert_eq!(a.updated_at, datetime!(2024-01-01 00:00:00 UTC));

    let encoded = serde_json::to_string(&a).unwrap();
    let b: ChatSessionMirror = serde_json::from_str(&encoded).unwrap();
    assert_eq!(a, b);

    // chat Date re-encodes as a STRING (not a number) — the inverse guard.
    let reencoded: Value = serde_json::from_str(&encoded).unwrap();
    assert!(
        reencoded["updatedAt"].is_string(),
        "iso8601 chat Date must serialize as a JSON string"
    );
    assert_eq!(reencoded["updatedAt"], "2024-01-01T00:00:00Z");
}

#[test]
fn lenient_absent_date_is_none() {
    // Date field absent → None (matches Swift decodeIfPresent / FINDINGS §4.5).
    let json = r#"{"id":"x","name":"y"}"#;
    let entry: ManifestEntryMirror = serde_json::from_str(json).unwrap();
    assert_eq!(entry.cached_remote_url_expires_at, None);
    assert!(entry.generation_input.is_none());
}
