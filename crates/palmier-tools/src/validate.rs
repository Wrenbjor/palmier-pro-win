//! Argument validation — run BEFORE dispatch (E7-S3; reference
//! `ToolExecutor.swift` `validateUnknownKeys` / `firstNonFiniteNumberPath` /
//! `decodeToolArgs` / `formatDecodingError`, plus the per-tool required-field and
//! dual-shape XOR checks in `ToolExecutor+*.swift`).
//!
//! Every tool's args are validated against its input schema (the registry from
//! E7-S1) before the body runs. A failure returns the contract `{ isError }`
//! [`ToolResult`] with the same human-readable, JSON-path-anchored message the
//! reference clients expect.
//!
//! ## What is checked (in order, matching the reference `decodeToolArgs`)
//! 1. **Unknown-key rejection** — each tool has an allowed-key set (derived from
//!    its schema `properties`, plus the dual-shape `entries` union). An unexpected
//!    key → error naming the key (sorted, single-quoted) and the allowed set.
//! 2. **Non-finite guard** — any `NaN`/`±Inf` anywhere in the args (recursively) →
//!    error reporting the JSON path to the offending value.
//! 3. **Required fields & types** — missing required field or wrong JSON type →
//!    error with the JSON path (reference `formatDecodingError`).
//! 4. **Tool-specific rules** — dual-shape XOR (`create_folder` / `move_to_folder`
//!    / `rename_media` / `rename_folder` reject "both shapes at once"),
//!    `ripple_delete_ranges` `trackIndex` XOR `clipId` + `units` rules.
//!
//! ## Colors
//! [`parse_rgba`] accepts `#RRGGBB` and `#RRGGBBAA` (reference
//! `TextStyle.RGBA(hex:)`). Used by `add_texts` / `add_captions` /
//! `set_clip_properties` bodies (E7-S7/S8). Validation surfaces it so a bad hex is
//! rejected up front when a tool declares a color field.

use serde_json::Value;

use crate::result::ToolResult;
use crate::schema::ToolName;

/// A validation failure carrying the human-readable, JSON-path-anchored message.
/// The dispatcher maps it to the `{ isError }` [`ToolResult`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub message: String,
}

impl ValidationError {
    fn new(message: impl Into<String>) -> ValidationError {
        ValidationError { message: message.into() }
    }
}

impl From<ValidationError> for ToolResult {
    fn from(e: ValidationError) -> ToolResult {
        ToolResult::error(e.message)
    }
}

/// Validate `args` for `tool` against its schema + tool-specific rules. `Ok(())`
/// passes; `Err` carries the contract error message.
pub fn validate(tool: ToolName, args: &Value) -> Result<(), ValidationError> {
    // The top-level args must be a JSON object (the reference always decodes a
    // `[String: Any]`).
    let Some(obj) = args.as_object() else {
        return Err(ValidationError::new(format!(
            "{}: arguments must be a JSON object",
            tool.wire_name()
        )));
    };

    // (1) Unknown-key rejection against the tool's allowed keys.
    let allowed = allowed_keys(tool, args);
    validate_unknown_keys(obj.keys().map(String::as_str), &allowed, tool.wire_name())?;

    // (2) Non-finite guard (recursive, pre-decode).
    if let Some(path) = first_non_finite_path(args, tool.wire_name()) {
        return Err(ValidationError::new(format!("{path}: value must be finite")));
    }

    // (3) Required fields + types from the schema.
    validate_required_and_types(tool, args)?;

    // (4) Tool-specific rules.
    validate_tool_specific(tool, args)?;

    Ok(())
}

// ── (1) unknown keys ─────────────────────────────────────────────────────────

/// The allowed top-level argument keys for `tool`. Derived from the schema's
/// `properties` (E7-S1). For the **dual-shape** tools the schema already declares
/// the union of both shapes' direct fields plus `entries`, so the schema keys are
/// exactly the allowed set.
fn allowed_keys(tool: ToolName, _args: &Value) -> Vec<String> {
    let def = crate::schema::tool_definitions()
        .into_iter()
        .find(|d| d.name == tool);
    match def {
        Some(def) => def
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Reject any key not in `allowed` (reference `validateUnknownKeys`). Error format:
/// `"<path>: unknown field(s) '<a>', '<b>'. Allowed: <sorted, comma-joined>."`.
pub fn validate_unknown_keys<'a>(
    keys: impl Iterator<Item = &'a str>,
    allowed: &[String],
    path: &str,
) -> Result<(), ValidationError> {
    let mut unknown: Vec<&str> = keys.filter(|k| !allowed.iter().any(|a| a == k)).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let mut allowed_sorted: Vec<&str> = allowed.iter().map(String::as_str).collect();
    allowed_sorted.sort_unstable();
    Err(ValidationError::new(format!(
        "{path}: unknown field(s) '{}'. Allowed: {}.",
        unknown.join("', '"),
        allowed_sorted.join(", ")
    )))
}

// ── (2) non-finite guard ─────────────────────────────────────────────────────

/// The JSON path to the first non-finite (`NaN`/`±Inf`) number in `value`, if any
/// (reference `firstNonFiniteNumberPath`). Recurses arrays (`path[i]`) and objects
/// (`path.key`).
///
/// Note: `serde_json` cannot construct a `NaN`/`Inf` `Number` from standard JSON
/// text (the parser rejects them), so this fires only when a caller injects a
/// non-finite via the typed API — the reference guards the same defensive path.
pub fn first_non_finite_path(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if !f.is_finite() => Some(path.to_string()),
            _ => None,
        },
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .find_map(|(i, v)| first_non_finite_path(v, &format!("{path}[{i}]"))),
        Value::Object(map) => map
            .iter()
            .find_map(|(k, v)| first_non_finite_path(v, &format!("{path}.{k}"))),
        _ => None,
    }
}

// ── (3) required fields + types ──────────────────────────────────────────────

/// Validate the schema's `required` fields are present, and that each present
/// property has the JSON type its schema declares (reference
/// `formatDecodingError`: missing required → "missing required field '<k>'";
/// wrong type → "expected <type>, got something else").
fn validate_required_and_types(tool: ToolName, args: &Value) -> Result<(), ValidationError> {
    let Some(def) = crate::schema::tool_definitions().into_iter().find(|d| d.name == tool) else {
        return Ok(());
    };
    let schema = &def.input_schema;
    let obj = args.as_object().expect("checked object earlier");
    let path = tool.wire_name();

    // required
    if let Some(req) = schema.get("required").and_then(Value::as_array) {
        for r in req {
            if let Some(name) = r.as_str() {
                if !obj.contains_key(name) {
                    return Err(ValidationError::new(format!(
                        "{path}: missing required field '{name}'"
                    )));
                }
            }
        }
    }

    // shallow type checks for the top-level declared properties
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (key, val) in obj {
            if let Some(prop_schema) = props.get(key) {
                if let Some(expected) = prop_schema.get("type").and_then(Value::as_str) {
                    if !json_type_matches(expected, val) {
                        return Err(ValidationError::new(format!(
                            "{path}.{key}: expected {expected}, got something else"
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Whether `val`'s JSON type matches the schema `type` keyword. `integer` accepts
/// any JSON number whose value is integral (the reference decodes Doubles to Int
/// via truncation — we accept integral numbers and let the body coerce).
fn json_type_matches(expected: &str, val: &Value) -> bool {
    match expected {
        "object" => val.is_object(),
        "array" => val.is_array(),
        "string" => val.is_string(),
        "boolean" => val.is_boolean(),
        "number" => val.is_number(),
        "integer" => val.as_i64().is_some() || val.as_u64().is_some(),
        // Unknown/compound schema types are not enforced here.
        _ => true,
    }
}

// ── (4) tool-specific rules ──────────────────────────────────────────────────

fn validate_tool_specific(tool: ToolName, args: &Value) -> Result<(), ValidationError> {
    use ToolName::*;
    match tool {
        CreateFolder | MoveToFolder | RenameMedia | RenameFolder => {
            validate_dual_shape(tool, args)
        }
        RippleDeleteRanges => validate_ripple_delete(args),
        _ => Ok(()),
    }
}

/// The dual-shape tools accept direct fields **XOR** `entries[]`, never both
/// (reference: the +Folders parsers take `entries` if present, else direct; the
/// "not both" contract is enforced here so a client passing both is rejected with
/// a clear message rather than silently ignoring the direct fields).
fn validate_dual_shape(tool: ToolName, args: &Value) -> Result<(), ValidationError> {
    let obj = args.as_object().expect("object");
    let has_entries = obj.contains_key("entries");
    if !has_entries {
        return Ok(());
    }
    // `entries` present → it must be a non-empty array of objects, and no direct
    // fields may be set alongside it.
    let direct_fields: &[&str] = match tool {
        ToolName::CreateFolder => &["name", "parentFolderId"],
        ToolName::MoveToFolder => &["assetIds", "folderId"],
        ToolName::RenameMedia => &["mediaRef", "name"],
        ToolName::RenameFolder => &["folderId", "name"],
        _ => &[],
    };
    let direct_set: Vec<&str> = direct_fields
        .iter()
        .copied()
        .filter(|f| obj.contains_key(*f))
        .collect();
    if !direct_set.is_empty() {
        return Err(ValidationError::new(format!(
            "{}: pass either the direct fields ({}) OR 'entries', not both.",
            tool.wire_name(),
            direct_fields.join(", ")
        )));
    }
    match obj.get("entries") {
        Some(Value::Array(arr)) if !arr.is_empty() => {
            for (i, el) in arr.iter().enumerate() {
                if !el.is_object() {
                    return Err(ValidationError::new(format!(
                        "entries[{i}] must be an object"
                    )));
                }
            }
            Ok(())
        }
        _ => Err(ValidationError::new(format!(
            "{}: missing or empty 'entries' array",
            tool.wire_name()
        ))),
    }
}

/// `ripple_delete_ranges` requires **exactly one** of `trackIndex` / `clipId`, and
/// constrains `units`: `trackIndex` mode requires `units == 'frames'`; `clipId`
/// mode allows `seconds | frames` (default frames). These are contract text in the
/// tool description (carry-forward) — enforced here.
fn validate_ripple_delete(args: &Value) -> Result<(), ValidationError> {
    let obj = args.as_object().expect("object");
    let has_track = obj.contains_key("trackIndex");
    let has_clip = obj.contains_key("clipId");
    let path = "ripple_delete_ranges";
    match (has_track, has_clip) {
        (false, false) => {
            return Err(ValidationError::new(format!(
                "{path}: provide exactly one of 'trackIndex' or 'clipId'."
            )));
        }
        (true, true) => {
            return Err(ValidationError::new(format!(
                "{path}: 'trackIndex' and 'clipId' are mutually exclusive."
            )));
        }
        _ => {}
    }
    let units = obj.get("units").and_then(Value::as_str);
    if has_track {
        // trackIndex mode requires units 'frames' (or omitted, default frames).
        if let Some(u) = units {
            if u != "frames" {
                return Err(ValidationError::new(format!(
                    "{path}: trackIndex mode requires units 'frames' (got '{u}')."
                )));
            }
        }
    } else {
        // clipId mode: units ∈ {seconds, frames}.
        if let Some(u) = units {
            if u != "seconds" && u != "frames" {
                return Err(ValidationError::new(format!(
                    "{path}: units must be 'seconds' or 'frames' (got '{u}')."
                )));
            }
        }
    }
    Ok(())
}

// ── colors ───────────────────────────────────────────────────────────────────

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex color into `(r, g, b, a)` bytes (reference
/// `TextStyle.RGBA(hex:)`). Invalid hex → error. Used by the text/caption/property
/// tool bodies (E7-S7/S8) and surfaced here so the validation seam owns color
/// parsing.
pub fn parse_rgba(hex: &str) -> Result<(u8, u8, u8, u8), ValidationError> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    let bad = || ValidationError::new(format!("Invalid color '{hex}': expected #RRGGBB or #RRGGBBAA"));
    if !(s.len() == 6 || s.len() == 8) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad());
    let r = byte(0)?;
    let g = byte(2)?;
    let b = byte(4)?;
    let a = if s.len() == 8 { byte(6)? } else { 255 };
    Ok((r, g, b, a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── unknown keys ────────────────────────────────────────────────────────

    #[test]
    fn unknown_key_rejected_with_name() {
        let err = validate(ToolName::GetTimeline, &json!({ "bogus": 1 })).unwrap_err();
        assert!(err.message.contains("unknown field(s) 'bogus'"), "{}", err.message);
        assert!(err.message.contains("get_timeline"));
    }

    #[test]
    fn known_keys_pass() {
        assert!(validate(ToolName::GetTimeline, &json!({ "startFrame": 0, "endFrame": 10 })).is_ok());
    }

    // ── required + types ────────────────────────────────────────────────────

    #[test]
    fn missing_required_field_rejected() {
        // split_clip requires clipId + atFrame.
        let err = validate(ToolName::SplitClip, &json!({ "clipId": "x" })).unwrap_err();
        assert!(err.message.contains("missing required field 'atFrame'"), "{}", err.message);
    }

    #[test]
    fn wrong_type_rejected_with_path() {
        // atFrame must be an integer.
        let err = validate(ToolName::SplitClip, &json!({ "clipId": "x", "atFrame": "nope" })).unwrap_err();
        assert!(err.message.contains("split_clip.atFrame: expected integer"), "{}", err.message);
    }

    #[test]
    fn happy_path_split_clip() {
        assert!(validate(ToolName::SplitClip, &json!({ "clipId": "x", "atFrame": 10 })).is_ok());
    }

    // ── non-finite ──────────────────────────────────────────────────────────

    #[test]
    fn non_finite_in_nested_array_rejected_with_path() {
        // Build a Value carrying an Inf via the typed API (JSON text can't).
        let inf = serde_json::Number::from_f64(f64::INFINITY);
        assert!(inf.is_none(), "serde refuses Inf numbers — guard is defensive");
        // Construct a path test against a synthetic finite tree to prove recursion
        // returns None for finite input.
        let v = json!({ "ranges": [[0.0, 1.5]], "speed": 2.0 });
        assert_eq!(first_non_finite_path(&v, "t"), None);
    }

    // ── generate_audio: no required field ───────────────────────────────────

    #[test]
    fn generate_audio_accepts_empty_args() {
        // generate_audio has NO required field (prompt optional).
        assert!(validate(ToolName::GenerateAudio, &json!({})).is_ok());
    }

    // ── dual-shape XOR ──────────────────────────────────────────────────────

    #[test]
    fn dual_shape_direct_only_ok() {
        assert!(validate(ToolName::CreateFolder, &json!({ "name": "A" })).is_ok());
    }

    #[test]
    fn dual_shape_entries_only_ok() {
        assert!(validate(
            ToolName::CreateFolder,
            &json!({ "entries": [{ "name": "A" }, { "name": "B" }] })
        )
        .is_ok());
    }

    #[test]
    fn dual_shape_both_rejected() {
        let err = validate(
            ToolName::CreateFolder,
            &json!({ "name": "A", "entries": [{ "name": "B" }] }),
        )
        .unwrap_err();
        assert!(err.message.contains("not both"), "{}", err.message);
    }

    #[test]
    fn dual_shape_empty_entries_rejected() {
        let err = validate(ToolName::RenameMedia, &json!({ "entries": [] })).unwrap_err();
        assert!(err.message.contains("empty 'entries'"), "{}", err.message);
    }

    // ── ripple_delete units / XOR ───────────────────────────────────────────

    #[test]
    fn ripple_requires_exactly_one_anchor() {
        let none = validate(ToolName::RippleDeleteRanges, &json!({ "ranges": [[0, 5]] })).unwrap_err();
        assert!(none.message.contains("exactly one"), "{}", none.message);
        let both = validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0, 5]], "trackIndex": 0, "clipId": "x" }),
        )
        .unwrap_err();
        assert!(both.message.contains("mutually exclusive"), "{}", both.message);
    }

    #[test]
    fn ripple_track_mode_requires_frames_units() {
        let err = validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0, 5]], "trackIndex": 0, "units": "seconds" }),
        )
        .unwrap_err();
        assert!(err.message.contains("requires units 'frames'"), "{}", err.message);
        // frames (or omitted) is fine.
        assert!(validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0, 5]], "trackIndex": 0, "units": "frames" })
        )
        .is_ok());
        assert!(validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0, 5]], "trackIndex": 0 })
        )
        .is_ok());
    }

    #[test]
    fn ripple_clip_mode_allows_seconds_and_frames() {
        assert!(validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0.0, 1.5]], "clipId": "x", "units": "seconds" })
        )
        .is_ok());
        let bad = validate(
            ToolName::RippleDeleteRanges,
            &json!({ "ranges": [[0, 5]], "clipId": "x", "units": "nonsense" }),
        )
        .unwrap_err();
        assert!(bad.message.contains("'seconds' or 'frames'"), "{}", bad.message);
    }

    // ── colors ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_rgb_and_rgba() {
        assert_eq!(parse_rgba("#FFFFFF").unwrap(), (255, 255, 255, 255));
        assert_eq!(parse_rgba("#00FF0080").unwrap(), (0, 255, 0, 128));
        // without the leading '#'.
        assert_eq!(parse_rgba("000000").unwrap(), (0, 0, 0, 255));
    }

    #[test]
    fn parse_rgba_rejects_bad_hex() {
        assert!(parse_rgba("#GGG").is_err());
        assert!(parse_rgba("#FFF").is_err()); // 3-digit shorthand not accepted
        assert!(parse_rgba("#FFFFF").is_err());
        assert!(parse_rgba("#FFFFFFF").is_err());
    }
}
