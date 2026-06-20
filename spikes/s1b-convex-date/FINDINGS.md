# Spike S-1b — Convex sample-payload Date encoding

**Status:** RESOLVED **from reference code** (live payload NOT reachable — see section 2).
**Branch:** `spike/S-1b-convex-date`
**Gates:** E2-S8 (`palmier-model` serde Date codec), the whole `palmier-project` write path. PRD section 11 / risk **R-6**.
**Date:** 2026-06-20

---

## TL;DR

| Bundle file | Date field(s) | Wire format | serde representation (E2-S8) |
|---|---|---|---|
| `project.json` (Timeline) | none (Timeline carries no Date) | n/a | n/a |
| `media.json` (MediaManifest) | `entries[].cachedRemoteURLExpiresAt`, `entries[].generationInput.createdAt` | Apple reference-epoch Double (seconds since 2001-01-01T00:00:00Z), JSON number, may be negative/fractional | custom module `apple_ref_epoch` (Option via `apple_ref_epoch::option`) |
| `generation-log.json` (GenerationLog) | `entries[].createdAt` | Apple reference-epoch Double | same `apple_ref_epoch::option` module |
| `chat/<uuid>.json` (ChatSession) | `updatedAt` (+ any Date in AgentMessage) | ISO-8601 string (`2024-01-01T00:00:00Z`); file is pretty-printed + sorted-keys | time/chrono ISO-8601 serde |

A single serde Date format WILL corrupt round-trips. project/media/log are Apple-epoch doubles; chat is ISO-8601. Match per-field (codec is uniform within a file).

---

## 1. Reference per-file Date strategy (exact, from Swift)

All paths under `E:\projects\palmier-pro\Sources\PalmierPro\`.

### project.json / media.json / generation-log.json -> bare JSONEncoder() / JSONDecoder()
`Project/VideoProject.swift`:
- `captureSaveSnapshot()` (lines 90-93) encodes all three with a bare `JSONEncoder()` (no dateEncodingStrategy):
  - `snapshotTimeline      = try? JSONEncoder().encode(editorViewModel.timeline)`       // project.json
  - `snapshotManifest      = try? JSONEncoder().encode(editorViewModel.mediaManifest)`  // media.json
  - `snapshotGenerationLog = try? JSONEncoder().encode(editorViewModel.generationLog)`  // generation-log.json
- `read(from:)` (lines 38, 45, 52) decodes all three with a bare `JSONDecoder()` (no dateDecodingStrategy).
- `Export/PalmierProjectExporter.swift:72-75` likewise re-encodes timeline/manifest/log with a bare `JSONEncoder()`.

A bare encoder/decoder uses `.deferredToDate` (the documented Swift default). `.deferredToDate` encodes a Date as a JSON number = `Date.timeIntervalSinceReferenceDate` = a Double of seconds since the Apple reference epoch 2001-01-01T00:00:00Z (UTC). NOT Unix epoch, NOT milliseconds, NOT ISO-8601.

Conversion: unix_seconds = apple_ref_seconds + 978307200 (seconds 1970-01-01 -> 2001-01-01).
Worked example: 2024-01-01T00:00:00Z -> unix 1704067200 -> apple-ref 725846400.0. (Negative for pre-2001; typically fractional.)

### chat/<uuid>.json -> .iso8601 + pretty + sorted-keys
`Agent/ChatSessionStore.swift:33-44`:
- `e.outputFormatting = [.prettyPrinted, .sortedKeys]`
- `e.dateEncodingStrategy = .iso8601` (encoder) / `d.dateDecodingStrategy = .iso8601` (decoder)

`ChatSession.updatedAt` (and any Date reachable through AgentMessage) serialize as ISO-8601 strings. `.iso8601` = ISO8601DateFormatter() defaults = [.withInternetDateTime] -> yyyy-MM-ddTHH:mm:ssZ (UTC, no fractional seconds).

### Date-bearing fields, enumerated
| Type | Field | File | Source |
|---|---|---|---|
| MediaManifestEntry | cachedRemoteURLExpiresAt: Date? | media.json | Models/MediaManifest.swift:33 |
| GenerationInput | createdAt: Date? | media.json (nested in entry) | Models/MediaManifest.swift:62 |
| GenerationLogEntry | createdAt: Date? | generation-log.json | Editor/ViewModel/EditorViewModel+Cost.swift:14 |
| ChatSession | updatedAt: Date | chat/*.json | Agent/ChatSessionStore.swift:6 |
| Timeline | none | project.json | Models/Timeline.swift (grep Date -> 0 hits) |
| ProjectEntry | createdDate, lastOpenedDate | registry (project-registry.json, NOT a bundle file) | Project/ProjectRegistry.swift:6-7 (bare encoder => Apple-epoch doubles too, but out of E2-S8 bundle scope) |

All bundle Date fields are Optional. media/log use decodeIfPresent with custom init(from:) (lenient — missing => nil); chat updatedAt is required (decode).

---

## 2. Live payload attempt — NOT reachable (fell back to reference code)

Result: could not obtain a real /v1/samples payload. Confirmed from reference code instead.

What I tried and why it failed:
- Deployment URL is a build-time secret, not committed. `Account/BackendConfig.swift` reads PalmierConvexHttpURL / PalmierConvexDeploymentURL from the app Info.plist; `scripts/bundle.sh:75-76` injects them from env vars CONVEX_DEPLOYMENT_URL / CONVEX_HTTP_URL at build time. A repo-wide grep across BOTH palmier-pro and palmier-pro-win for *.convex.cloud / *.convex.site / the env-var names found no committed value (only the injection sites). No .env/.xcconfig committed.
- Public discovery failed. palmier.io -> 307 redirect, no Convex reference in body; app.palmier.io -> 404; api.palmier.io / convex.palmier.io /v1/samples -> no route (000/unresolved). No public, unauthenticated way to derive the deployment from this box.

Why the reference-code answer is authoritative (not a guess):
`Project/SampleProjectService.swift:65-104` does NOT re-encode dates. It parses the resolve response with JSONSerialization, pulls the project / manifest / generationLog sub-objects, and writes them BYTE-FOR-BYTE to project.json / media.json / generation-log.json (JSONSerialization.data(withJSONObject:) -> .write(to:)). Those exact bytes are then read back by VideoProject.read with the bare .deferredToDate JSONDecoder (manifest decode failure = HARD .fileReadCorruptFile).

=> For the shipping reference app to open ANY materialized sample, the server is FORCED to emit cachedRemoteURLExpiresAt / generationInput.createdAt / log createdAt as Apple reference-epoch doubles. Any other Date shape (ISO-8601 string, Unix ms) makes the bare .deferredToDate decoder throw -> sample un-openable. The server wire format is pinned by the client decoder, and we know the client decoder exactly. Chat files are downloaded as opaque blobs and decoded by the .iso8601 ChatSessionStore decoder, pinning chat to ISO-8601 the same way.

Strongest confirmation short of a live capture; depends only on committed client code, not on guessing server behavior.

sample-payload.json in this folder is SYNTHETIC (labeled), built to match this format, as the round-trip fixture until a real capture is taken. R-6 carry-forward: when a deployment URL becomes available (latest: alongside Spike S-2, before Epic 9), capture a real /v1/samples/resolve payload, diff its Date fields against this fixture, apply section 5 fallback if divergent.

---

## 3. Recommended serde implementation for E2-S8 (palmier-model)

serde with a custom module per Date strategy. Date lib: time (or chrono). Apple-epoch side is just an f64 transform.

### 3a. Apple reference-epoch module (media.json + generation-log.json)

    // crates/palmier-model/src/serde_date.rs
    pub mod apple_ref_epoch {
        use serde::{Deserialize, Deserializer, Serializer};
        use time::OffsetDateTime;

        /// Seconds between Unix epoch (1970-01-01) and Apple reference epoch (2001-01-01).
        const APPLE_EPOCH_OFFSET: f64 = 978_307_200.0;

        pub fn to_apple_secs(dt: OffsetDateTime) -> f64 {
            (dt.unix_timestamp_nanos() as f64 / 1_000_000_000.0) - APPLE_EPOCH_OFFSET
        }
        pub fn from_apple_secs(secs: f64) -> OffsetDateTime {
            let unix = secs + APPLE_EPOCH_OFFSET;
            OffsetDateTime::from_unix_timestamp_nanos((unix * 1_000_000_000.0).round() as i128).unwrap()
        }

        /// Option<OffsetDateTime> <-> JSON number | null  (ALL bundle Date fields are Optional)
        pub mod option {
            use super::*;
            pub fn serialize<S: Serializer>(v: &Option<OffsetDateTime>, s: S) -> Result<S::Ok, S::Error> {
                match v { Some(dt) => s.serialize_f64(to_apple_secs(*dt)), None => s.serialize_none() }
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<OffsetDateTime>, D::Error> {
                Ok(Option::<f64>::deserialize(d)?.map(from_apple_secs))
            }
        }
    }

Apply #[serde(with = "crate::serde_date::apple_ref_epoch::option", default, skip_serializing_if = "Option::is_none")] on:
- MediaManifestEntry::cached_remote_url_expires_at
- GenerationInput::created_at
- GenerationLogEntry::created_at

Byte-identity note: Swift .deferredToDate writes the number with JSONEncoder's Double formatting; serde_json uses Ryu — not guaranteed character-identical for every value (trailing .0, exponent form, last-digit rounding). SM-7 byte-identical applies to OUR OWN write->read->write round-trip (Ryu is stable) and to re-emitting a CAPTURED payload. If a real capture's number formatting differs, that is formatting not semantics — see R-6.4. Prefer SEMANTIC round-trip (decode A, encode, decode B, A == B) over raw-byte equality for the Apple-epoch numbers.

### 3b. ISO-8601 module (chat/*.json) — adjacent (ChatSession also lives in palmier-model)

    pub mod iso8601 {
        use serde::{Deserialize, Deserializer, Serializer};
        use time::{OffsetDateTime, format_description::well_known::Iso8601};
        pub fn serialize<S: Serializer>(dt: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
            // Swift .iso8601 = [.withInternetDateTime] => "yyyy-MM-ddTHH:mm:ssZ", NO fractional seconds, UTC.
            let fmt = time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]Z").unwrap();
            s.serialize_str(&dt.to_offset(time::UtcOffset::UTC).format(&fmt).unwrap())
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
            let s = String::deserialize(d)?;
            OffsetDateTime::parse(&s, &Iso8601::DEFAULT).map_err(serde::de::Error::custom)
        }
    }

Apply (required, non-Optional) on ChatSession::updated_at. Chat files also write pretty (2-space) + sorted-keys to match [.prettyPrinted, .sortedKeys] (BTreeMap / canonicalizing serializer). Chat byte-fidelity is an Epic-7/8 concern; flag there.

Critical: never apply ISO-8601 semantics to media/log or Apple-epoch to chat. There is no global serde_json date setting, so per-field is natural — the hazard only appears if someone writes one shared helper.

---

## 4. Round-trip test to add (E2-S8 regression gate, R-6)

Add to crates/palmier-model (e.g. tests/date_roundtrip.rs):

1. Apple-epoch SEMANTIC round-trip (MediaManifest, GenerationLog): load manifest/generationLog from spikes/s1b-convex-date/sample-payload.json (or copy into tests/fixtures/); decode -> encode -> decode; assert the two decoded structs are ==. Assert cachedRemoteURLExpiresAt serializes as a JSON NUMBER and from_apple_secs(n) reconstructs the expected instant.
2. Known-vector (guards the 978307200 offset and epoch direction): to_apple_secs(2024-01-01T00:00:00Z) == 725_846_400.0 and the inverse. Catches Unix-vs-Apple epoch and sign mistakes — the exact R-6 corruption.
3. ISO-8601 round-trip (ChatSession): decode an ISO-8601 fixture, re-encode, assert the STRING is byte-identical ("2024-01-01T00:00:00Z", no fractional seconds, trailing Z).
4. Cross-contamination guard: media/log Date serializes as a JSON NUMBER (not string); chat Date as a STRING (not number).
5. Lenient-decode parity: Date field absent -> None (matches decodeIfPresent); malformed MANIFEST = hard error, malformed LOG = tolerated (mirror project-io.md read severities).

When a real payload lands (S-2 window): add it as a fixture, re-run 1/3/4. Green = R-6 retired.

---

## 5. Fallback (R-6 named switch)

Codec is isolated to two modules + per-field #[serde(with = ...)], so divergence is a localized swap:

- R-6.1 — media/log emit ISO-8601 strings instead of Apple doubles. Swap the three media/log fields to an iso8601::option variant. (Unlikely: breaks the reference app's .deferredToDate decoder — but a future Rust-only server could.)
- R-6.2 — Unix epoch (s or ms) instead of Apple epoch. Set APPLE_EPOCH_OFFSET = 0.0 (seconds) or add /1000 (ms). Known-vector test (4.2) catches it instantly.
- R-6.3 — Convex $date / wrapped object ({"$date": ...}). Replace the field module with a wrapper (de)serializer. (Convex JS value encoding can wrap dates; reference dodges it by byte-passing server JSON, but a typed Rust Convex client might hit it.)
- R-6.4 — number formatting differs from serde_json Ryu (e.g. Swift emits 7.258464e8 / extra precision). Keep the semantic codec; if true raw-byte re-emission of a captured payload is needed, add a Swift-mirroring f64->string formatter, OR (preferred) relax the gate to SEMANTIC equality for these numbers — only imported sample numbers must decode correctly, which semantic equality guarantees.

Each fallback = one-module / one-constant change behind the same attributes; section 4 tests are the trip-wire.

---

## 6. What E2-S8 must implement (handoff)

1. crates/palmier-model/src/serde_date.rs with TWO modules: apple_ref_epoch (+ ::option) and iso8601. Offset constant 978_307_200.
2. Field attributes:
   - MediaManifestEntry::cached_remote_url_expires_at -> apple_ref_epoch::option
   - GenerationInput::created_at -> apple_ref_epoch::option
   - GenerationLogEntry::created_at -> apple_ref_epoch::option
   - ChatSession::updated_at -> iso8601
   - All four default; media/log Optionals skip_serializing_if = "Option::is_none" (matches decodeIfPresent omission).
3. Bundle writers: media.json / generation-log.json = COMPACT serde_json (no pretty, no sorted keys — bare-encoder parity). chat/*.json = PRETTY (2-space) + sorted keys.
4. The section 4 round-trip test module, wired as a crate test (M1 regression gate, R-6 / PRD section 11.5).
5. Carry the R-6 note forward: serde Dates are "provisional-confirmed-from-code" until a real /v1/samples/resolve payload is captured (S-2 window) and diffed against sample-payload.json.

Date-lib choice open (time vs chrono); pick whatever palmier-model already pulls in. Examples use time::OffsetDateTime.
