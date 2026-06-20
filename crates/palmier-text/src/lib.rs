//! # palmier-text
//!
//! Text **layout + shaping** for the preview/export compositor (FOUNDATION §4,
//! §6.6, §6.9; story E5-S9). Wraps [`cosmic-text`](cosmic_text) (layout, shaping,
//! multi-line line-breaking) and `fontdb` (font discovery — the bundled reference
//! fonts + the user's installed system fonts on Windows/Linux), and ports the macOS
//! reference `TextLayerController` geometry.
//!
//! ## What lives here (presentation-agnostic)
//! This crate is **pure layout** — it owns no GPU device and no pixels. Given a
//! [`palmier_model::TextStyle`] + content string + the clip's normalized transform
//! box, it produces a [`GlyphRun`] of **positioned glyphs** (each carrying the
//! cosmic-text [`CacheKey`](cosmic_text::CacheKey) the GPU text pass rasterizes
//! through a `SwashCache`) plus the resolved [`RenderStyle`] (color / background /
//! border / shadow as plain numbers the engine uploads as shader uniforms). The
//! wgpu glyph-atlas pass that turns these into textured quads lives in
//! `palmier-engine` behind the `wgpu-compositor` feature (E5-S8/E5-S9).
//!
//! ## Reference parity (`TextLayerController.swift`)
//! - **Geometry flip (risk #7).** The reference roots text in a `CALayer` with
//!   `isGeometryFlipped=true` and scales `fontSize` by `containerH / 1080`. We lay
//!   out in cosmic-text's **top-left origin** space (already the right convention vs.
//!   the y-down render canvas), apply the same `containerH/1080` font scale
//!   ([`FontScale`]), and place the box from the normalized transform's top-left —
//!   so text does **not** mirror vertically vs. video. See [`layout`].
//! - **30-frame preroll.** A text clip is materialized when
//!   `start_frame - 30 <= current_frame < end_frame` and is visible (full opacity
//!   sampling) only at `current_frame >= start_frame`. See [`preroll`].
//! - **Style.** color, background fill, border, shadow, alignment ported verbatim
//!   from `applyStyle`. See [`style`].
//!
//! ## Quick start
//! ```no_run
//! use palmier_text::{FontRegistry, TextLayout, LayoutBox};
//! use palmier_model::TextStyle;
//!
//! let mut fonts = FontRegistry::with_bundled_fonts();
//! let mut layout = TextLayout::new();
//! let run = layout.layout_clip(
//!     &mut fonts,
//!     "Hello\nWorld",
//!     &TextStyle::default(),
//!     LayoutBox { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 },
//!     1080.0, // container height (drives the containerH/1080 font scale)
//! );
//! assert!(!run.glyphs.is_empty());
//! ```

pub mod caption;
pub mod layout;
pub mod preroll;
pub mod registry;
pub mod style;

pub use caption::{caption_theme, phrases, CaptionCase, Phrase, Segment, MIN_DISPLAY_DURATION};
pub use layout::{GlyphRun, LayoutBox, NaturalSize, PositionedGlyph, TextLayout, SHADOW_PADDING};
pub use preroll::{is_visible, preroll_window, PREROLL_FRAMES};
pub use registry::{FontRegistry, BUNDLED_FONTS};
pub use style::{render_style, FontScale, RenderColor, RenderStyle, ShadowStyle};

// Re-export the cosmic-text glyph identity the engine's text pass needs to
// rasterize (the GPU pass owns the `SwashCache`; palmier-text owns the layout).
pub use cosmic_text::{self, CacheKey, Color as CosmicColor};

/// Register the bundled reference fonts (and system fonts) into a fresh
/// [`FontRegistry`]. Kept as a free function for the boot path (E1-S5) that just
/// needs the side effect of a populated registry; most callers want
/// [`FontRegistry::with_bundled_fonts`] directly so they can reuse the handle.
pub fn register_bundled_fonts() -> FontRegistry {
    FontRegistry::with_bundled_fonts()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_populated_registry() {
        let reg = register_bundled_fonts();
        // The bundled families are queryable (system fonts may or may not exist on
        // a headless CI box, but the bundled ones always do).
        assert!(reg.face_count() >= BUNDLED_FONTS.len());
    }
}
