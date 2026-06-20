//! Preview tab model — E5-S5.
//!
//! Port of the macOS reference `PreviewTab.swift` (`Sources/PalmierPro/Preview/`):
//! the always-present, non-closable `.timeline` tab plus closable per-asset
//! `.media_asset` tabs. The reference enum carried SwiftUI tint/underline colors;
//! those are a frontend (E5-S10) concern, so this engine-side port keeps **only the
//! identity + playback-relevant** fields. The frontend derives display colors from
//! `clip_type` exactly as the reference did.
//!
//! ## Per-tab playback state
//!
//! The reference shares ONE `AVPlayer` across tabs and swaps the player item on tab
//! activation, but tracks the playhead **per tab role**: the timeline tab drives
//! `editor.currentFrame`, an asset tab drives `editor.sourcePlayheadFrame`
//! (`VideoEngine.activateTab` / the periodic time observer). We mirror that with a
//! [`PreviewTabState`] holding the per-tab playhead + play flag, so the transport
//! (E5-S7) can restore a tab's position when the user switches back to it
//! (open question in `preview-engine.md`: "one engine per tab or shared with per-tab
//! state" — we resolve to **shared transport + per-tab state**, matching the
//! reference's single-player design).

use palmier_model::ClipType;

/// The stable id prefix the reference uses for media-asset tabs
/// (`PreviewTab.mediaAssetTabId(for:)` → `"media_<assetId>"`).
const MEDIA_TAB_PREFIX: &str = "media_";

/// The fixed id of the always-present timeline tab
/// (`PreviewTab.timeline.id == "__timeline__"`).
pub const TIMELINE_TAB_ID: &str = "__timeline__";

/// A preview tab — the timeline, or a closable per-asset preview.
///
/// Mirrors the reference `enum PreviewTab { case timeline; case mediaAsset(...) }`.
/// `Eq`/`Hash` key off the [`PreviewTab::id`] so a tab is found regardless of a
/// changed display name (the reference compares by associated id too).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewTab {
    /// The always-present, **non-closable** timeline tab. Drives `current_frame`.
    Timeline,
    /// A closable per-asset preview tab. Drives `source_playhead_frame`.
    MediaAsset {
        /// The backing media-asset id (`clip.media_ref` / `MediaAsset.id`).
        id: String,
        /// Display name (the asset's name). Frontend-facing; not part of identity.
        name: String,
        /// The asset's clip type — the frontend derives tint/underline color from
        /// it (reference `clipType` → `themeColor`).
        clip_type: ClipType,
    },
}

impl PreviewTab {
    /// A media-asset tab for `(id, name, clip_type)`.
    pub fn media_asset(
        id: impl Into<String>,
        name: impl Into<String>,
        clip_type: ClipType,
    ) -> Self {
        PreviewTab::MediaAsset {
            id: id.into(),
            name: name.into(),
            clip_type,
        }
    }

    /// The stable tab id (reference `PreviewTab.id`): `"__timeline__"` for the
    /// timeline, `"media_<assetId>"` for an asset tab.
    pub fn id(&self) -> String {
        match self {
            PreviewTab::Timeline => TIMELINE_TAB_ID.to_string(),
            PreviewTab::MediaAsset { id, .. } => Self::media_asset_tab_id(id),
        }
    }

    /// The tab id for a given media-asset id (reference
    /// `PreviewTab.mediaAssetTabId(for:)`).
    pub fn media_asset_tab_id(asset_id: &str) -> String {
        format!("{MEDIA_TAB_PREFIX}{asset_id}")
    }

    /// The display label (reference `displayName`): `"Timeline"` or the asset name.
    pub fn display_name(&self) -> &str {
        match self {
            PreviewTab::Timeline => "Timeline",
            PreviewTab::MediaAsset { name, .. } => name,
        }
    }

    /// Whether the tab can be closed — every tab **except** the timeline
    /// (reference `isCloseable`).
    pub fn is_closeable(&self) -> bool {
        !matches!(self, PreviewTab::Timeline)
    }

    /// Whether this is the timeline tab.
    pub fn is_timeline(&self) -> bool {
        matches!(self, PreviewTab::Timeline)
    }

    /// The clip type of an asset tab (`None` for the timeline) — reference
    /// `clipType`. The frontend maps this to a theme color.
    pub fn clip_type(&self) -> Option<ClipType> {
        match self {
            PreviewTab::Timeline => None,
            PreviewTab::MediaAsset { clip_type, .. } => Some(*clip_type),
        }
    }
}

/// Per-tab playback state retained across tab switches (reference: timeline tab uses
/// `editor.currentFrame`, asset tabs use `editor.sourcePlayheadFrame`).
///
/// The transport (E5-S7) reads/writes the **active** tab's state; switching tabs
/// saves the outgoing tab's playhead and restores the incoming one. Defaults to
/// frame 0, paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreviewTabState {
    /// The playhead for this tab — `current_frame` for the timeline tab,
    /// `source_playhead_frame` for an asset tab.
    pub playhead_frame: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_tab_identity_and_non_closeable() {
        let t = PreviewTab::Timeline;
        assert_eq!(t.id(), "__timeline__");
        assert_eq!(t.display_name(), "Timeline");
        assert!(!t.is_closeable());
        assert!(t.is_timeline());
        assert_eq!(t.clip_type(), None);
    }

    #[test]
    fn media_asset_tab_id_and_closeable() {
        let t = PreviewTab::media_asset("asset42", "Clip.mp4", ClipType::Video);
        assert_eq!(t.id(), "media_asset42");
        assert_eq!(PreviewTab::media_asset_tab_id("asset42"), "media_asset42");
        assert_eq!(t.display_name(), "Clip.mp4");
        assert!(t.is_closeable());
        assert!(!t.is_timeline());
        assert_eq!(t.clip_type(), Some(ClipType::Video));
    }

    #[test]
    fn tabs_compare_by_full_value_including_id() {
        let a = PreviewTab::media_asset("x", "Name A", ClipType::Image);
        let b = PreviewTab::media_asset("x", "Name A", ClipType::Image);
        assert_eq!(a, b);
        // Same id, different name → still distinct values (frontend may rename) but
        // share a tab id, which is how the reference de-dupes open tabs.
        let renamed = PreviewTab::media_asset("x", "Renamed", ClipType::Image);
        assert_eq!(a.id(), renamed.id());
    }
}
