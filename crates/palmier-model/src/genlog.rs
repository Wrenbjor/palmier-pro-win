//! `GenerationLog` / `GenerationLogEntry` — the append-only `generation-log.json`
//! serde shapes (story E2-S7).
//!
//! Ported 1:1 from the macOS reference
//! `Sources/PalmierPro/Editor/ViewModel/EditorViewModel+Cost.swift`
//! (`struct GenerationLog`, `struct GenerationLogEntry`). See
//! docs/reference/project-io.md Port risks "Lenient decode is load-bearing".
//!
//! ## Legacy `cost` (dollars) → `cost_credits = ceil(dollars * 100)`
//!
//! The reference `GenerationLogEntry.init(from:)` first tries `costCredits` (Int);
//! if absent it reads a legacy `cost` (dollars, Double) and converts via
//! `Int((dollars * 100).rounded(.up))` — i.e. **ceil**. Old projects predating the
//! credit system stored dollars; new ones store credits. serde's derive can't
//! express "field A, else field-B-with-transform", so we hand-write `Deserialize`
//! to replicate the fallback exactly. (Serialize stays derived — we only ever write
//! `costCredits`.)

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

fn default_version() -> u32 {
    // Reference `var version: Int = 1`.
    1
}

fn new_uuid_string() -> String {
    Uuid::new_v4().to_string()
}

/// `generation-log.json`: the append-only record of every AI generation in the
/// project (reference `GenerationLog`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationLog {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<GenerationLogEntry>,
}

impl GenerationLog {
    pub fn new() -> Self {
        GenerationLog {
            version: default_version(),
            entries: Vec::new(),
        }
    }
}

impl Default for GenerationLog {
    fn default() -> Self {
        GenerationLog::new()
    }
}

/// One row in the project activity log (reference `GenerationLogEntry`).
///
/// `Serialize` is derived (always writes `costCredits` + the apple-epoch
/// `createdAt`); `Deserialize` is hand-written to honor the legacy `cost`-dollars
/// fallback.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenerationLogEntry {
    /// UUID **string** (regenerated if absent on decode).
    #[serde(default = "new_uuid_string")]
    pub id: String,
    pub model: String,
    /// Cost in credits. `None` when neither `costCredits` nor a legacy `cost` is
    /// present.
    #[serde(rename = "costCredits", skip_serializing_if = "Option::is_none")]
    pub cost_credits: Option<i64>,
    /// Apple reference-epoch double (E2-S8 codec seam).
    #[serde(
        rename = "createdAt",
        with = "crate::serde_date::apple_ref_epoch::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

impl GenerationLogEntry {
    pub fn new(model: impl Into<String>, cost_credits: Option<i64>) -> Self {
        GenerationLogEntry {
            id: new_uuid_string(),
            model: model.into(),
            cost_credits,
            created_at: None,
        }
    }
}

impl<'de> Deserialize<'de> for GenerationLogEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Mirror of the reference custom `init(from:)`: decode `id` (regenerate if
        // absent), `model` (required), the apple-epoch `createdAt`, then
        // `costCredits` with a legacy `cost`-dollars → ceil(dollars*100) fallback.
        //
        // We decode into a raw helper holding every wire key (both the modern
        // `costCredits` and the legacy `cost`), then resolve the fallback.
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default = "new_uuid_string")]
            id: String,
            model: String,
            #[serde(rename = "costCredits", default)]
            cost_credits: Option<i64>,
            /// Legacy field: cost in dollars (Double). Only read when
            /// `costCredits` is absent.
            #[serde(default)]
            cost: Option<f64>,
            #[serde(
                rename = "createdAt",
                with = "crate::serde_date::apple_ref_epoch::option",
                default
            )]
            created_at: Option<OffsetDateTime>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let cost_credits = match raw.cost_credits {
            Some(c) => Some(c),
            // Legacy `cost` dollars → ceil(dollars * 100) (reference
            // `Int((dollars * 100).rounded(.up))`). `f64::ceil` is round-toward-
            // +infinity, matching Swift `.rounded(.up)`.
            None => raw.cost.map(|dollars| (dollars * 100.0).ceil() as i64),
        };
        Ok(GenerationLogEntry {
            id: raw.id,
            model: raw.model,
            cost_credits,
            created_at: raw.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn log_missing_version_decodes_to_1() {
        let log: GenerationLog = serde_json::from_str("{}").unwrap();
        assert_eq!(log.version, 1);
        assert!(log.entries.is_empty());
    }

    #[test]
    fn entry_modern_cost_credits_round_trips() {
        let json = r#"{"id":"e1","model":"veo-3","costCredits":250}"#;
        let e: GenerationLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.cost_credits, Some(250));
        assert_eq!(e.model, "veo-3");
        // Re-encode emits costCredits.
        let out = serde_json::to_string(&e).unwrap();
        assert!(out.contains("\"costCredits\":250"), "{out}");
    }

    #[test]
    fn entry_legacy_cost_dollars_converts_to_ceil_credits() {
        // Legacy dollars → ceil(dollars * 100). 1.234 → 123.4 → ceil → 124.
        let json = r#"{"id":"e1","model":"veo-3","cost":1.234}"#;
        let e: GenerationLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.cost_credits, Some(124));

        // Exact boundary: 2.50 → 250.0 → ceil → 250 (no rounding up needed).
        let json2 = r#"{"model":"m","cost":2.50}"#;
        let e2: GenerationLogEntry = serde_json::from_str(json2).unwrap();
        assert_eq!(e2.cost_credits, Some(250));

        // Just over a boundary: 0.001 → 0.1 → ceil → 1.
        let json3 = r#"{"model":"m","cost":0.001}"#;
        let e3: GenerationLogEntry = serde_json::from_str(json3).unwrap();
        assert_eq!(e3.cost_credits, Some(1));
    }

    #[test]
    fn entry_costcredits_takes_precedence_over_legacy_cost() {
        // If both present, modern costCredits wins (reference checks it first).
        let json = r#"{"model":"m","costCredits":7,"cost":99.0}"#;
        let e: GenerationLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.cost_credits, Some(7));
    }

    #[test]
    fn entry_neither_cost_field_is_none() {
        let json = r#"{"model":"m"}"#;
        let e: GenerationLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.cost_credits, None);
        // id regenerated as a UUID string.
        assert!(Uuid::parse_str(&e.id).is_ok());
        // Serialize omits costCredits (skip_serializing_if None) and createdAt.
        let out = serde_json::to_string(&e).unwrap();
        assert!(!out.contains("costCredits"), "{out}");
        assert!(!out.contains("createdAt"), "{out}");
    }

    #[test]
    fn entry_created_at_uses_apple_epoch_number() {
        let mut e = GenerationLogEntry::new("m", Some(10));
        e.id = "e1".into();
        e.created_at = Some(datetime!(2025-01-01 00:00:00 UTC));
        let out = serde_json::to_string(&e).unwrap();
        // createdAt encodes as a NUMBER (apple-epoch seconds), not iso8601.
        // 2025-01-01Z → 757_382_400.0 (see serde_date known vector).
        assert!(out.contains("\"createdAt\":757382400.0"), "{out}");
        let back: GenerationLogEntry = serde_json::from_str(&out).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn log_round_trips_with_entries() {
        let mut log = GenerationLog::new();
        log.entries.push(GenerationLogEntry {
            id: "e1".into(),
            model: "veo-3".into(),
            cost_credits: Some(120),
            created_at: None,
        });
        let json = serde_json::to_string(&log).unwrap();
        let back: GenerationLog = serde_json::from_str(&json).unwrap();
        assert_eq!(log, back);
    }
}
