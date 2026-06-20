//! Media resolution seam for the XMEML emitter.
//!
//! The reference emitter consults a `MediaResolver` (`Models/MediaResolver.swift`)
//! for three things: the manifest [`entry`](MediaResolver::entry) for a media ref,
//! its [`display_name`](MediaResolver::display_name), and a resolvable
//! [`resolve_url`](MediaResolver::resolve_url) (`nil`/`None` when the file is
//! missing — those clips are dropped from emission and from `<link>` indexing).
//!
//! In the macOS app the URL comes from the live filesystem (`resolveURL` returns
//! a URL only when `fileExists`). The pure XMEML emitter needs *a* URL string to
//! build `<pathurl>` and to decide "resolvable", so we model the resolver as a
//! trait: the production wiring (palmier-project, later) supplies a
//! filesystem-backed impl; the [`ManifestResolver`] here is a pure, deterministic
//! resolver over a [`MediaManifest`] that the golden tests use.
//!
//! ## `pathurl` parity
//!
//! The reference builds `pathurl` from `url.absoluteString` (a `file://…` URL)
//! and then rewrites `file://` → `file://localhost//` (Premiere needs the extra
//! slash). [`file_url_string`] reproduces macOS `URL(fileURLWithPath:)
//! .absoluteString` for an absolute POSIX path so that rewrite lands on the same
//! bytes. The Builder owns the `localhost` rewrite.

use palmier_model::{MediaManifest, MediaManifestEntry, MediaSource};

/// What the XMEML [`Builder`](crate::xmeml::Builder) needs from media resolution.
///
/// Mirrors the reference `MediaResolver` surface used by `XMLExporter`
/// (`entry(for:)`, `displayName(for:)`, `resolveURL(for:)`).
pub trait MediaResolver {
    /// The manifest entry for `media_ref`, if present.
    fn entry(&self, media_ref: &str) -> Option<&MediaManifestEntry>;

    /// Display name for the clip (`entry.name`, else the reference's `"Offline"`).
    fn display_name(&self, media_ref: &str) -> String {
        self.entry(media_ref)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "Offline".to_string())
    }

    /// A resolvable `file://…` URL string for the media, or `None` when the
    /// media is unresolvable (missing). Clips whose media is unresolvable are
    /// **dropped** from track emission and from `<link>` indexing.
    fn resolve_url(&self, media_ref: &str) -> Option<String>;
}

/// A pure, deterministic [`MediaResolver`] over a [`MediaManifest`], with an
/// optional project base directory used to resolve `Project { relative_path }`
/// sources. Used by the golden tests (no filesystem access — every entry whose
/// URL can be *formed* is treated as resolvable).
///
/// `project_base` is an absolute POSIX-style directory path (e.g.
/// `/Users/x/proj.palmier`) joined with the entry's relative path.
pub struct ManifestResolver {
    manifest: MediaManifest,
    project_base: Option<String>,
}

impl ManifestResolver {
    /// Resolver with no project base (only `External` entries resolve).
    pub fn new(manifest: MediaManifest) -> Self {
        ManifestResolver {
            manifest,
            project_base: None,
        }
    }

    /// Resolver with an absolute project base dir for `Project` sources.
    pub fn with_project_base(manifest: MediaManifest, project_base: impl Into<String>) -> Self {
        ManifestResolver {
            manifest,
            project_base: Some(project_base.into()),
        }
    }

    /// The absolute POSIX path for an entry's source, mirroring the reference
    /// `MediaResolver.expectedURL`: `External` → its `absolute_path`; `Project`
    /// → `project_base / relative_path` (requires a base).
    fn expected_path(&self, entry: &MediaManifestEntry) -> Option<String> {
        match &entry.source {
            MediaSource::External { absolute_path } => Some(absolute_path.clone()),
            MediaSource::Project { relative_path } => self
                .project_base
                .as_ref()
                .map(|base| join_path(base, relative_path)),
        }
    }
}

impl MediaResolver for ManifestResolver {
    fn entry(&self, media_ref: &str) -> Option<&MediaManifestEntry> {
        self.manifest.entries.iter().find(|e| e.id == media_ref)
    }

    fn resolve_url(&self, media_ref: &str) -> Option<String> {
        let entry = self.entry(media_ref)?;
        let path = self.expected_path(entry)?;
        Some(file_url_string(&path))
    }
}

/// Join an absolute base dir and a relative path with a single `/` (no
/// normalization — the inputs are already in `media/<file>` form).
fn join_path(base: &str, rel: &str) -> String {
    let base = base.trim_end_matches('/');
    let rel = rel.trim_start_matches('/');
    format!("{base}/{rel}")
}

/// Build a `file://…` URL string from an absolute POSIX path, reproducing macOS
/// `URL(fileURLWithPath:).absoluteString`.
///
/// macOS yields `file://` + percent-encoded absolute path (the host is empty, so
/// `file://` is followed directly by the leading `/` of the path → `file:///…`).
/// We percent-encode the characters macOS encodes in a file URL path: space and
/// the RFC-3986 reserved/unsafe set, while leaving `/` and the common
/// unreserved/safe path characters intact. The Builder then rewrites the leading
/// `file://` → `file://localhost//`.
pub fn file_url_string(abs_path: &str) -> String {
    // Ensure a leading slash (absolute path).
    let path = if abs_path.starts_with('/') {
        abs_path.to_string()
    } else {
        format!("/{abs_path}")
    };
    let encoded = percent_encode_path(&path);
    format!("file://{encoded}")
}

/// Percent-encode a file-URL path component-wise, matching the set macOS encodes.
///
/// Unreserved per RFC 3986 (`A-Z a-z 0-9 - _ . ~`) plus the sub-delims and
/// path-safe characters Foundation leaves unescaped in a file URL path
/// (`/`, `!`, `$`, `&`, `'`, `(`, `)`, `*`, `+`, `,`, `:`, `=`, `@`) pass
/// through; everything else (notably space → `%20`) is percent-encoded.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = matches!(b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~'
            | b'/' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')'
            | b'*' | b'+' | b',' | b':' | b'=' | b'@');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_upper(b >> 4));
            out.push(hex_upper(b & 0x0f));
        }
    }
    out
}

fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::ClipType;

    fn entry(id: &str, name: &str, source: MediaSource) -> MediaManifestEntry {
        MediaManifestEntry {
            id: id.into(),
            name: name.into(),
            asset_type: ClipType::Video,
            source,
            duration: 4.0,
            generation_input: None,
            source_width: Some(1920),
            source_height: Some(1080),
            source_fps: Some(30.0),
            has_audio: Some(true),
            folder_id: None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        }
    }

    #[test]
    fn external_resolves_to_file_url() {
        let mut m = MediaManifest::new();
        m.entries.push(entry(
            "a1",
            "Clip One.mov",
            MediaSource::External {
                absolute_path: "/Users/x/Movies/Clip One.mov".into(),
            },
        ));
        let r = ManifestResolver::new(m);
        assert_eq!(r.display_name("a1"), "Clip One.mov");
        assert_eq!(
            r.resolve_url("a1").as_deref(),
            Some("file:///Users/x/Movies/Clip%20One.mov")
        );
        // Unknown ref → None / Offline.
        assert_eq!(r.resolve_url("nope"), None);
        assert_eq!(r.display_name("nope"), "Offline");
    }

    #[test]
    fn project_needs_base_to_resolve() {
        let mut m = MediaManifest::new();
        m.entries.push(entry(
            "p1",
            "internal.mov",
            MediaSource::Project {
                relative_path: "media/internal.mov".into(),
            },
        ));
        // No base → unresolvable.
        let r0 = ManifestResolver::new(m.clone());
        assert_eq!(r0.resolve_url("p1"), None);
        // With base → joined and file-URL'd.
        let r = ManifestResolver::with_project_base(m, "/Users/x/proj.palmier");
        assert_eq!(
            r.resolve_url("p1").as_deref(),
            Some("file:///Users/x/proj.palmier/media/internal.mov")
        );
    }

    #[test]
    fn file_url_encodes_space_but_keeps_slashes() {
        assert_eq!(file_url_string("/a/b c.mov"), "file:///a/b%20c.mov");
        assert_eq!(file_url_string("/plain.mov"), "file:///plain.mov");
    }
}
