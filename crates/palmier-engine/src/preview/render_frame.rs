//! Render-ready frame description — E5-S5.
//!
//! The seam between the composition graph (E5-S3/S4 [`CompositionFrame`]) and the
//! GPU present (E5-S8). [`build_frame`](crate::build_frame) emits the *structural*
//! layer stack; this story **finalizes** it into a [`RenderFrame`]: the
//! `CompositionFrame` plus the canvas geometry (render size / aspect) and the
//! [`QualityTarget`] the compositor renders at. E5-S8 consumes a `RenderFrame` and
//! does the actual wgpu draw + present; it does **not** re-derive canvas geometry.
//!
//! ## Why a separate finalize step
//!
//! A `CompositionFrame` is presentation-agnostic — it knows *which* layers exist and
//! their sampled transforms in **render-pixel** space, but a transform in render
//! pixels is only meaningful against a known canvas size. The reference carried this
//! implicitly via `renderSize` (the timeline `width × height`) inside
//! `CompositionBuilder`; here we surface it explicitly so the compositor and the
//! viewport overlays (E5-S10) agree on one canvas rect. The [`QualityTarget`] lets
//! the transport drop to a smaller backing resolution during an interactive scrub
//! (the reference relied on AVFoundation's internal downscaling; we make it an
//! explicit knob the transport sets per `SeekMode`).
//!
//! This module owns **descriptors only** — no wgpu, no device, no pixels.

use crate::composition::CompositionFrame;

/// The canvas geometry a [`RenderFrame`] is composited into: the timeline's
/// render size in pixels. The black background (the opaque floor) fills this rect;
/// every layer's [`Mat3`](crate::Mat3) maps source pixels into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    /// Render width in pixels (timeline `width`).
    pub width: u32,
    /// Render height in pixels (timeline `height`).
    pub height: u32,
}

impl Canvas {
    /// A canvas of `width × height` render pixels.
    pub fn new(width: u32, height: u32) -> Self {
        Canvas { width, height }
    }

    /// Aspect ratio `width / height` (the viewport letterboxes to this). `0.0` for a
    /// degenerate zero-height canvas (caller guards before dividing).
    pub fn aspect(&self) -> f64 {
        if self.height == 0 {
            0.0
        } else {
            self.width as f64 / self.height as f64
        }
    }
}

/// The backing resolution the compositor should render at for this frame.
///
/// `Full` renders at the canvas size (playback start, frame-step, [`SeekMode::Exact`]
/// targets). During an interactive scrub the transport may request `Scaled` to keep
/// the SM-2 FPS floor, rendering at `canvas × scale` and letting the present stage
/// upscale — an explicit version of AVFoundation's implicit scrub downscaling. The
/// compositor still composites the same layers; only the target backing size changes.
///
/// [`SeekMode`]: palmier_media::SeekMode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QualityTarget {
    /// Render at the full canvas resolution (exact seeks, playback, frame-step).
    Full,
    /// Render at `scale × canvas` (interactive scrub); `scale` is clamped to
    /// `(0, 1]`. The present stage upscales to the viewport.
    Scaled(f64),
}

impl QualityTarget {
    /// The effective scale factor in `(0, 1]`: `1.0` for [`QualityTarget::Full`],
    /// the clamped `scale` for [`QualityTarget::Scaled`].
    pub fn scale(self) -> f64 {
        match self {
            QualityTarget::Full => 1.0,
            QualityTarget::Scaled(s) => s.clamp(f64::MIN_POSITIVE, 1.0),
        }
    }

    /// The backing pixel size the compositor allocates for `canvas`, applying the
    /// quality scale and clamping each dimension to ≥ 1 px.
    pub fn backing_size(self, canvas: Canvas) -> (u32, u32) {
        let s = self.scale();
        let w = ((canvas.width as f64 * s).round() as u32).max(1);
        let h = ((canvas.height as f64 * s).round() as u32).max(1);
        (w, h)
    }
}

/// A fully-finalized frame the wgpu compositor (E5-S8) renders and presents: the
/// composition layer stack plus the canvas it composites into and the quality
/// target to render at.
///
/// This is the **render-frame description** the transport (E5-S7) emits per visible
/// frame ("render this") alongside the `current_frame` change. It is
/// presentation-agnostic: no wgpu handles, no pixels — E5-S8 resolves each layer's
/// [`FrameRef`](crate::FrameRef) through `palmier-media` and draws.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFrame {
    /// The bottom→top composition layer stack for this frame.
    pub composition: CompositionFrame,
    /// The canvas geometry (render size / aspect) the layers map into.
    pub canvas: Canvas,
    /// The backing resolution to render at (full vs. scrub-scaled).
    pub quality: QualityTarget,
}

impl RenderFrame {
    /// Finalize a [`CompositionFrame`] into a render-ready frame for `canvas` at the
    /// given `quality`.
    pub fn new(composition: CompositionFrame, canvas: Canvas, quality: QualityTarget) -> Self {
        RenderFrame {
            composition,
            canvas,
            quality,
        }
    }

    /// The timeline frame index this render-frame depicts (mirrors the inner
    /// [`CompositionFrame::frame_index`]).
    pub fn frame_index(&self) -> i32 {
        self.composition.frame_index
    }

    /// The backing pixel size the compositor should allocate (canvas × quality scale).
    pub fn backing_size(&self) -> (u32, u32) {
        self.quality.backing_size(self.canvas)
    }

    /// Whether this frame has no visible layers (the compositor just clears to
    /// black — reference's opaque floor).
    pub fn is_black(&self) -> bool {
        self.composition.layers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_aspect_and_zero_guard() {
        assert!((Canvas::new(1920, 1080).aspect() - 16.0 / 9.0).abs() < 1e-12);
        assert_eq!(Canvas::new(100, 0).aspect(), 0.0);
    }

    #[test]
    fn quality_full_is_unit_scale() {
        assert_eq!(QualityTarget::Full.scale(), 1.0);
        assert_eq!(QualityTarget::Full.backing_size(Canvas::new(1920, 1080)), (1920, 1080));
    }

    #[test]
    fn quality_scaled_clamps_and_floors_to_one_px() {
        // 0.5 scale halves the backing size.
        let q = QualityTarget::Scaled(0.5);
        assert_eq!(q.backing_size(Canvas::new(1920, 1080)), (960, 540));
        // A scale > 1 clamps to 1.0 (never upscale the backing target).
        assert_eq!(QualityTarget::Scaled(4.0).scale(), 1.0);
        // A tiny canvas at a tiny scale still allocates ≥ 1 px.
        assert_eq!(QualityTarget::Scaled(0.001).backing_size(Canvas::new(1, 1)), (1, 1));
    }

    #[test]
    fn render_frame_exposes_frame_index_and_blackness() {
        let cf = CompositionFrame::empty(7);
        let rf = RenderFrame::new(cf, Canvas::new(1920, 1080), QualityTarget::Full);
        assert_eq!(rf.frame_index(), 7);
        assert!(rf.is_black(), "no layers → black floor");
        assert_eq!(rf.backing_size(), (1920, 1080));
    }
}
