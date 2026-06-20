//! Resolved render style — the plain-number form of [`palmier_model::TextStyle`]
//! the engine's text pass uploads as **shader uniforms** (FOUNDATION §6.6: "Text
//! style (color, background fill, border, shadow) as shader uniforms on the text
//! pass"). Ported from the reference `TextLayerController.applyStyle`.
//!
//! Everything here is presentation-agnostic numbers (sRGB `[0,1]` colors, pixel
//! offsets in **render pixels**) — no GPU types. The reference scales shadow/border
//! by the same `containerH/1080` factor as the font; we surface that scale as
//! [`FontScale`] so the engine applies one consistent factor.

use palmier_model::{Fill, Rgba, Shadow, TextAlignment, TextStyle};

/// The reference canvas height the font/shadow/border scale is normalized against
/// (`TextLayerController.referenceCanvasHeight = 1080`).
pub const REFERENCE_CANVAS_HEIGHT: f64 = 1080.0;

/// The `containerH / 1080` scale the reference applies to font size, shadow offset
/// / blur, and border width so text occupies the same fraction of any canvas size.
/// (`applyStyle`: `let scale = containerSize.height / referenceCanvasHeight`.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontScale(pub f64);

impl FontScale {
    /// The scale for a canvas `container_height` px tall (clamped ≥ 0).
    pub fn for_container(container_height: f64) -> Self {
        FontScale((container_height / REFERENCE_CANVAS_HEIGHT).max(0.0))
    }

    /// Apply the scale to a value.
    pub fn apply(self, v: f64) -> f64 {
        v * self.0
    }
}

/// An sRGB color, components in `[0, 1]` (matches [`palmier_model::Rgba`]). The
/// reference builds `NSColor(srgbRed:…)`; we keep sRGB and let the compositor's
/// BT.709 working space handle transfer (risk #5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl From<Rgba> for RenderColor {
    fn from(c: Rgba) -> Self {
        RenderColor { r: c.r, g: c.g, b: c.b, a: c.a }
    }
}

impl RenderColor {
    /// Premultiplied `[r, g, b, a]` bytes (the compositor blends premultiplied —
    /// risk #3). Clamped to `[0, 1]` then scaled to `0..=255`.
    pub fn premultiplied_bytes(self) -> [u8; 4] {
        let a = self.a.clamp(0.0, 1.0);
        let to_u8 = |c: f64| (c.clamp(0.0, 1.0) * a * 255.0).round() as u8;
        [to_u8(self.r), to_u8(self.g), to_u8(self.b), (a * 255.0).round() as u8]
    }
}

/// A resolved drop shadow (reference `TextStyle.Shadow` after the `containerH/1080`
/// scale). `offset` is in **render pixels**; the reference flips offset-Y via the
/// geometry-flipped layer — see [`render_style`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    pub color: RenderColor,
    pub offset_x: f64,
    pub offset_y: f64,
    pub blur: f64,
}

/// The fully-resolved style the text pass renders: text color, optional background
/// fill, optional border (color + width), optional shadow, and alignment — all
/// scaled into render pixels for one frame's canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderStyle {
    /// Glyph fill color.
    pub color: RenderColor,
    /// Background box fill, if `background.enabled`.
    pub background: Option<RenderColor>,
    /// Border color + width (render px), if `border.enabled`.
    pub border: Option<(RenderColor, f64)>,
    /// Drop shadow, if `shadow.enabled`.
    pub shadow: Option<ShadowStyle>,
    /// Horizontal alignment (drives the layout, carried here for the engine too).
    pub alignment: TextAlignment,
    /// The font size in render px: `(fontSize × fontScale) × containerH/1080`.
    pub font_px: f64,
}

/// The reference border width (`AppTheme.BorderWidth.thin`) the reference scales by
/// `containerH/1080`. The token is `1.0` pt in the macOS theme.
pub const BORDER_WIDTH_THIN: f64 = 1.0;

/// Resolve a [`TextStyle`] into a [`RenderStyle`] for a canvas `container_height`
/// px tall — verbatim with the reference `applyStyle`:
/// - `fontSize = (style.fontSize × style.fontScale) × containerH/1080`,
/// - `borderWidth = BorderWidth.thin × scale` when the border is enabled,
/// - `shadowOffset/Radius × scale` when the shadow is enabled,
/// - background/border/shadow only present when their `Fill.enabled` / `shadow.enabled`.
pub fn render_style(style: &TextStyle, container_height: f64) -> RenderStyle {
    let scale = FontScale::for_container(container_height);
    let font_px = scale.apply(style.font_size * style.font_scale);

    RenderStyle {
        color: style.color.into(),
        background: enabled_fill(&style.background),
        border: enabled_fill(&style.border).map(|c| (c, scale.apply(BORDER_WIDTH_THIN))),
        shadow: resolve_shadow(&style.shadow, scale),
        alignment: style.alignment,
        font_px,
    }
}

fn enabled_fill(fill: &Fill) -> Option<RenderColor> {
    if fill.enabled {
        Some(fill.color.into())
    } else {
        None
    }
}

fn resolve_shadow(shadow: &Shadow, scale: FontScale) -> Option<ShadowStyle> {
    if !shadow.enabled {
        return None;
    }
    Some(ShadowStyle {
        color: shadow.color.into(),
        offset_x: scale.apply(shadow.offset_x),
        offset_y: scale.apply(shadow.offset_y),
        blur: scale.apply(shadow.blur).max(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::TextStyle;

    #[test]
    fn font_scale_is_one_at_1080() {
        assert_eq!(FontScale::for_container(1080.0).0, 1.0);
        assert_eq!(FontScale::for_container(2160.0).0, 2.0);
        assert_eq!(FontScale::for_container(540.0).0, 0.5);
    }

    #[test]
    fn default_style_resolves_font_px() {
        // Default: 96 px font × scale 1 × containerH/1080. At 1080 → 96 px.
        let s = TextStyle::default();
        let rs = render_style(&s, 1080.0);
        assert_eq!(rs.font_px, 96.0);
        // At 4K (2160 tall) the font doubles.
        assert_eq!(render_style(&s, 2160.0).font_px, 192.0);
    }

    #[test]
    fn default_style_has_shadow_no_bg_no_border() {
        let rs = render_style(&TextStyle::default(), 1080.0);
        assert!(rs.background.is_none(), "default background disabled");
        assert!(rs.border.is_none(), "default border disabled");
        let sh = rs.shadow.expect("default shadow enabled");
        assert_eq!(sh.offset_y, -2.0);
        assert_eq!(sh.blur, 6.0);
    }

    #[test]
    fn enabled_background_and_border_resolve_scaled() {
        let mut s = TextStyle::default();
        s.background.enabled = true;
        s.background.color = Rgba::new(0.0, 0.0, 1.0, 1.0);
        s.border.enabled = true;
        let rs = render_style(&s, 2160.0); // scale 2
        assert!(rs.background.is_some());
        let (_, w) = rs.border.expect("border");
        assert_eq!(w, BORDER_WIDTH_THIN * 2.0);
    }

    #[test]
    fn premultiplied_color_bytes() {
        // Opaque red → (255,0,0,255).
        assert_eq!(RenderColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }.premultiplied_bytes(), [255, 0, 0, 255]);
        // 50% white → premultiplied ~ (128,128,128,128).
        let p = RenderColor { r: 1.0, g: 1.0, b: 1.0, a: 0.5 }.premultiplied_bytes();
        assert!((126..=130).contains(&p[0]) && (126..=130).contains(&p[3]));
    }
}
