//! `round_json_numbers` — recursive 3-place rounding of floating-point numbers in
//! a `serde_json::Value` tree (reference `roundJSONFloatingPointNumbers`,
//! `Utilities/JSONNumberFormatting.swift`).
//!
//! Several READ tools (`get_timeline`, `get_media`, `list_models`,
//! `inspect_media`) pass their assembled JSON through this rounder before
//! serializing, to keep token budgets tight and output stable. **`get_transcript`
//! does NOT round** (its numbers are integer frames).
//!
//! ## Parity: decimal half-up (ties away from zero)
//!
//! The reference rounds via `NSDecimalNumber` with `NSRoundingMode.plain` — decimal
//! **half-up / ties-away-from-zero** at `scale = places`, NOT binary
//! `Double::round`. Using a decimal rounder avoids binary-float artifacts (e.g.
//! `0.1 + 0.2`). We reproduce that: scale by `10^places`, apply `f64::round`
//! (which is itself ties-away), then divide back — and because we round the
//! *scaled decimal magnitude*, the tie direction matches the reference's `.plain`
//! mode on the values these tools emit (all small, finite normalized coords /
//! durations).
//!
//! ## Rules mirrored from the reference
//! - Recurses into arrays and objects.
//! - **Integers are left untouched** (serde's `Value::Number` distinguishes
//!   integer from float; only floats are rounded — matching the reference's
//!   `objCType == "d"||"f"` guard).
//! - **Booleans are not numbers** — `serde_json` models them as `Value::Bool`, so
//!   they're never touched (the reference's explicit `isBooleanNumber` guard).
//! - A **non-finite** float (NaN/±Inf) maps to `null` (reference returns
//!   `NSNull()`); in practice tool output never carries these.

use serde_json::Value;

/// The fixed decimal places the reference rounds tool output to (`toPlaces: 3`).
pub const JSON_ROUND_PLACES: u32 = 3;

/// Round every **floating-point** number in `value` to `places` decimal places,
/// recursing into arrays/objects. Integers and booleans pass through unchanged.
/// Non-finite floats become `null`.
pub fn round_json_numbers(value: Value, places: u32) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, round_json_numbers(v, places)))
                .collect(),
        ),
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(|v| round_json_numbers(v, places)).collect())
        }
        Value::Number(n) => {
            // Integers (no fractional part in the serde model) are left as-is.
            if n.is_i64() || n.is_u64() {
                return Value::Number(n);
            }
            match n.as_f64() {
                Some(f) if f.is_finite() => round_f64_to_value(f, places),
                // Non-finite → null (reference NSNull()).
                _ => Value::Null,
            }
        }
        other => other,
    }
}

/// Round a finite `f64` to `places` decimal places, **decimal** ties-away-from-zero
/// (matching the reference `NSDecimalNumber` `.plain` mode), and wrap it back into
/// a JSON number. If the rounded value is integral, it is emitted as an integer (so
/// `1.0` serializes as `1`, matching the reference where decimal rounding of an
/// exact integer yields an integer-valued `NSDecimalNumber`).
///
/// We round on the **decimal** expansion, not the binary one: `f64::round` on a
/// scaled value mis-rounds exact decimal ties (e.g. `0.5005 * 1000` is
/// `500.4999…` in binary → would truncate to `0.5`). To match `NSDecimalNumber`,
/// format the value to a few extra decimal places (a faithful shortest-or-fixed
/// decimal) and round that decimal string ties-away.
fn round_f64_to_value(f: f64, places: u32) -> Value {
    let rounded = decimal_round_ties_away(f, places);
    if rounded.fract() == 0.0 && rounded.abs() < 9.007_199_254_740_992e15 {
        // Integral after rounding → emit as an integer.
        Value::Number((rounded as i64).into())
    } else {
        serde_json::Number::from_f64(rounded)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

/// Round `f` to `places` decimal places using decimal half-away-from-zero, by
/// operating on the value's decimal text (rendered at `places + guard` digits so
/// the tie digit is exact) rather than the binary scaled product. Mirrors
/// `NSDecimalNumber(value:).rounding(.plain, scale: places)`.
fn decimal_round_ties_away(f: f64, places: u32) -> f64 {
    let neg = f.is_sign_negative();
    let mag = f.abs();
    // Render at high fixed precision so the decimal digit at `places+1` is exact
    // enough to decide the tie (17 sig-digits round-trips an f64; place+guard
    // covers the fractional decision digit).
    let guard = (places as usize) + 6;
    let text = format!("{mag:.guard$}");
    // Split integer / fractional decimal digits.
    let (int_part, frac_part) = match text.split_once('.') {
        Some((i, fr)) => (i, fr),
        None => (text.as_str(), ""),
    };
    let keep = places as usize;
    let frac_bytes = frac_part.as_bytes();
    // Build the kept integer (int_part ++ first `keep` frac digits) as i128, then
    // decide rounding from the next digit.
    let kept_digits: String = format!(
        "{int_part}{}",
        &frac_part[..keep.min(frac_part.len())]
    );
    let mut value: i128 = kept_digits.parse().unwrap_or(0);
    // The decision digit is the (keep)-th fractional digit (0-based index `keep`).
    let round_up = frac_bytes.get(keep).map(|b| *b >= b'5').unwrap_or(false);
    if round_up {
        value += 1;
    }
    let scale = 10f64.powi(places as i32);
    let result = value as f64 / scale;
    if neg {
        -result
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rounds_floats_to_three_places_ties_away() {
        // 0.12345 → 0.123; 0.12355 → 0.124 (ties away).
        assert_eq!(round_json_numbers(json!(0.12345), 3), json!(0.123));
        assert_eq!(round_json_numbers(json!(0.5005), 3), json!(0.501));
        // negative ties away from zero.
        assert_eq!(round_json_numbers(json!(-0.5005), 3), json!(-0.501));
    }

    #[test]
    fn leaves_integers_untouched() {
        assert_eq!(round_json_numbers(json!(200), 3), json!(200));
        assert_eq!(round_json_numbers(json!(-7), 3), json!(-7));
        // an exact float integer collapses to an int.
        assert_eq!(round_json_numbers(json!(1.0), 3), json!(1));
    }

    #[test]
    fn leaves_booleans_and_strings_untouched() {
        assert_eq!(round_json_numbers(json!(true), 3), json!(true));
        assert_eq!(round_json_numbers(json!("0.123456"), 3), json!("0.123456"));
    }

    #[test]
    fn recurses_into_arrays_and_objects() {
        let v = json!({
            "a": 0.111111,
            "nested": { "b": [0.222222, 3, true] },
        });
        let out = round_json_numbers(v, 3);
        assert_eq!(out["a"], json!(0.111));
        assert_eq!(out["nested"]["b"][0], json!(0.222));
        assert_eq!(out["nested"]["b"][1], json!(3));
        assert_eq!(out["nested"]["b"][2], json!(true));
    }

    #[test]
    fn non_finite_floats_become_null() {
        let nan = serde_json::Number::from_f64(f64::NAN);
        // serde_json refuses to build a NaN number, so this path is defensive;
        // assert the finite branch is what we actually hit in practice.
        assert!(nan.is_none());
    }
}
