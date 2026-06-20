//! Composition descriptor types — the output of the E5-S3 build (`CompositionFrame`
//! / `LayerRender`) and the value types they carry.
//!
//! These are **pure descriptors**: the composition graph references a decoded frame
//! by `(media_ref, source_frame)` ([`FrameRef`]) but never owns a `wgpu::Texture`
//! (textures + the GPU device land with E5-S8, reconciliation #22/#23). The wgpu
//! compositor (E5-S8) consumes a [`CompositionFrame`], fetches each [`FrameRef`]'s
//! pixels from `palmier-media`'s `FrameCache`, uploads them, and renders the layers
//! bottom→top with each layer's [`Mat3`] affine, opacity, and [`CropRect`].

use super::mat3::Mat3;

/// A reference to a decoded source frame, addressable in `palmier-media`'s
/// `FrameCache` by `(media_ref, source_frame)`.
///
/// The composition build (E5-S3) is the **pure assembly** — it computes which
/// source frame each visible clip needs (timeline→source mapping) and records this
/// handle. E5-S8 calls `FrameSource::request_frame(media_ref, source_frame, …)` to
/// get the actual pixels. Stills/Lottie are first-class (no `.mov` bake, #22): they
/// carry a `source_frame` of `0` (a still has one frame; Lottie is pre-rendered to
/// a texture by `palmier-media`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameRef {
    /// The manifest id of the backing media asset (`clip.media_ref`).
    pub media_ref: String,
    /// The source frame index to decode (the cache key's frame component). `0` for
    /// stills and Lottie.
    pub source_frame: u64,
}

impl FrameRef {
    /// A frame reference for `(media_ref, source_frame)`.
    pub fn new(media_ref: impl Into<String>, source_frame: u64) -> Self {
        FrameRef {
            media_ref: media_ref.into(),
            source_frame,
        }
    }
}

/// A crop rectangle in **source pixels** (reference `emitCrop` `rect`), already
/// mapped through the source's `preferred_transform.inverted()` so it is expressed
/// in the decoded frame's own pixel coordinate space.
///
/// `(x, y)` is the top-left of the visible region; `width`/`height` are its extent
/// (clamped to `≥ 1` like the reference). An identity crop covers the whole frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl CropRect {
    /// A crop covering the full `width × height` source frame (no crop), origin
    /// at `(0, 0)`.
    pub fn full(width: f64, height: f64) -> Self {
        CropRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }
}

/// One renderable layer in a [`CompositionFrame`], in the reference's
/// `AVVideoCompositionLayerInstruction` role: a source frame plus the per-frame
/// **sampled** transform / opacity / crop the compositor applies.
///
/// Variants mirror FOUNDATION §6.5's `LayerRender` enum
/// (Video / Image / Text / Lottie). Video/Image/Lottie carry a [`FrameRef`]; the
/// `Text` variant is a placeholder the text pass (E5-S9) fills with glyph runs —
/// the E5-S3 builder **excludes `.text` clips** from video layering exactly as the
/// reference does (text renders via the overlay pass, never as a composition
/// track), so the builder never emits `Text` itself this wave.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerRender {
    /// A decoded video frame.
    Video(VisualLayer),
    /// A still image decoded straight to a texture (first-class, no `.mov` bake, #22).
    Image(VisualLayer),
    /// A Lottie animation pre-rendered to a texture (first-class, #22).
    Lottie(VisualLayer),
    /// A text layer — shaped by `palmier-text` (E5-S9, cosmic-text glyph runs) and
    /// rasterized in the wgpu text pass. The E5-S3 video builder ([`build_frame`])
    /// still **excludes** `.text` clips from video layering (matching the
    /// reference); text layers are produced separately by
    /// [`build_text_layers`](crate::build_text_layers) with the 30-frame preroll,
    /// then appended on top of the video stack by the compositor.
    Text(TextLayer),
}

impl LayerRender {
    /// The common visual payload for the texture-backed variants
    /// (Video/Image/Lottie); `None` for `Text`.
    pub fn visual(&self) -> Option<&VisualLayer> {
        match self {
            LayerRender::Video(v) | LayerRender::Image(v) | LayerRender::Lottie(v) => Some(v),
            LayerRender::Text(_) => None,
        }
    }

    /// The originating clip id, for `refresh_visuals` re-sampling and overlay
    /// hit-testing.
    pub fn clip_id(&self) -> &str {
        match self {
            LayerRender::Video(v) | LayerRender::Image(v) | LayerRender::Lottie(v) => &v.clip_id,
            LayerRender::Text(t) => &t.clip_id,
        }
    }
}

/// The texture-backed layer payload shared by Video / Image / Lottie.
///
/// `transform`, `opacity`, and `crop` are the **already-sampled** values at the
/// frame this layer was built for (E5-S4). The `refresh_visuals` fast path
/// (risk #8) re-samples only these three fields without re-deciding which clips are
/// active or re-fetching pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualLayer {
    /// The clip this layer came from (stable id for re-sampling / overlays).
    pub clip_id: String,
    /// The decoded source frame to sample (E5-S8 fetches it).
    pub frame: FrameRef,
    /// The full layer affine: `preferred_transform ∘ affine(clip.transform@frame)`
    /// in **render-pixel** space (E5-S4 / reference `emitTransform`).
    pub transform: Mat3,
    /// Effective opacity at the frame, in `[0, 1]` — folds static × keyframe ×
    /// fade (reference `opacityAt`, clamped). Premultiplied-alpha intent: the
    /// compositor multiplies premultiplied source RGBA by this scalar (risk #3).
    pub opacity: f64,
    /// The visible crop rectangle in source pixels (reference `emitCrop`).
    pub crop: CropRect,
    /// The decoded frame's natural (display) size in pixels — `natSize` after the
    /// `preferred_transform` bbox re-origin (reference `clipNaturalSizes`). The
    /// compositor needs it to place the textured quad; carried so E5-S8 doesn't
    /// re-derive it.
    pub natural_size: (f64, f64),
    /// Whether the source carries straight alpha that must be premultiplied on
    /// upload (risk #3 — codec/pixfmt alpha flag, surfaced here for the compositor).
    pub has_alpha: bool,
}

/// A text layer (E5-S9): the clip's shaped [`GlyphRun`](palmier_text::GlyphRun)
/// (positioned glyphs + resolved style + box) plus the sampled opacity. The glyph
/// run is laid out by `palmier-text` (cosmic-text) in **render-pixel** space with
/// the box already placed from the clip's normalized transform, so the text pass
/// draws glyph/background/border/shadow quads straight from it — no per-layer
/// [`Mat3`] is needed (the box geometry is baked into the glyph positions). The
/// `transform` is retained for parity with visual layers / overlay hit-testing.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayer {
    pub clip_id: String,
    /// The shaped glyph run (positions + style + box), render-pixel space.
    pub run: palmier_text::GlyphRun,
    /// Effective opacity at the frame, in `[0, 1]` (folds keyframes × fade; `0`
    /// during the 30-frame preroll lead-in). The text pass multiplies every glyph /
    /// box / shadow alpha by this.
    pub opacity: f64,
    /// The sampled layer affine (parity with visual layers; the glyph positions
    /// already embed the box placement, so the text pass does not re-apply it).
    pub transform: Mat3,
}

/// The composition graph for a single timeline frame: the ordered, bottom→top stack
/// of [`LayerRender`]s the compositor draws over a black background.
///
/// `layers[0]` is the **bottom** of the z-stack (drawn first); the last element is
/// the top. There is **no** black-background layer in the vec — the compositor
/// clears its target to black as the opaque floor (reference inserts a real black
/// track; the wgpu model just clears, FOUNDATION §6.5 / risk #2).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompositionFrame {
    /// The timeline frame index this composition was built for.
    pub frame_index: i32,
    /// Visible layers, **bottom→top** (track order = z-order).
    pub layers: Vec<LayerRender>,
}

impl CompositionFrame {
    /// An empty composition (black frame) at `frame_index`.
    pub fn empty(frame_index: i32) -> Self {
        CompositionFrame {
            frame_index,
            layers: Vec::new(),
        }
    }
}
