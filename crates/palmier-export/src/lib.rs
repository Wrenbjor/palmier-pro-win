//! # palmier-export
//!
//! Export subsystem (FOUNDATION §4, §6.12). This crate currently owns the two
//! pure, no-media export stories:
//!
//! - **E6-S1 / E6-S2 / E6-S3 / E6-S4 — FCP7 XMEML 4 emitter** ([`export_xmeml`]):
//!   a pure `Timeline -> String` function reproducing the macOS reference
//!   `XMLExporter` byte-for-byte (golden-test-critical, SM-7). See [`xml`]
//!   (the whitespace-exact render core) and [`xmeml`] (the document builder).
//! - **E6-S7 — `.palmier` self-contained bundle export**
//!   ([`bundle::export_palmier_project`]): collects all media into a portable
//!   `.palmier` directory and rewrites references (FR-23).
//!
//! The video-encode pipeline (E6-S5/S6) and the social sidecar (E6-S8) are
//! later, ffmpeg-gated stories and are intentionally absent here (no ffmpeg
//! dependency in this crate yet).

pub mod bundle;
pub mod resolver;
pub mod xmeml;
pub mod xml;

pub use bundle::{export_palmier_project, ExportError, Missing, Report};
pub use resolver::{file_url_string, ManifestResolver, MediaResolver};
pub use xmeml::{format_timecode, Builder};

use palmier_model::Timeline;

/// Emit the FCP7 XMEML 4 document for `timeline` as a byte-stable string, using
/// `resolver` for media display names / URLs (E6-S1..S4).
///
/// This is the pure `Timeline -> String` entry point the export panel and CLI
/// call; it performs no I/O. The output is asserted byte-exact against committed
/// golden fixtures (`crates/palmier-export/tests/`).
pub fn export_xmeml<R: MediaResolver>(timeline: &Timeline, resolver: &R) -> String {
    Builder::new(timeline, resolver).build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{Clip, ClipType, MediaManifest, MediaManifestEntry, MediaSource, Track};

    fn video_entry(id: &str, name: &str, path: &str) -> MediaManifestEntry {
        MediaManifestEntry {
            id: id.into(),
            name: name.into(),
            asset_type: ClipType::Video,
            source: MediaSource::External {
                absolute_path: path.into(),
            },
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
    fn xmeml_is_well_formed_and_has_prolog() {
        let mut manifest = MediaManifest::new();
        manifest
            .entries
            .push(video_entry("v1", "clip.mov", "/Users/x/clip.mov"));
        let resolver = ManifestResolver::new(manifest);

        let mut tl = Timeline::new();
        let mut track = Track::new(ClipType::Video);
        track.clips.push(Clip::new("v1", 0, 120));
        tl.tracks.push(track);

        let xml = export_xmeml(&tl, &resolver);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n"));
        assert!(xml.contains("<xmeml version=\"4\">"));
        assert!(xml.contains("<sequence id=\"sequence-1\">"));
        // The clipitem id is the clip's id.
        assert!(xml.contains(&format!("<clipitem id=\"clipitem-{}\">", tl.tracks[0].clips[0].id)));
        // pathurl got the localhost rewrite.
        assert!(xml.contains("<pathurl>file://localhost///Users/x/clip.mov</pathurl>"));
    }

    #[test]
    fn unresolvable_clips_are_dropped() {
        // Empty manifest → the clip's media is unresolvable → no clipitem.
        let resolver = ManifestResolver::new(MediaManifest::new());
        let mut tl = Timeline::new();
        let mut track = Track::new(ClipType::Video);
        track.clips.push(Clip::new("missing", 0, 120));
        tl.tracks.push(track);
        let xml = export_xmeml(&tl, &resolver);
        assert!(!xml.contains("<clipitem"));
    }
}
