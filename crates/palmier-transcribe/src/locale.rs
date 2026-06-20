//! Locale resolution (E10-S3).
//!
//! Clean-room parity port of the macOS reference
//! `Sources/PalmierPro/Transcription/Transcription.swift` —
//! `supportedLocales`, `bestSupportedLocale`, `matchLocale`. The reference leans on
//! Apple `Locale`; here we model a small BCP-47 [`LocaleTag`] (language + optional
//! region) and replicate the matching *order* exactly:
//!
//! 1. Prefer a `preferred_locale` if `match_locale` finds it among the supported set.
//! 2. Else fall back to a best-supported locale derived from the **OS locale**
//!    (`sys-locale`, replacing `Locale.preferredLanguages` / `Locale.current`).
//! 3. Else `TranscriptionError::UnsupportedLocale`.
//!
//! `match_locale` = first candidate whose **BCP-47 language code** has any supported
//! match, **preferring an exact region**, else the first same-language entry. Region
//! and language comparisons are case-insensitive (BCP-47 is case-insensitive; we
//! normalize lang to lowercase, region to uppercase on parse — matching how the
//! reference reads `languageCode` / `region.identifier`).
//!
//! For `.en` Whisper models the supported set is English only, so the resolver always
//! lands on an English tag (the engine, E10-S2, passes [`english_only_supported`] as
//! the supported set for `.en` models). Whisper auto-detects language from audio when
//! no override is given; the override path uses this resolver.

use crate::error::TranscriptionError;

/// A parsed BCP-47 locale tag: a language subtag plus an optional region subtag.
///
/// This is the parity stand-in for the reference's Apple `Locale` in the matching
/// logic — we only need the language code and region for `match_locale`. `language`
/// is normalized to lowercase, `region` to uppercase (BCP-47 convention), so equality
/// comparisons are case-insensitive without per-call `to_lowercase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleTag {
    /// Lowercased primary language subtag, e.g. `"en"`, `"fr"`, `"zh"`.
    pub language: String,
    /// Uppercased region subtag if present, e.g. `"US"`, `"GB"`. `None` when the tag
    /// carries no region (e.g. `"en"`).
    pub region: Option<String>,
}

impl LocaleTag {
    /// Parse a BCP-47 (or POSIX-ish) tag into language + optional region.
    ///
    /// Accepts the common shapes seen from `sys-locale` and user/config input:
    /// `"en"`, `"en-US"`, `"en_US"`, `"zh-Hans-CN"` (script subtag ignored — only
    /// language + region are used for matching, matching the reference which keys on
    /// `languageCode` and `region`), and tolerates a trailing `.UTF-8` POSIX codeset.
    /// Returns `None` for an empty/blank tag.
    #[must_use]
    pub fn parse(tag: &str) -> Option<LocaleTag> {
        // Drop a POSIX codeset suffix (`en_US.UTF-8`) and any `@modifier`.
        let core = tag
            .split(['.', '@'])
            .next()
            .unwrap_or("")
            .trim();
        if core.is_empty() {
            return None;
        }
        // BCP-47 uses '-', POSIX uses '_'. Split on either.
        let mut parts = core.split(['-', '_']).filter(|p| !p.is_empty());
        let language = parts.next()?.to_ascii_lowercase();
        if language.is_empty() {
            return None;
        }
        // The region is the first subsequent subtag that looks like a region:
        // a 2-letter alpha code or a 3-digit UN M.49 code. A 4-letter subtag is a
        // script (e.g. `Hans`) and is skipped, mirroring the reference's region-only
        // read.
        let region = parts.find_map(|p| {
            let is_alpha2 = p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic());
            let is_digit3 = p.len() == 3 && p.chars().all(|c| c.is_ascii_digit());
            if is_alpha2 || is_digit3 {
                Some(p.to_ascii_uppercase())
            } else {
                None
            }
        });
        Some(LocaleTag {
            language,
            region,
        })
    }

    /// Render this tag back to a canonical BCP-47 string (`"en"` or `"en-US"`).
    ///
    /// Used to set `TranscriptionResult.language` to the resolved locale's BCP-47 tag.
    #[must_use]
    pub fn to_bcp47(&self) -> String {
        match &self.region {
            Some(region) => format!("{}-{}", self.language, region),
            None => self.language.clone(),
        }
    }
}

/// The supported-locale set for an English-only (`.en`) Whisper model.
///
/// For `.en` models the engine only understands English, so the resolver is given
/// just `en-US` as the supported universe — any candidate whose language is `en`
/// resolves; anything else falls through to `UnsupportedLocale` (or, for a regionless
/// `en` candidate, lands on `en-US`).
#[must_use]
pub fn english_only_supported() -> Vec<LocaleTag> {
    vec![LocaleTag {
        language: "en".to_string(),
        region: Some("US".to_string()),
    }]
}

/// Parity port of the reference `matchLocale(candidates:supported:)`.
///
/// Walks `candidates` in order; for the first candidate whose **language code** has any
/// match in `supported`, returns the supported entry with an **exact region** match if
/// one exists, else the **first** same-language supported entry. Returns `None` when no
/// candidate's language is supported.
#[must_use]
pub fn match_locale(candidates: &[LocaleTag], supported: &[LocaleTag]) -> Option<LocaleTag> {
    for candidate in candidates {
        let same_lang: Vec<&LocaleTag> = supported
            .iter()
            .filter(|s| s.language == candidate.language)
            .collect();
        if same_lang.is_empty() {
            continue;
        }
        // Prefer an exact region match (`region == candidate.region`, including the
        // `None == None` case), else the first same-language entry — exactly the
        // reference `sameLang.first { $0.region == region } ?? sameLang.first`.
        let exact = same_lang
            .iter()
            .find(|s| s.region == candidate.region)
            .copied();
        return Some(exact.unwrap_or_else(|| same_lang[0]).clone());
    }
    None
}

/// Parity port of the reference `bestSupportedLocale(from:)`.
///
/// Candidates = the OS preferred locale(s) followed by the current locale, replacing
/// `Locale.preferredLanguages + [Locale.current]`. On this platform `sys-locale`
/// exposes the ordered preferred-language list (`get_locales()`), whose first entry is
/// effectively `Locale.current`; we feed that ordered list through [`match_locale`].
#[must_use]
pub fn best_supported_locale(supported: &[LocaleTag]) -> Option<LocaleTag> {
    let candidates = os_candidate_locales();
    match_locale(&candidates, supported)
}

/// The OS-derived candidate locales, most-preferred first.
///
/// Replaces `Locale.preferredLanguages.map(Locale.init) + [Locale.current]`. Uses
/// `sys-locale::get_locales()` (an ordered iterator of the user's preferred BCP-47
/// tags); the first entry doubles as `Locale.current`. Unparseable/blank tags are
/// dropped. Returns an empty `Vec` when the OS exposes no locale (head-less CI).
#[must_use]
pub fn os_candidate_locales() -> Vec<LocaleTag> {
    sys_locale::get_locales()
        .filter_map(|tag| LocaleTag::parse(&tag))
        .collect()
}

/// Resolve the transcription locale, parity with the reference `transcribe`'s locale
/// block: **prefer the user locale, else auto-detect from the OS, else error.**
///
/// - If `preferred_locale` is `Some` and `match_locale([preferred], supported)` finds
///   it, that match wins.
/// - Else `best_supported_locale(supported)` (OS-derived) is used.
/// - Else `TranscriptionError::UnsupportedLocale(<bcp47 of the preferred-or-current
///   locale>)` — matching the reference's
///   `throw .unsupportedLocale((preferredLocale ?? Locale.current).identifier(.bcp47))`.
///
/// `preferred_locale` is the raw user/config tag (e.g. `"en"`, `"fr-FR"`); it is parsed
/// internally. For `.en` models pass [`english_only_supported`] as `supported`.
///
/// # Errors
/// Returns [`TranscriptionError::UnsupportedLocale`] when neither the preferred locale
/// nor any OS-derived candidate has a supported-language match.
pub fn resolve_locale(
    preferred_locale: Option<&str>,
    supported: &[LocaleTag],
) -> Result<LocaleTag, TranscriptionError> {
    let preferred = preferred_locale.and_then(LocaleTag::parse);

    if let Some(ref pref) = preferred {
        if let Some(matched) = match_locale(std::slice::from_ref(pref), supported) {
            return Ok(matched);
        }
    }

    if let Some(auto) = best_supported_locale(supported) {
        return Ok(auto);
    }

    // Parity with `(preferredLocale ?? Locale.current).identifier(.bcp47)`: report the
    // preferred tag if given, else the current OS locale, else the raw input.
    let reported = preferred
        .map(|p| p.to_bcp47())
        .or_else(|| os_candidate_locales().first().map(LocaleTag::to_bcp47))
        .or_else(|| preferred_locale.map(str::to_string))
        .unwrap_or_default();
    Err(TranscriptionError::UnsupportedLocale(reported))
}

/// Convenience wrapper for `.en` Whisper models: resolve against the English-only
/// supported set. This is the seam E10-S2's engine consumes — it always yields an
/// English BCP-47 tag for an English (or regionless) candidate, and
/// `UnsupportedLocale` for a non-English override.
///
/// # Errors
/// Returns [`TranscriptionError::UnsupportedLocale`] when the override (and OS locale)
/// is non-English.
pub fn resolve_locale_en(preferred_locale: Option<&str>) -> Result<LocaleTag, TranscriptionError> {
    resolve_locale(preferred_locale, &english_only_supported())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(lang: &str, region: Option<&str>) -> LocaleTag {
        LocaleTag {
            language: lang.to_string(),
            region: region.map(str::to_string),
        }
    }

    #[test]
    fn parse_handles_bcp47_posix_and_script() {
        assert_eq!(LocaleTag::parse("en"), Some(loc("en", None)));
        assert_eq!(LocaleTag::parse("en-US"), Some(loc("en", Some("US"))));
        assert_eq!(LocaleTag::parse("en_US"), Some(loc("en", Some("US"))));
        // Script subtag (Hans) is skipped; region (CN) is captured.
        assert_eq!(LocaleTag::parse("zh-Hans-CN"), Some(loc("zh", Some("CN"))));
        // POSIX codeset + case normalization.
        assert_eq!(
            LocaleTag::parse("FR_fr.UTF-8"),
            Some(loc("fr", Some("FR")))
        );
        // UN M.49 numeric region.
        assert_eq!(LocaleTag::parse("es-419"), Some(loc("es", Some("419"))));
        assert_eq!(LocaleTag::parse(""), None);
        assert_eq!(LocaleTag::parse("   "), None);
    }

    #[test]
    fn to_bcp47_roundtrips() {
        assert_eq!(loc("en", None).to_bcp47(), "en");
        assert_eq!(loc("en", Some("US")).to_bcp47(), "en-US");
    }

    #[test]
    fn match_locale_prefers_exact_region() {
        let supported = vec![loc("en", Some("GB")), loc("en", Some("US"))];
        // Candidate en-US: exact region wins even though en-GB is listed first.
        let m = match_locale(&[loc("en", Some("US"))], &supported);
        assert_eq!(m, Some(loc("en", Some("US"))));
    }

    #[test]
    fn match_locale_falls_back_to_first_same_language() {
        let supported = vec![loc("en", Some("GB")), loc("en", Some("US"))];
        // Candidate en-AU: no exact region → first same-language entry (en-GB).
        let m = match_locale(&[loc("en", Some("AU"))], &supported);
        assert_eq!(m, Some(loc("en", Some("GB"))));
    }

    #[test]
    fn match_locale_regionless_candidate_uses_first_same_language() {
        let supported = vec![loc("en", Some("GB")), loc("en", Some("US"))];
        // Candidate `en` (no region): no exact `None` region in supported → first.
        let m = match_locale(&[loc("en", None)], &supported);
        assert_eq!(m, Some(loc("en", Some("GB"))));
    }

    #[test]
    fn match_locale_walks_candidates_in_order() {
        let supported = vec![loc("fr", Some("FR"))];
        // First candidate (en) unsupported → falls to second (fr).
        let m = match_locale(&[loc("en", Some("US")), loc("fr", Some("CA"))], &supported);
        assert_eq!(m, Some(loc("fr", Some("FR"))));
    }

    #[test]
    fn match_locale_returns_none_when_no_language_supported() {
        let supported = vec![loc("en", Some("US"))];
        assert_eq!(match_locale(&[loc("de", Some("DE"))], &supported), None);
    }

    #[test]
    fn resolve_prefers_user_locale_when_supported() {
        let supported = vec![loc("en", Some("GB")), loc("en", Some("US"))];
        let r = resolve_locale(Some("en-US"), &supported).expect("resolves");
        assert_eq!(r, loc("en", Some("US")));
    }

    #[test]
    fn resolve_errors_when_preferred_unsupported_and_no_os_match() {
        // A supported set with no English entry; preferred `de-DE` unsupported. The OS
        // locale on CI may or may not match, so use a supported set unlikely to match
        // any OS locale to force the error deterministically: a synthetic language.
        let supported = vec![loc("xx", Some("ZZ"))];
        let err = resolve_locale(Some("de-DE"), &supported).unwrap_err();
        match err {
            TranscriptionError::UnsupportedLocale(id) => assert_eq!(id, "de-DE"),
            other => panic!("expected UnsupportedLocale, got {other:?}"),
        }
    }

    #[test]
    fn resolve_en_accepts_english_override() {
        let r = resolve_locale_en(Some("en-GB")).expect("english resolves");
        // English-only supported is en-US; en-GB has no exact region → first (en-US).
        assert_eq!(r, loc("en", Some("US")));
        assert_eq!(r.to_bcp47(), "en-US");
    }

    #[test]
    fn resolve_en_regionless_english_resolves_to_en_us() {
        let r = resolve_locale_en(Some("en")).expect("english resolves");
        assert_eq!(r, loc("en", Some("US")));
    }

    #[test]
    fn resolve_en_non_english_override_falls_back_to_os_then_errors() {
        // Parity: a non-English override is unsupported by the `.en` set, so resolution
        // falls through to OS auto-detect (reference `bestSupportedLocale`). The result
        // is environment-dependent — on an English-locale OS it resolves to en-US; on a
        // non-English-locale OS (no English candidate) it errors with the override's
        // BCP-47 tag. Both branches are valid parity outcomes; assert whichever holds.
        match resolve_locale_en(Some("fr-FR")) {
            Ok(tag) => assert_eq!(tag.language, "en"),
            Err(TranscriptionError::UnsupportedLocale(id)) => assert_eq!(id, "fr-FR"),
            Err(other) => panic!("expected UnsupportedLocale, got {other:?}"),
        }
    }

    #[test]
    fn resolve_non_english_override_against_synthetic_set_errors() {
        // Deterministic rejection: a supported set with a synthetic language the OS will
        // never report (`xx`) forces the OS fallback to miss, so the override tag is
        // reported verbatim — exactly the reference's
        // `(preferredLocale ?? .current).identifier(.bcp47)`.
        let supported = vec![loc("xx", Some("ZZ"))];
        let err = resolve_locale(Some("fr-FR"), &supported).unwrap_err();
        match err {
            TranscriptionError::UnsupportedLocale(id) => assert_eq!(id, "fr-FR"),
            other => panic!("expected UnsupportedLocale, got {other:?}"),
        }
    }

    #[test]
    fn english_only_supported_is_en_us() {
        assert_eq!(english_only_supported(), vec![loc("en", Some("US"))]);
    }
}
