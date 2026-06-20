//! Per-field `Date` codec seam (story E2-S8) — Apple reference-epoch doubles
//! (project/media/log) + ISO-8601 strings (chat).
//!
//! ## PROVISIONAL until Spike S-1b confirms the live `/v1/samples` payload
//!
//! S-1b resolved the wire format **from reference Swift code** (the live Convex
//! deployment URL was a build-time secret and unreachable). See
//! `spikes/s1b-convex-date/FINDINGS.md`. The reference client decodes
//! `project.json` / `media.json` / `generation-log.json` with a bare
//! `JSONDecoder()` (Swift's `.deferredToDate`), which encodes a `Date` as a JSON
//! **number** = seconds since the **Apple reference epoch 2001-01-01T00:00:00Z**
//! — pinning the server wire format. `chat/*.json` is decoded with
//! `.iso8601`, pinning chat to RFC3339 strings.
//!
//! A single global Date format would corrupt round-trips, so the codec is
//! **per-field** (`#[serde(with = …)]`), letting a single field's encoder flip
//! in isolation (the R-6 fallback seam):
//!
//! - [`apple_ref_epoch`] (+ [`apple_ref_epoch::option`]) — `DateTime` ↔ f64
//!   seconds-since-2001. Applied to `MediaManifestEntry::cached_remote_url_expires_at`,
//!   `GenerationInput::created_at`, `GenerationLogEntry::created_at` (all
//!   Optional, `skip_serializing_if = "Option::is_none"`). Those shapes land in
//!   E2-S7; this story owns the codec modules they reference.
//! - [`iso8601`] — `DateTime` ↔ RFC3339 (`[year]-[month]-[day]T…Z`, **no
//!   fractional seconds**, UTC). Applied to `ChatSession::updated_at` (E2-S7/S8
//!   chat shape).
//!
//! Date library: `time` (`OffsetDateTime`). The Apple-epoch side is a pure f64
//! transform around the offset constant; ISO-8601 uses `time`'s well-known
//! formats.
//!
//! ## R-6 round-trip regression gate
//!
//! `tests/date_roundtrip.rs` decodes the committed
//! `spikes/s1b-convex-date/sample-payload.json` Date fields, re-encodes, and
//! asserts **semantic** equality (per FINDINGS §3a: serde_json's Ryu formatting
//! is not guaranteed character-identical to Swift's `JSONEncoder` for every f64,
//! so the gate is semantic — decode A, encode, decode B, `A == B` — plus a
//! known-vector test pinning the 978_307_200 offset and epoch direction). When a
//! real `/v1/samples` payload lands (S-2 window), add it as a fixture and re-run.

use time::OffsetDateTime;

/// Apple reference-epoch codec: `OffsetDateTime` ↔ **f64 seconds since
/// 2001-01-01T00:00:00Z** (Swift `JSONEncoder` `.deferredToDate`).
///
/// Apply via `#[serde(with = "crate::serde_date::apple_ref_epoch")]` on a bare
/// `OffsetDateTime`, or `apple_ref_epoch::option` on an `Option<OffsetDateTime>`.
pub mod apple_ref_epoch {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Seconds between the Unix epoch (1970-01-01) and the Apple reference epoch
    /// (2001-01-01). `unix_seconds = apple_ref_seconds + APPLE_EPOCH_OFFSET`.
    ///
    /// R-6.2 fallback seam: set to `0.0` for Unix-epoch seconds (the
    /// known-vector test trips instantly if this is wrong).
    pub const APPLE_EPOCH_OFFSET: f64 = 978_307_200.0;

    /// `OffsetDateTime` → Apple reference-epoch seconds (f64).
    pub fn to_apple_secs(dt: OffsetDateTime) -> f64 {
        (dt.unix_timestamp_nanos() as f64 / 1_000_000_000.0) - APPLE_EPOCH_OFFSET
    }

    /// Apple reference-epoch seconds (f64) → `OffsetDateTime` (UTC).
    pub fn from_apple_secs(secs: f64) -> OffsetDateTime {
        let unix = secs + APPLE_EPOCH_OFFSET;
        OffsetDateTime::from_unix_timestamp_nanos((unix * 1_000_000_000.0).round() as i128)
            .expect("apple-epoch seconds out of representable range")
    }

    /// `#[serde(with = "...apple_ref_epoch")]` for a required `OffsetDateTime`.
    pub fn serialize<S: Serializer>(dt: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(to_apple_secs(*dt))
    }

    /// Deserialize a JSON number → `OffsetDateTime`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
        Ok(from_apple_secs(f64::deserialize(d)?))
    }

    /// `Option<OffsetDateTime>` ↔ JSON number | null. ALL bundle Date fields are
    /// Optional (`decodeIfPresent`), so pair this with `default,
    /// skip_serializing_if = "Option::is_none"` at the field.
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            v: &Option<OffsetDateTime>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            match v {
                Some(dt) => s.serialize_f64(to_apple_secs(*dt)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<OffsetDateTime>, D::Error> {
            Ok(Option::<f64>::deserialize(d)?.map(from_apple_secs))
        }
    }
}

/// ISO-8601 codec for chat Dates: `OffsetDateTime` ↔ RFC3339 string
/// `[year]-[month]-[day]T[hour]:[minute]:[second]Z` (UTC, **no fractional
/// seconds**) — matching Swift `.iso8601` = `[.withInternetDateTime]`.
///
/// Apply via `#[serde(with = "crate::serde_date::iso8601")]` on a required
/// `OffsetDateTime` (e.g. `ChatSession::updated_at`).
pub mod iso8601 {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};
    use time::format_description::well_known::Iso8601;
    use time::macros::format_description;

    /// Fixed output format: no fractional seconds, trailing `Z`, UTC. Matches
    /// Swift `ISO8601DateFormatter()` defaults.
    const OUT_FMT: &[time::format_description::FormatItem<'static>] = format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
    );

    pub fn serialize<S: Serializer>(dt: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
        let utc = dt.to_offset(time::UtcOffset::UTC);
        let formatted = utc
            .format(&OUT_FMT)
            .map_err(serde::ser::Error::custom)?;
        s.serialize_str(&formatted)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
        let s = String::deserialize(d)?;
        // Iso8601::DEFAULT parses the internet-date-time variants (with or
        // without fractional seconds), tolerating real-world inputs.
        OffsetDateTime::parse(&s, &Iso8601::DEFAULT).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use time::macros::datetime;

    /// Known vector (guards the 978_307_200 offset + epoch direction — the exact
    /// R-6 corruption).
    ///
    /// 2024-01-01T00:00:00Z → unix 1_704_067_200 → apple-ref
    /// `1_704_067_200 − 978_307_200 = 725_760_000.0`.
    ///
    /// NOTE: `spikes/s1b-convex-date/FINDINGS.md` and `sample-payload.json` state
    /// this vector as `725_846_400.0`, which is **arithmetically wrong by one day
    /// (+86_400 s)** — that number decodes to 2024-01-**02**Z. The *offset*
    /// 978_307_200 the spike specifies is correct; only its worked 2024 example is
    /// off. The codec uses the correct math (which the inline arithmetic above and
    /// the Python cross-check confirm); see the result summary's "check before
    /// merge" note to correct the fixture's `createdAt` (out of this story's
    /// `crates/palmier-model` scope). The spike's 2025 vector
    /// (`757_382_400.0`) IS correct.
    #[test]
    fn apple_epoch_known_vector() {
        let dt = datetime!(2024-01-01 00:00:00 UTC);
        assert_eq!(apple_ref_epoch::to_apple_secs(dt), 725_760_000.0);
        // Inverse reconstructs the same instant.
        let back = apple_ref_epoch::from_apple_secs(725_760_000.0);
        assert_eq!(back, dt);

        // 2001-01-01 itself is exactly 0.0 seconds.
        let epoch = datetime!(2001-01-01 00:00:00 UTC);
        assert_eq!(apple_ref_epoch::to_apple_secs(epoch), 0.0);

        // 2025-01-01 → 757_382_400.0 (the spike's 2025 vector, which is correct).
        let y2025 = datetime!(2025-01-01 00:00:00 UTC);
        assert_eq!(apple_ref_epoch::to_apple_secs(y2025), 757_382_400.0);
    }

    #[test]
    fn apple_epoch_pre_2001_is_negative() {
        let pre = datetime!(2000-01-01 00:00:00 UTC);
        assert!(apple_ref_epoch::to_apple_secs(pre) < 0.0);
        // Round-trip.
        let secs = apple_ref_epoch::to_apple_secs(pre);
        assert_eq!(apple_ref_epoch::from_apple_secs(secs), pre);
    }

    #[test]
    fn iso8601_round_trips_no_fractional_seconds() {
        let dt = datetime!(2024-01-01 00:00:00 UTC);

        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrap {
            #[serde(with = "crate::serde_date::iso8601")]
            updated_at: OffsetDateTime,
        }

        let w = Wrap { updated_at: dt };
        let json = serde_json::to_string(&w).unwrap();
        // Exact RFC3339, no fractional seconds, trailing Z.
        assert_eq!(json, r#"{"updated_at":"2024-01-01T00:00:00Z"}"#);
        let back: Wrap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, w);
    }

    /// A struct using BOTH codecs on two fields encodes each in the correct
    /// format (number vs string) — the cross-contamination guard.
    #[test]
    fn mixed_codec_struct_encodes_each_field_correctly() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Mixed {
            #[serde(
                with = "crate::serde_date::apple_ref_epoch::option",
                default,
                skip_serializing_if = "Option::is_none"
            )]
            expires_at: Option<OffsetDateTime>,
            #[serde(with = "crate::serde_date::iso8601")]
            updated_at: OffsetDateTime,
        }

        let m = Mixed {
            expires_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
            updated_at: datetime!(2024-01-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&m).unwrap();
        // media/log Date → NUMBER; chat Date → STRING.
        assert!(
            json.contains("\"expires_at\":757382400.0"),
            "apple-epoch field must be a JSON number: {json}"
        );
        assert!(
            json.contains("\"updated_at\":\"2024-01-01T00:00:00Z\""),
            "chat field must be an ISO-8601 string: {json}"
        );
        let back: Mixed = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn apple_epoch_option_absent_is_none() {
        #[derive(Deserialize, PartialEq, Debug)]
        struct Opt {
            #[serde(
                with = "crate::serde_date::apple_ref_epoch::option",
                default
            )]
            expires_at: Option<OffsetDateTime>,
        }
        // Absent field → None (matches Swift decodeIfPresent).
        let decoded: Opt = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded.expires_at, None);
        // null → None.
        let decoded2: Opt = serde_json::from_str(r#"{"expires_at":null}"#).unwrap();
        assert_eq!(decoded2.expires_at, None);
        // number → Some. 725_760_000.0 is the CORRECT apple-ref value for
        // 2024-01-01Z (1_704_067_200 − 978_307_200).
        let decoded3: Opt = serde_json::from_str(r#"{"expires_at":725760000.0}"#).unwrap();
        assert_eq!(
            decoded3.expires_at,
            Some(datetime!(2024-01-01 00:00:00 UTC))
        );
    }
}
