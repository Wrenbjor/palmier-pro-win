//! ClipType **extension gate** + the Lottie second-gate sniff (story E4-S1).
//!
//! The import pipeline classifies every dropped file by extension. This module
//! is the single, case-insensitive entry point for that — it owns the lowercasing
//! the reference does inline (`ClipType(fileExtension:)` in
//! `Sources/PalmierPro/Models/ClipType.swift` is fed an already-lowercased ext by
//! `MediaTab.swift`). The raw, case-*sensitive* table itself lives in
//! [`palmier_model::ClipType::from_file_extension`]; here we lowercase first so a
//! `.MOV` / `.JPG` / `.PNG` drop classifies exactly like its lowercase twin.
//!
//! ## The two gates
//! 1. **Extension gate** — [`clip_type_for_extension`] / [`clip_type_for_path`]:
//!    lowercase the extension, look it up in the reference table. Unknown → `None`
//!    (the caller emits a `mediaPanelToast` "unsupported file type" and drops it).
//! 2. **Lottie second gate** — a `.json` only classifies as `lottie` if
//!    [`is_lottie`] passes (port of `LottieVideoGenerator.isLottie(at:)`). A plain
//!    `{}` / non-Lottie JSON is *refused*, not imported. `.lottie` files skip the
//!    sniff (the extension is authoritative). [`classify_path`] applies both gates
//!    together — it is the function the import pipeline should call.
//!
//! See `docs/reference/media-panel.md` §"Import & supported extensions" and
//! `_bmad-output/implementation-artifacts/epic-04-media-panel.md` (E4-S1).

use std::path::Path;

pub use palmier_model::ClipType;

/// Map a file **extension** (with or without a leading dot, any case) to a
/// [`ClipType`], or `None` if unsupported.
///
/// This is the case-insensitive media-side gate over
/// [`ClipType::from_file_extension`] (which is case-*sensitive* and dot-free, a
/// 1:1 port of the Swift enum). We strip a single leading `.` and lowercase so
/// `"MOV"`, `".mov"`, `"Mov"` and `"mov"` all resolve to [`ClipType::Video`].
///
/// Supported (per the reference table):
/// - video: `mov, mp4, m4v` · audio: `mp3, wav, aac, m4a`
/// - image: `png, jpg, jpeg, tiff, heic, webp` · lottie: `json, lottie`
///
/// NOTE: a `json` resolves to [`ClipType::Lottie`] here on **extension alone** —
/// the Lottie content sniff is the *separate* second gate ([`is_lottie`]). Use
/// [`classify_path`] to apply both at once.
pub fn clip_type_for_extension(ext: &str) -> Option<ClipType> {
    let normalized = ext.trim_start_matches('.').to_ascii_lowercase();
    ClipType::from_file_extension(&normalized)
}

/// Map a file **path**'s extension to a [`ClipType`] (extension gate only).
///
/// Returns `None` for a path with no extension or an unsupported one. Like
/// [`clip_type_for_extension`], a `.json` resolves to [`ClipType::Lottie`] on
/// extension alone; prefer [`classify_path`] which also runs the Lottie sniff.
pub fn clip_type_for_path(path: impl AsRef<Path>) -> Option<ClipType> {
    let ext = path.as_ref().extension()?.to_str()?;
    clip_type_for_extension(ext)
}

/// Apply **both** gates to a path: the extension gate, then — for a `.json` that
/// classified as [`ClipType::Lottie`] — the Lottie content sniff.
///
/// A `.json` whose content is not a Lottie animation is **rejected** (returns
/// `None`), matching the reference: extension match alone is insufficient for
/// JSON. A `.lottie` file is accepted on its extension without sniffing (the
/// reference trusts the explicit `.lottie` extension). Reading the file for the
/// sniff is only attempted for `.json`; any other classified type returns
/// immediately. An I/O error reading a `.json` is treated as "not a Lottie" →
/// `None` (the file can't be validated, so it isn't imported).
pub fn classify_path(path: impl AsRef<Path>) -> Option<ClipType> {
    let path = path.as_ref();
    let clip_type = clip_type_for_path(path)?;

    // Only `.json` needs the second gate. `.lottie` (and every non-lottie type)
    // is authoritative on extension alone.
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if clip_type == ClipType::Lottie && is_json && !is_lottie(path) {
        return None;
    }
    Some(clip_type)
}

/// Sniff whether the file at `path` is a real Lottie animation.
///
/// Port of `LottieVideoGenerator.isLottie(at:)`: a Lottie JSON is a top-level
/// object carrying the Bodymovin/Lottie schema markers. We require the canonical
/// trio that every Lottie export has — `"v"` (version, a string like `"5.7.4"`),
/// the in/out points `"ip"` and `"op"` (numbers), and a `"layers"` array — which
/// a plain `{}` or an arbitrary JSON object will not satisfy. Returns `false` on
/// any read/parse error or non-object root.
pub fn is_lottie(path: impl AsRef<Path>) -> bool {
    let Ok(bytes) = std::fs::read(path.as_ref()) else {
        return false;
    };
    is_lottie_bytes(&bytes)
}

/// [`is_lottie`] over an in-memory buffer (the testable core).
pub fn is_lottie_bytes(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    // Canonical Lottie markers. `v` is the schema version, `ip`/`op` the in/out
    // frames, `layers` the animation content — all present in every Lottie export
    // and absent from arbitrary JSON.
    obj.get("v").is_some_and(|v| v.is_string())
        && obj.get("ip").is_some_and(|v| v.is_number())
        && obj.get("op").is_some_and(|v| v.is_number())
        && obj.get("layers").is_some_and(|v| v.is_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_table_every_supported_ext() {
        // video
        for e in ["mov", "mp4", "m4v"] {
            assert_eq!(clip_type_for_extension(e), Some(ClipType::Video), "ext {e}");
        }
        // audio
        for e in ["mp3", "wav", "aac", "m4a"] {
            assert_eq!(clip_type_for_extension(e), Some(ClipType::Audio), "ext {e}");
        }
        // image
        for e in ["png", "jpg", "jpeg", "tiff", "heic", "webp"] {
            assert_eq!(clip_type_for_extension(e), Some(ClipType::Image), "ext {e}");
        }
        // lottie (extension gate only)
        for e in ["json", "lottie"] {
            assert_eq!(clip_type_for_extension(e), Some(ClipType::Lottie), "ext {e}");
        }
    }

    #[test]
    fn extension_table_rejects_unsupported() {
        for e in ["txt", "mkv", "avi", "gif", "", "doc"] {
            assert_eq!(clip_type_for_extension(e), None, "ext {e:?} must be unsupported");
        }
    }

    #[test]
    fn extension_gate_is_case_insensitive_and_dot_tolerant() {
        // Uppercase — the reference lowercases before the table lookup.
        assert_eq!(clip_type_for_extension("MOV"), Some(ClipType::Video));
        assert_eq!(clip_type_for_extension("JPG"), Some(ClipType::Image));
        assert_eq!(clip_type_for_extension("PnG"), Some(ClipType::Image));
        assert_eq!(clip_type_for_extension("M4A"), Some(ClipType::Audio));
        // Leading dot tolerated.
        assert_eq!(clip_type_for_extension(".mp4"), Some(ClipType::Video));
        assert_eq!(clip_type_for_extension(".Tiff"), Some(ClipType::Image));
    }

    #[test]
    fn clip_type_for_path_uses_extension() {
        assert_eq!(
            clip_type_for_path("C:/clips/My Clip.MP4"),
            Some(ClipType::Video)
        );
        assert_eq!(
            clip_type_for_path("/home/u/pic.JPEG"),
            Some(ClipType::Image)
        );
        assert_eq!(clip_type_for_path("/tmp/no_extension"), None);
        assert_eq!(clip_type_for_path("/tmp/archive.zip"), None);
    }

    #[test]
    fn lottie_sniff_accepts_real_lottie_rejects_plain_json() {
        let real = br#"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":512,"h":512,"layers":[]}"#;
        assert!(is_lottie_bytes(real), "a real Lottie JSON must pass");

        // Plain / non-Lottie JSON objects are refused.
        assert!(!is_lottie_bytes(b"{}"), "empty object is not Lottie");
        assert!(
            !is_lottie_bytes(br#"{"hello":"world"}"#),
            "arbitrary object is not Lottie"
        );
        // Missing one required marker (`layers`).
        assert!(
            !is_lottie_bytes(br#"{"v":"5.7.4","ip":0,"op":60}"#),
            "missing layers ⇒ not Lottie"
        );
        // `v` must be a string, `ip`/`op` numbers.
        assert!(
            !is_lottie_bytes(br#"{"v":5,"ip":0,"op":60,"layers":[]}"#),
            "numeric version ⇒ not Lottie"
        );
        // Non-object root / invalid JSON.
        assert!(!is_lottie_bytes(b"[]"), "array root is not Lottie");
        assert!(!is_lottie_bytes(b"not json"), "invalid JSON is not Lottie");
    }

    #[test]
    fn classify_path_applies_lottie_second_gate() {
        use std::io::Write;

        // A `.json` that is a real Lottie → accepted as lottie.
        let mut real = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        real.write_all(br#"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":256,"h":256,"layers":[]}"#)
            .unwrap();
        assert_eq!(classify_path(real.path()), Some(ClipType::Lottie));

        // A `.json` that is NOT a Lottie → rejected (None), not imported.
        let mut plain = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        plain.write_all(b"{}").unwrap();
        assert_eq!(classify_path(plain.path()), None);

        // Non-json types skip the sniff entirely (no file read needed/attempted).
        assert_eq!(classify_path("C:/x/clip.mp4"), Some(ClipType::Video));
        assert_eq!(classify_path("C:/x/photo.png"), Some(ClipType::Image));
        // A `.lottie` file is trusted on its extension (sniff skipped) even if the
        // path doesn't exist on disk.
        assert_eq!(classify_path("C:/x/anim.lottie"), Some(ClipType::Lottie));
    }
}
