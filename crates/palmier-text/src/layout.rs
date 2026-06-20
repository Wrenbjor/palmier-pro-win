//! Text layout + shaping (FOUNDATION §6.6, reference
//! `TextLayerController.applyStyle` typesetting).
//!
//! Lays out a clip's content string in its box using [`cosmic-text`](cosmic_text):
//! multi-line line-breaking (word-wrap, matching the reference's
//! `lineBreakMode = .byWordWrapping`), shaping, and alignment. Produces a
//! [`GlyphRun`] of [`PositionedGlyph`]s in **render-pixel** coordinates (top-left
//! origin), each carrying the cosmic-text [`CacheKey`](cosmic_text::CacheKey) the
//! GPU text pass rasterizes through its `SwashCache`.
//!
//! ## Geometry (risk #7 — no vertical mirror)
//! The reference roots text in an `isGeometryFlipped=true` `CALayer` (so its y-down
//! coordinates match AppKit's y-up after the flip) and places the box at the
//! normalized transform's **top-left** `(tl.x·W, tl.y·H)`. cosmic-text already lays
//! out **top-left origin, y-down**, which is the render canvas convention used by
//! the compositor's [`layer_clip_matrix`] (it y-flips to NDC). So we lay out in the
//! box directly with **no extra flip** — glyph `y` grows downward exactly like the
//! video layers, and the engine's existing source-pixel→NDC matrix handles the flip
//! once. Adding a flip here would double it and mirror text vs. video.

use cosmic_text::{Attrs, Buffer, CacheKey, Family, Metrics, Shaping, Weight};
use palmier_model::{TextAlignment, TextStyle};

use crate::registry::FontRegistry;
use crate::style::{render_style, RenderStyle};

/// The clip's layout box in **render pixels** (the normalized transform mapped onto
/// the canvas: `x = tl.x·W`, `y = tl.y·H`, `width = t.width·W`, `height = t.height·H`).
/// Matches the reference `layer.frame` rect in `applyStyle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl LayoutBox {
    /// Build the box from a normalized [`palmier_model::Transform`]-style top-left +
    /// size and the canvas dimensions (render px). The reference uses
    /// `transform.topLeft` (center-based model → top-left) × container size.
    pub fn from_normalized(
        top_left: (f64, f64),
        norm_w: f64,
        norm_h: f64,
        canvas_w: f64,
        canvas_h: f64,
    ) -> Self {
        LayoutBox {
            x: top_left.0 * canvas_w,
            y: top_left.1 * canvas_h,
            width: norm_w * canvas_w,
            height: norm_h * canvas_h,
        }
    }
}

/// One shaped, positioned glyph ready for the GPU text pass. Coordinates are in
/// **render pixels**, top-left origin (the layout box's own space + the box origin
/// folded in, so `x`/`y` are absolute on the canvas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// The cosmic-text glyph identity (font + glyph id + subpixel offset). The GPU
    /// text pass calls `SwashCache::get_image(font_system, cache_key)` to rasterize
    /// it into the glyph atlas.
    pub cache_key: CacheKey,
    /// Pen x of the glyph's physical origin, render px (absolute on the canvas).
    pub x: f32,
    /// Pen y of the glyph's physical origin (baseline-relative line y), render px.
    pub y: f32,
}

/// A laid-out text clip: its positioned glyphs, the resolved [`RenderStyle`]
/// (color/bg/border/shadow uniforms), and the layout box (the bg/border rect the
/// engine fills). This is the payload the engine carries in `LayerRender::Text`.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// Shaped glyphs, render-pixel positions (top-left origin).
    pub glyphs: Vec<PositionedGlyph>,
    /// Resolved style (shader uniforms).
    pub style: RenderStyle,
    /// The clip box in render px (background/border fill rect).
    pub box_rect: LayoutBox,
}

impl GlyphRun {
    /// Whether the run produced no glyphs (empty content / whitespace) — the engine
    /// may still draw the background box, but there's nothing to rasterize.
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

/// The layout engine. Wraps the cosmic-text scratch buffer reuse; construct once
/// and call [`TextLayout::layout_clip`] per text clip per frame (cheap — shaping is
/// cached in the [`FontRegistry`]'s `FontSystem`).
#[derive(Default)]
pub struct TextLayout {
    _private: (),
}

impl TextLayout {
    /// A new layout engine.
    pub fn new() -> Self {
        TextLayout { _private: () }
    }

    /// Lay out `content` for `style` within `box_rect` (render px) on a canvas
    /// `container_height` px tall (drives the `containerH/1080` font scale).
    ///
    /// Line-breaking is word-wrap to the box width (reference
    /// `lineBreakMode = .byWordWrapping`); alignment follows `style.alignment`.
    /// Returns a [`GlyphRun`] of absolute render-pixel glyph positions.
    pub fn layout_clip(
        &mut self,
        registry: &mut FontRegistry,
        content: &str,
        style: &TextStyle,
        box_rect: LayoutBox,
        container_height: f64,
    ) -> GlyphRun {
        let resolved = render_style(style, container_height);
        let font_system = registry.font_system_mut();

        // Metrics: font size in render px; line height = font size (the reference
        // CATextLayer uses the font's natural leading; 1.2× is the common default,
        // but the reference relies on the attributed string's own line height — we
        // use the font size as line height for tight caption layout, matching the
        // single-line-dominant caption case and letting wrap add lines cleanly).
        let metrics = Metrics::new(resolved.font_px as f32, (resolved.font_px * 1.2) as f32);
        let mut buffer = Buffer::new(font_system, metrics);

        // Constrain to the box so word-wrap matches the reference's wrapped layer.
        buffer.set_size(
            font_system,
            Some(box_rect.width.max(1.0) as f32),
            Some(box_rect.height.max(1.0) as f32),
        );

        let attrs = attrs_for(style);
        buffer.set_text(font_system, content, &attrs, Shaping::Advanced);
        // Apply horizontal alignment per line.
        let align = cosmic_align(style.alignment);
        for line in buffer.lines.iter_mut() {
            line.set_align(Some(align));
        }
        buffer.shape_until_scroll(font_system, false);

        // Collect positioned glyphs. cosmic-text lays out at origin (0,0) of the
        // buffer; we offset by the box origin so positions are absolute on the canvas.
        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((box_rect.x as f32, 0.0), 1.0);
                glyphs.push(PositionedGlyph {
                    cache_key: physical.cache_key,
                    x: physical.x as f32,
                    // run.line_y is the baseline y within the buffer; add the box y.
                    y: physical.y as f32 + box_rect.y as f32 + run.line_y,
                });
            }
        }

        GlyphRun { glyphs, style: resolved, box_rect }
    }
}

/// Build cosmic-text [`Attrs`] for a [`TextStyle`]: resolve the font family by name
/// (the reference `NSFont(name:)`), defaulting weight to bold for the reference's
/// `Helvetica-Bold` default. cosmic-text falls back to its default family + the
/// system fallback chain when the named family is absent (mirrors the reference's
/// `?? NSFont.boldSystemFont`).
fn attrs_for(style: &TextStyle) -> Attrs<'_> {
    let name = style.font_name.as_str();
    // The reference default "Helvetica-Bold" encodes weight in the name; map that
    // hint to a bold weight so a bundled/ system Helvetica picks the bold face.
    let weight = if name.to_ascii_lowercase().contains("bold") {
        Weight::BOLD
    } else {
        Weight::NORMAL
    };
    // Strip a trailing style suffix ("-Bold"/"-Italic") so the family query matches
    // the base family name fontdb stores (weight is requested separately).
    let family_name = name
        .split('-')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    Attrs::new().family(Family::Name(family_name)).weight(weight)
}

/// Map our [`TextAlignment`] to cosmic-text's `Align`.
fn cosmic_align(a: TextAlignment) -> cosmic_text::Align {
    match a {
        TextAlignment::Left => cosmic_text::Align::Left,
        TextAlignment::Center => cosmic_text::Align::Center,
        TextAlignment::Right => cosmic_text::Align::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::TextStyle;

    fn registry() -> FontRegistry {
        FontRegistry::bundled_only()
    }

    fn box1080() -> LayoutBox {
        LayoutBox { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 }
    }

    #[test]
    fn lays_out_nonempty_glyphs() {
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let run = layout.layout_clip(&mut reg, "Hello", &TextStyle::default(), box1080(), 1080.0);
        assert!(!run.is_empty(), "expected glyphs for 'Hello'");
        // 'Hello' is 5 glyphs (no ligature in the default face for this run).
        assert!(run.glyphs.len() >= 4, "got {} glyphs", run.glyphs.len());
    }

    #[test]
    fn empty_content_has_no_glyphs() {
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let run = layout.layout_clip(&mut reg, "", &TextStyle::default(), box1080(), 1080.0);
        assert!(run.is_empty());
    }

    #[test]
    fn multiline_breaks_into_more_lines() {
        // An explicit newline must yield glyphs on two distinct baseline rows.
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let run = layout.layout_clip(&mut reg, "AA\nBB", &TextStyle::default(), box1080(), 1080.0);
        let ys: std::collections::BTreeSet<i64> =
            run.glyphs.iter().map(|g| g.y.round() as i64).collect();
        assert!(ys.len() >= 2, "two text lines → ≥2 baseline rows, got {:?}", ys);
    }

    #[test]
    fn word_wrap_to_narrow_box_adds_lines() {
        // A long single word-set in a narrow box wraps to multiple baseline rows.
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let narrow = LayoutBox { x: 0.0, y: 0.0, width: 300.0, height: 1080.0 };
        let run = layout.layout_clip(
            &mut reg,
            "word word word word word word",
            &TextStyle::default(),
            narrow,
            1080.0,
        );
        let rows: std::collections::BTreeSet<i64> =
            run.glyphs.iter().map(|g| g.y.round() as i64).collect();
        assert!(rows.len() >= 2, "narrow box should wrap, got rows {:?}", rows);
    }

    #[test]
    fn center_alignment_shifts_right_vs_left() {
        // The first glyph of a centered short line starts further right than left-aligned.
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let mut left = TextStyle::default();
        left.alignment = TextAlignment::Left;
        let mut center = TextStyle::default();
        center.alignment = TextAlignment::Center;
        let bx = LayoutBox { x: 0.0, y: 0.0, width: 1200.0, height: 1080.0 };
        let lrun = layout.layout_clip(&mut reg, "Hi", &left, bx, 1080.0);
        let crun = layout.layout_clip(&mut reg, "Hi", &center, bx, 1080.0);
        let lx = lrun.glyphs.iter().map(|g| g.x).fold(f32::MAX, f32::min);
        let cx = crun.glyphs.iter().map(|g| g.x).fold(f32::MAX, f32::min);
        assert!(cx > lx + 50.0, "centered ({cx}) should start right of left ({lx})");
    }

    #[test]
    fn box_origin_offsets_glyph_positions() {
        // Moving the box down by 540 px shifts every baseline down by ~540.
        let mut reg = registry();
        let mut layout = TextLayout::new();
        let top = LayoutBox { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 };
        let mid = LayoutBox { x: 0.0, y: 540.0, width: 1920.0, height: 1080.0 };
        let trun = layout.layout_clip(&mut reg, "X", &TextStyle::default(), top, 1080.0);
        let mrun = layout.layout_clip(&mut reg, "X", &TextStyle::default(), mid, 1080.0);
        let ty = trun.glyphs[0].y;
        let my = mrun.glyphs[0].y;
        assert!((my - ty - 540.0).abs() < 2.0, "box y offset applied: {ty} → {my}");
    }
}
