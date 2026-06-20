//! Pure export-spec types + math — E6-S5 (always compiled, GPU/FFmpeg-free).
//!
//! These are the load-bearing decisions the reference encodes in
//! `ExportService` (`ExportFormat`, `ExportResolution`, `renderSize`, the
//! frame-count loop bound, the BT.709 color tags) lifted to pure Rust so they
//! unit-test without a device. The `render` module consumes them to drive the
//! actual encode.

/// The output codec. `Xml` is **not** here — that's the XMEML emitter
/// (`export_xmeml`), a separate, no-media path. This enum is the *video* codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// H.264 in an MP4 container (`.mp4`). Hardware encoder only (LGPL build has
    /// no libx264). Reference preset family `AVAssetExportPreset…`.
    H264,
    /// H.265 / HEVC in an MP4 container (`.mp4`). Hardware encoder only.
    H265,
    /// ProRes **422 LPCM** in a QuickTime container (`.mov`) via `prores_ks`
    /// (LGPL-clean). Ruling #17 — no 4444/alpha for v1.
    ProRes422,
}

impl ExportFormat {
    /// The output container file extension (no dot): `mp4` for H.264/H.265,
    /// `mov` for ProRes (reference: ProRes ships in a `.mov`).
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::H264 | ExportFormat::H265 => "mp4",
            ExportFormat::ProRes422 => "mov",
        }
    }

    /// The FFmpeg muxer/format short name for this container.
    pub fn muxer(self) -> &'static str {
        match self {
            ExportFormat::H264 | ExportFormat::H265 => "mp4",
            ExportFormat::ProRes422 => "mov",
        }
    }

    /// A human label for diagnostics / the `NoHardwareEncoder` error.
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::H264 => "H.264",
            ExportFormat::H265 => "H.265",
            ExportFormat::ProRes422 => "ProRes 422",
        }
    }

    /// Whether this format **requires** a hardware encoder in the LGPL build
    /// (H.264/H.265 do; ProRes uses the always-present `prores_ks`).
    pub fn requires_hw_encoder(self) -> bool {
        matches!(self, ExportFormat::H264 | ExportFormat::H265)
    }
}

/// Export resolution presets, keyed by **short-side pixels** (reference
/// `ExportResolution {r720p=720, r1080p=1080, r4k=2160}`). The actual encoded
/// dimensions come from [`render_size`], which scales the project canvas so its
/// short side hits this value and snaps both dims to even.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportResolution {
    /// 720p — short side 720 px.
    P720,
    /// 1080p — short side 1080 px.
    P1080,
    /// 4K — short side 2160 px.
    P2160,
}

impl ExportResolution {
    /// The target **short-side** pixel count for this preset.
    pub fn short_side_px(self) -> u32 {
        match self {
            ExportResolution::P720 => 720,
            ExportResolution::P1080 => 1080,
            ExportResolution::P2160 => 2160,
        }
    }
}

/// Compute the even-snapped encode dimensions for a project canvas of
/// `canvas_w × canvas_h` at the requested `resolution`.
///
/// Reproduces the reference `ExportResolution.renderSize(canvas)` exactly
/// (`docs/reference/export.md` §A.1):
/// `scale = shortSidePx / min(w, h)`; each dim `= (round(dim·scale)/2)·2`
/// (snap to **even** — encoders reject odd dims), with a **minimum of 2**.
///
/// `round` is `f64::round` (ties-away), matching Swift `.rounded()`. The even
/// snap is `(round(v) / 2).floor() * 2`, i.e. round to nearest then drop to the
/// even number at/below it (matching the reference integer `/2*2`).
pub fn render_size(canvas_w: u32, canvas_h: u32, resolution: ExportResolution) -> (u32, u32) {
    let w = canvas_w.max(1) as f64;
    let h = canvas_h.max(1) as f64;
    let short = resolution.short_side_px() as f64;
    let scale = short / w.min(h);

    let snap = |dim: f64| -> u32 {
        let rounded = (dim * scale).round();
        // Integer divide by 2 then ×2 → the even number at/below `rounded`.
        let even = ((rounded as i64) / 2) * 2;
        even.max(2) as u32
    };
    (snap(w), snap(h))
}

/// Total output frames the per-frame loop must encode.
///
/// FOUNDATION §6.12 / `docs/reference/export.md` "Mapping": the loop runs
/// `0..total_frames · (output_fps / project_fps)`. For v1 `output_fps ==
/// project_fps` (variable output fps is out of scope — Open Questions), so this
/// is just `total_frames` clamped to ≥ 0. Kept as a function (not a field) so a
/// later variable-fps change has one place to touch.
///
/// Returns `0` when the timeline is empty (the pipeline then reports
/// [`ExportError::Empty`](crate::video::ExportError::Empty)).
pub fn frame_count(total_frames: i32, project_fps: u32, output_fps: u32) -> u64 {
    if total_frames <= 0 || project_fps == 0 {
        return 0;
    }
    let scale = if output_fps == 0 || output_fps == project_fps {
        1.0
    } else {
        output_fps as f64 / project_fps as f64
    };
    let scaled = (total_frames as f64 * scale).round();
    if scaled <= 0.0 {
        0
    } else {
        scaled as u64
    }
}

/// BT.709 color tags applied to the encoded video stream (reference
/// `AVVideoComposition` color tags: BT.709 primaries / transfer / YCbCr matrix).
/// Values are the FFmpeg/ITU enum codes so the `render` module can set them on
/// the codec context without re-deriving them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTags {
    /// Color primaries (ITU-T H.273) — BT.709 = 1.
    pub primaries: i32,
    /// Transfer characteristics — BT.709 = 1.
    pub transfer: i32,
    /// YCbCr matrix coefficients — BT.709 = 1.
    pub matrix: i32,
}

/// The single BT.709 working-space color tag set (risk #5: one working space).
pub const BT709: ColorTags = ColorTags {
    primaries: 1,
    transfer: 1,
    matrix: 1,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_muxers_match_container() {
        assert_eq!(ExportFormat::H264.extension(), "mp4");
        assert_eq!(ExportFormat::H265.extension(), "mp4");
        assert_eq!(ExportFormat::ProRes422.extension(), "mov");
        assert_eq!(ExportFormat::ProRes422.muxer(), "mov");
    }

    #[test]
    fn prores_needs_no_hw_encoder_but_h26x_do() {
        assert!(!ExportFormat::ProRes422.requires_hw_encoder());
        assert!(ExportFormat::H264.requires_hw_encoder());
        assert!(ExportFormat::H265.requires_hw_encoder());
    }

    #[test]
    fn render_size_1080p_landscape_16x9() {
        // 1920×1080 canvas → 1080p: short side 1080 already, scale 1.0.
        assert_eq!(render_size(1920, 1080, ExportResolution::P1080), (1920, 1080));
        // 720p: scale = 720/1080 = 0.6666… → 1920·.6666=1280, 1080·.6666=720.
        assert_eq!(render_size(1920, 1080, ExportResolution::P720), (1280, 720));
        // 4K: scale = 2160/1080 = 2 → 3840×2160.
        assert_eq!(render_size(1920, 1080, ExportResolution::P2160), (3840, 2160));
    }

    #[test]
    fn render_size_portrait_keys_off_short_side() {
        // 1080×1920 portrait → 1080p means short side (1080, the width) stays.
        assert_eq!(render_size(1080, 1920, ExportResolution::P1080), (1080, 1920));
        // 720p portrait → width 720, height 1280.
        assert_eq!(render_size(1080, 1920, ExportResolution::P720), (720, 1280));
    }

    #[test]
    fn render_size_snaps_to_even_min_two() {
        // An odd canvas that would land on an odd dim must snap to even.
        // 641×481, target short side 481 → scale 1.0 → 641 must drop to 640.
        let (w, h) = render_size(641, 481, ExportResolution::P1080);
        assert_eq!(w % 2, 0, "width even");
        assert_eq!(h % 2, 0, "height even");
        // Tiny canvas can't go below 2.
        let (w, h) = render_size(1, 1, ExportResolution::P720);
        assert!(w >= 2 && h >= 2);
    }

    #[test]
    fn frame_count_v1_is_total_frames() {
        // Equal fps → 1:1.
        assert_eq!(frame_count(1800, 30, 30), 1800);
        // output_fps 0 (unspecified) → treat as project fps.
        assert_eq!(frame_count(1800, 30, 0), 1800);
        // Empty timeline → 0.
        assert_eq!(frame_count(0, 30, 30), 0);
        assert_eq!(frame_count(-5, 30, 30), 0);
        // Zero fps guarded.
        assert_eq!(frame_count(100, 0, 0), 0);
    }

    #[test]
    fn frame_count_scales_with_output_fps() {
        // 60 fps out of a 30 fps project doubles the frame count (future path).
        assert_eq!(frame_count(100, 30, 60), 200);
        // 24 out of 30 → round(100 * 0.8) = 80.
        assert_eq!(frame_count(100, 30, 24), 80);
    }

    #[test]
    fn one_minute_1080p_30fps_is_1800_frames() {
        // SM-5 reference: 1 min @ 30 fps = 1800 frames.
        assert_eq!(frame_count(60 * 30, 30, 30), 1800);
    }

    #[test]
    fn bt709_tags_are_all_one() {
        assert_eq!(BT709.primaries, 1);
        assert_eq!(BT709.transfer, 1);
        assert_eq!(BT709.matrix, 1);
    }
}
