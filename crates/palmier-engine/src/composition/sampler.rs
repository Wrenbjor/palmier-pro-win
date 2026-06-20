//! Per-frame layer-property sampling — E5-S4.
//!
//! Verbatim port of the macOS reference `CompositionBuilder` visual-instruction
//! math (`affineTransform(for:natSize:renderSize:)`, `emitTransform`, `emitCrop`,
//! `emitOpacity`), specialized to the **per-frame sampling** model.
//!
//! ## Why per-frame sampling needs no ramp pre-bake
//!
//! The reference pre-bakes ramps because AVFoundation interpolates **linearly**
//! between layer-instruction times, so a smooth curve has to be approximated by
//! subdividing each segment into `smoothSegments = 8` linear ramps
//! (`CompositionBuilder.smoothSubdivisions`). Our compositor samples one value
//! **per output frame**, so we can sample the *true* curve directly — and because
//! `palmier_model::KeyframeTrack::sample` evaluates the **same** `smoothstep`
//! (`t·t·(3−2t)`) the reference's 8-segment pre-bake approximates, a frame sampled
//! here is fidelity-equivalent to the reference's baked ramp at that frame
//! (preview-engine.md risk #4 / SM-C1). The `smooth_8segment_parity` test pins this
//! by reconstructing the reference's piecewise-linear ramp and asserting our
//! direct sample matches at the 8 segment boundaries.
//!
//! ## What the sampler needs that the model can't give
//!
//! `affine_transform` and `crop_rect` depend on the **source** `natural_size` +
//! `preferred_transform`, which come from the decoder, not the timeline model. The
//! pure build can't decode, so the caller supplies a [`SourceInfo`] per `media_ref`
//! (E5-S8 wires the real decoder metadata; tests pass it directly). This keeps the
//! sampler pure and presentation-agnostic.

use palmier_model::{Clip, Crop, Transform};

use super::mat3::Mat3;
use super::types::CropRect;

/// Source-frame geometry the sampler needs but the timeline model doesn't carry:
/// the decoded frame's natural (display) size and its `preferred_transform`, both
/// after the reference's bbox re-origin (reference `clipNaturalSizes` /
/// `clipTransforms`, see `build.rs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceInfo {
    /// Natural display size in pixels (`|bbox of natSize·preferredTransform|`).
    pub natural_size: (f64, f64),
    /// The source's preferred transform, already re-origined so the bbox sits at
    /// `(0, 0)` (reference `pt.concatenating(translation(-box.minX, -box.minY))`).
    pub preferred_transform: Mat3,
}

impl SourceInfo {
    /// A source with the given natural size and an **identity** preferred transform
    /// (the common case — most decoded frames need no display-matrix correction).
    pub fn upright(natural_size: (f64, f64)) -> Self {
        SourceInfo {
            natural_size,
            preferred_transform: Mat3::IDENTITY,
        }
    }
}

/// Map a clip's normalized `0–1` canvas [`Transform`] to the render-pixel affine,
/// **verbatim** from the reference `affineTransform(for:natSize:renderSize:)`.
///
/// ```text
/// sx = (renderW/natW) · t.width  · (flipH ? -1 : 1)
/// sy = (renderH/natH) · t.height · (flipV ? -1 : 1)
/// tx = (flipH ? tl.x + t.width : tl.x) · renderW
/// ty = (flipV ? tl.y + t.height : tl.y) · renderH
/// placed = scale(sx, sy) ∘ translation(tx, ty)
/// // rotation about (centerX·renderW, centerY·renderH):
/// placed ∘ translation(-c) ∘ rotation(deg·π/180) ∘ translation(c)
/// ```
///
/// `tl` is the clip transform's computed top-left (center-based storage, ruling #7).
pub fn affine_transform(t: &Transform, nat_size: (f64, f64), render_size: (f64, f64)) -> Mat3 {
    let (nat_w, nat_h) = nat_size;
    let (render_w, render_h) = render_size;
    let (tlx, tly) = t.top_left();

    let sx = (render_w / nat_w) * t.width * if t.flip_horizontal { -1.0 } else { 1.0 };
    let sy = (render_h / nat_h) * t.height * if t.flip_vertical { -1.0 } else { 1.0 };
    let tx = (if t.flip_horizontal { tlx + t.width } else { tlx }) * render_w;
    let ty = (if t.flip_vertical { tly + t.height } else { tly }) * render_h;

    let placed = Mat3::scale(sx, sy).concatenating(Mat3::translation(tx, ty));
    if t.rotation == 0.0 {
        return placed;
    }
    let cx = t.center_x * render_w;
    let cy = t.center_y * render_h;
    let radians = t.rotation * std::f64::consts::PI / 180.0;
    placed
        .concatenating(Mat3::translation(-cx, -cy))
        .concatenating(Mat3::rotation(radians))
        .concatenating(Mat3::translation(cx, cy))
}

/// The full layer transform for `clip` at absolute timeline `frame`:
/// `preferred_transform ∘ affine(clip.transform_at(frame))` (reference
/// `emitTransform`'s `affine` closure). `clip.transform_at` already folds the
/// position/scale/rotation keyframe tracks with the correct interpolation
/// (Hold/Linear/Smooth) via the model sampler.
pub fn layer_transform(
    clip: &Clip,
    frame: i32,
    source: &SourceInfo,
    render_size: (f64, f64),
) -> Mat3 {
    let t = clip.transform_at(frame);
    let affine = affine_transform(&t, source.natural_size, render_size);
    source.preferred_transform.concatenating(affine)
}

/// The visible crop rectangle in **source pixels** for `clip` at `frame`, verbatim
/// from the reference `emitCrop`:
///
/// ```text
/// rect = (left·natW, top·natH, max(1, visW·natW), max(1, visH·natH))
///          .applying(preferred_transform.inverted())
/// ```
///
/// `clip.crop_at` folds the crop keyframe track. When the preferred transform is
/// singular (degenerate), we fall back to the untransformed rect (CG's
/// `inverted()` returns the input unchanged in that case).
pub fn crop_rect(clip: &Clip, frame: i32, source: &SourceInfo) -> CropRect {
    let cp: Crop = clip.crop_at(frame);
    let (nat_w, nat_h) = source.natural_size;
    let x = cp.left * nat_w;
    let y = cp.top * nat_h;
    let w = (cp.visible_width_fraction() * nat_w).max(1.0);
    let h = (cp.visible_height_fraction() * nat_h).max(1.0);

    let to_source = source.preferred_transform.inverted();
    apply_to_rect(to_source, x, y, w, h)
}

/// Apply an affine to an axis-aligned rect, returning the axis-aligned bbox of the
/// transformed corners (matches CG `CGRect.applying(_:)`). `None` transform → the
/// rect unchanged.
fn apply_to_rect(m: Option<Mat3>, x: f64, y: f64, w: f64, h: f64) -> CropRect {
    let Some(m) = m else {
        return CropRect {
            x,
            y,
            width: w,
            height: h,
        };
    };
    let corners = [
        m.apply(x, y),
        m.apply(x + w, y),
        m.apply(x, y + h),
        m.apply(x + w, y + h),
    ];
    let min_x = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let max_x = corners.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    let max_y = corners.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);
    CropRect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// The effective opacity for `clip` at absolute timeline `frame`, in `[0, 1]`.
///
/// `clip.opacity_at` already folds static opacity × opacity-track keyframes × fade
/// envelope (reference `opacityAt`, which calls `rawOpacityAt` × `fadeMultiplier`,
/// and only applies the fade for non-audio clips). We clamp to `[0, 1]` and treat a
/// non-finite sample as fully transparent, matching the reference
/// `normalizedOpacity` (`isFinite ? clamp(0,1) : drop`).
pub fn layer_opacity(clip: &Clip, frame: i32) -> f64 {
    let o = clip.opacity_at(frame);
    if o.is_finite() {
        o.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{AnimPair, Interpolation, Keyframe, KeyframeTrack};

    const RENDER: (f64, f64) = (1920.0, 1080.0);

    fn identity_source() -> SourceInfo {
        SourceInfo::upright((1920.0, 1080.0))
    }

    #[test]
    fn affine_full_canvas_identity_maps_to_render_box() {
        // A full-canvas (default) transform on a render-sized source: scale 1,
        // translate to (0,0). A point at the source's bottom-right (natW, natH)
        // lands at (renderW, renderH).
        let t = Transform::default();
        let m = affine_transform(&t, (1920.0, 1080.0), RENDER);
        let (x, y) = m.apply(1920.0, 1080.0);
        assert!((x - 1920.0).abs() < 1e-6, "x={x}");
        assert!((y - 1080.0).abs() < 1e-6, "y={y}");
        let (ox, oy) = m.apply(0.0, 0.0);
        assert!(ox.abs() < 1e-6 && oy.abs() < 1e-6);
    }

    #[test]
    fn affine_half_size_top_left_quadrant() {
        // Clip occupying the top-left quadrant: width/height 0.5, top-left (0,0)
        // (so center 0.25,0.25). sx = (1920/1920)*0.5 = 0.5; tx = 0*1920 = 0.
        let t = Transform {
            center_x: 0.25,
            center_y: 0.25,
            width: 0.5,
            height: 0.5,
            ..Transform::default()
        };
        let m = affine_transform(&t, (1920.0, 1080.0), RENDER);
        // The source's full extent (1920,1080) scales by 0.5 → (960,540), then
        // translates by tl*(render) = (0,0).
        let (x, y) = m.apply(1920.0, 1080.0);
        assert!((x - 960.0).abs() < 1e-6, "x={x}");
        assert!((y - 540.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn affine_flip_horizontal_negates_sx_and_shifts_tx() {
        // flipH: sx negative; tx uses (tl.x + width).
        let t = Transform {
            center_x: 0.5,
            center_y: 0.5,
            width: 1.0,
            height: 1.0,
            flip_horizontal: true,
            ..Transform::default()
        };
        let m = affine_transform(&t, (1920.0, 1080.0), RENDER);
        // sx = -(1920/1920)*1 = -1; tl.x = 0, so tx = (0 + 1)*1920 = 1920.
        assert!((m.a - -1.0).abs() < 1e-9, "a={}", m.a);
        assert!((m.tx - 1920.0).abs() < 1e-6, "tx={}", m.tx);
        // A source point at x=0 maps to render x=1920 (mirrored), x=1920 → 0.
        assert!((m.apply(0.0, 0.0).0 - 1920.0).abs() < 1e-6);
        assert!((m.apply(1920.0, 0.0).0 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn affine_rotation_about_center_keeps_center_fixed() {
        // A 90° rotation about the canvas center must leave the center point fixed.
        let t = Transform {
            center_x: 0.5,
            center_y: 0.5,
            width: 1.0,
            height: 1.0,
            rotation: 90.0,
            ..Transform::default()
        };
        let m = affine_transform(&t, (1920.0, 1080.0), RENDER);
        // The render-space center is (0.5*1920, 0.5*1080) = (960, 540). Under the
        // un-rotated `placed` the source center (960,540) maps to (960,540); after
        // rotation about that same point it stays put.
        let (x, y) = m.apply(960.0, 540.0);
        assert!((x - 960.0).abs() < 1e-6, "x={x}");
        assert!((y - 540.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn preferred_transform_is_pre_concatenated() {
        // layer_transform = preferred ∘ affine. With a non-identity preferred
        // (a 2× scale) the result differs from affine alone.
        let mut clip = Clip::new("m", 0, 100);
        clip.transform = Transform::default();
        let src = SourceInfo {
            natural_size: (1920.0, 1080.0),
            preferred_transform: Mat3::scale(2.0, 2.0),
        };
        let m = layer_transform(&clip, 50, &src, RENDER);
        // affine for default is identity-ish (scale 1, translate 0); preferred ∘ id
        // = preferred → a point (100,100) → (200,200).
        let (x, y) = m.apply(100.0, 100.0);
        assert!((x - 200.0).abs() < 1e-6 && (y - 200.0).abs() < 1e-6, "{x},{y}");
    }

    #[test]
    fn crop_rect_in_source_pixels() {
        // left 0.1, right 0.3 → visW 0.6; top 0.2, bottom 0.1 → visH 0.7.
        let mut clip = Clip::new("m", 0, 100);
        clip.crop = Crop {
            left: 0.1,
            top: 0.2,
            right: 0.3,
            bottom: 0.1,
        };
        let src = identity_source();
        let r = crop_rect(&clip, 50, &src);
        assert!((r.x - 0.1 * 1920.0).abs() < 1e-6, "x={}", r.x);
        assert!((r.y - 0.2 * 1080.0).abs() < 1e-6, "y={}", r.y);
        assert!((r.width - 0.6 * 1920.0).abs() < 1e-6, "w={}", r.width);
        assert!((r.height - 0.7 * 1080.0).abs() < 1e-6, "h={}", r.height);
    }

    #[test]
    fn crop_rect_clamps_to_min_one_pixel() {
        // Over-crop → visible fraction 0 → width/height clamp to 1px (reference max(1, …)).
        let mut clip = Clip::new("m", 0, 100);
        clip.crop = Crop {
            left: 0.6,
            top: 0.0,
            right: 0.6,
            bottom: 0.0,
        };
        let r = crop_rect(&clip, 0, &identity_source());
        assert!((r.width - 1.0).abs() < 1e-9, "w={}", r.width);
    }

    #[test]
    fn opacity_folds_fade_and_clamps() {
        // Visual clip, linear fade-in over 10 frames: at rel 5 → 0.5.
        let mut clip = Clip::new("m", 0, 100);
        clip.fade_in_frames = 10;
        clip.fade_in_interpolation = Interpolation::Linear;
        assert!((layer_opacity(&clip, 5) - 0.5).abs() < 1e-9);
        // Static opacity > 1 would clamp (defensive); set raw opacity to 2.0.
        let mut over = Clip::new("m", 0, 100);
        over.opacity = 2.0;
        assert!((layer_opacity(&over, 50) - 1.0).abs() < 1e-9);
    }

    /// SM-C1 parity: our per-frame Smooth sample must equal the reference's
    /// 8-segment pre-baked piecewise-linear ramp at each segment boundary.
    #[test]
    fn smooth_8segment_parity() {
        // An opacity track 0→1 over [0,80] with Smooth interpolation. The reference
        // subdivides into 8 segments and emits linear ramps whose *endpoints* are
        // the true smoothstep values — so AT every boundary frame the baked value
        // equals our direct sample. Verify our sampler hits those boundary values.
        let mut track = KeyframeTrack::new();
        track.upsert(Keyframe::with_interpolation(0, 0.0_f64, Interpolation::Smooth));
        track.upsert(Keyframe::with_interpolation(80, 1.0_f64, Interpolation::Smooth));
        let mut clip = Clip::new("m", 0, 80);
        clip.opacity_track = Some(track);

        let smooth_segments = 8;
        for s in 0..=smooth_segments {
            // Reference boundary offset = round(span * s/8).
            let t = s as f64 / smooth_segments as f64;
            let offset = (80.0 * t).round() as i32;
            // Reference's baked endpoint value at this boundary = smoothstep(t)
            // (lerp(0,1, smoothstep(t))).
            let expected = palmier_model::smoothstep(t);
            let sampled = layer_opacity(&clip, offset);
            assert!(
                (sampled - expected).abs() < 1e-9,
                "segment {s}: offset {offset} sampled {sampled} != reference {expected}"
            );
        }
    }

    #[test]
    fn transform_tracks_folded_via_model_sampler() {
        // An active scale track animates width 0.5→1.0 over [0,100] (Linear). At
        // frame 50 width should be 0.75, so sx = 0.75.
        let mut clip = Clip::new("m", 0, 100);
        clip.transform = Transform {
            center_x: 0.5,
            center_y: 0.5,
            width: 0.5,
            height: 0.5,
            ..Transform::default()
        };
        let mut scale = KeyframeTrack::new();
        scale.upsert(Keyframe::with_interpolation(0, AnimPair::new(0.5, 0.5), Interpolation::Linear));
        scale.upsert(Keyframe::with_interpolation(100, AnimPair::new(1.0, 1.0), Interpolation::Linear));
        clip.scale_track = Some(scale);

        let m = layer_transform(&clip, 50, &identity_source(), RENDER);
        // sx = (1920/1920) * 0.75 = 0.75.
        assert!((m.a - 0.75).abs() < 1e-9, "a={}", m.a);
    }
}
