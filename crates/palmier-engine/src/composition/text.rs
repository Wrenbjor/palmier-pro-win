//! Text-layer build — E5-S9 (port of the reference
//! `TextLayerController.reconcile` / `visibleTextClips`).
//!
//! The video builder ([`build_frame`](super::build::build_frame)) drops `.text`
//! clips (text never becomes a video composition track, matching the reference).
//! Text is instead collected here into [`LayerRender::Text`] layers, each carrying
//! a `palmier-text` [`GlyphRun`](palmier_text::GlyphRun) (cosmic-text shaping) with
//! the **30-frame preroll** retained:
//!
//! - a clip is **materialized** (laid out, atlas-warmed by the GPU pass) when
//!   `start_frame - 30 <= frame < end_frame` (reference `prerollFrames = 30`),
//! - it is **visible** (its sampled opacity applies) only at `frame >= start_frame`;
//!   during the lead-in it carries opacity `0` (laid out but drawn transparent).
//!
//! Geometry: the box is the clip's normalized [`Transform`](palmier_model::Transform)
//! top-left × the canvas (reference `applyStyle` `layer.frame`), and the font scale
//! is `containerH/1080` — both handled inside `palmier-text` so text does not mirror
//! vs. video (risk #7).

use palmier_model::{ClipType, Timeline};
use palmier_text::{FontRegistry, LayoutBox, TextLayout};

use super::sampler;
use super::types::{LayerRender, TextLayer};
use super::Mat3;

/// Build the [`LayerRender::Text`] layers visible at `frame_index` for `timeline`,
/// laid out for a `canvas_w × canvas_h` render canvas (render px).
///
/// Iterates tracks top-of-model order (reference `visibleTextClips` appends in
/// track/clip order; text always composites **above** video, so the caller appends
/// these after the video stack). Honors the preroll window + the visibility opacity
/// gate. `registry`/`layout` are reused across frames (shaping is cached in the
/// `FontSystem`).
pub fn build_text_layers(
    timeline: &Timeline,
    frame_index: i32,
    canvas_w: f64,
    canvas_h: f64,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> Vec<LayerRender> {
    let mut layers = Vec::new();

    for track in &timeline.tracks {
        if track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.media_type != ClipType::Text {
                continue;
            }
            let start = clip.start_frame;
            let end = clip.end_frame();
            // Reference `visibleTextClips` requires endFrame > startFrame.
            if end <= start {
                continue;
            }
            // 30-frame preroll window (materialize gate).
            if !palmier_text::preroll_window(frame_index, start, end) {
                continue;
            }

            // Box from the normalized (center-based) transform's top-left × canvas.
            let t = clip.transform;
            let box_rect = LayoutBox::from_normalized(
                t.top_left(),
                t.width,
                t.height,
                canvas_w,
                canvas_h,
            );

            let content = clip.text_content.as_deref().unwrap_or("");
            let style = clip.text_style.clone().unwrap_or_default();
            let run = layout.layout_clip(registry, content, &style, box_rect, canvas_h);

            // Visibility gate: opacity applies only from start; preroll lead-in → 0.
            let opacity = if palmier_text::is_visible(frame_index, start) {
                sampler::layer_opacity(clip, frame_index)
            } else {
                0.0
            };

            layers.push(LayerRender::Text(TextLayer {
                clip_id: clip.id.clone(),
                run,
                opacity,
                transform: Mat3::IDENTITY,
            }));
        }
    }

    layers
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{Clip, ClipType, TextStyle, Timeline, Track, Transform};

    fn text_clip(id: &str, start: i32, dur: i32, content: &str) -> Clip {
        let mut c = Clip::new("text", start, dur);
        c.id = id.to_string();
        c.media_type = ClipType::Text;
        c.text_content = Some(content.to_string());
        c.text_style = Some(TextStyle::default());
        c.transform = Transform::default(); // full canvas, centered.
        c
    }

    fn timeline_with(clips: Vec<Clip>) -> Timeline {
        let mut tl = Timeline::default();
        tl.width = 1920;
        tl.height = 1080;
        let mut track = Track::new(ClipType::Text);
        track.clips = clips;
        tl.tracks = vec![track];
        tl
    }

    #[test]
    fn no_text_layer_before_preroll() {
        let tl = timeline_with(vec![text_clip("t0", 100, 60, "Hi")]);
        let mut reg = FontRegistry::bundled_only();
        let mut layout = TextLayout::new();
        // Frame 50 is before the start-30 preroll window.
        let layers = build_text_layers(&tl, 50, 1920.0, 1080.0, &mut reg, &mut layout);
        assert!(layers.is_empty(), "no text before preroll window");
    }

    #[test]
    fn materialized_in_preroll_but_zero_opacity() {
        let tl = timeline_with(vec![text_clip("t0", 100, 60, "Hi")]);
        let mut reg = FontRegistry::bundled_only();
        let mut layout = TextLayout::new();
        // Frame 80 is in [70, 100): materialized, opacity 0 (lead-in).
        let layers = build_text_layers(&tl, 80, 1920.0, 1080.0, &mut reg, &mut layout);
        assert_eq!(layers.len(), 1);
        let LayerRender::Text(t) = &layers[0] else { panic!("expected text layer") };
        assert_eq!(t.opacity, 0.0, "preroll lead-in draws at 0");
        assert!(!t.run.is_empty(), "glyphs are shaped during preroll (atlas warm-up)");
    }

    #[test]
    fn visible_with_opacity_during_clip() {
        let tl = timeline_with(vec![text_clip("t0", 100, 60, "Hi")]);
        let mut reg = FontRegistry::bundled_only();
        let mut layout = TextLayout::new();
        // Frame 120 is inside [100, 160): visible, full opacity.
        let layers = build_text_layers(&tl, 120, 1920.0, 1080.0, &mut reg, &mut layout);
        assert_eq!(layers.len(), 1);
        let LayerRender::Text(t) = &layers[0] else { panic!("expected text layer") };
        assert!(t.opacity > 0.0, "visible clip has opacity");
        assert!(!t.run.is_empty());
    }

    #[test]
    fn gone_after_end() {
        let tl = timeline_with(vec![text_clip("t0", 100, 60, "Hi")]);
        let mut reg = FontRegistry::bundled_only();
        let mut layout = TextLayout::new();
        // Frame 160 == end (exclusive) → gone.
        let layers = build_text_layers(&tl, 160, 1920.0, 1080.0, &mut reg, &mut layout);
        assert!(layers.is_empty(), "end is exclusive");
    }

    #[test]
    fn hidden_track_contributes_nothing() {
        let mut tl = timeline_with(vec![text_clip("t0", 100, 60, "Hi")]);
        tl.tracks[0].hidden = true;
        let mut reg = FontRegistry::bundled_only();
        let mut layout = TextLayout::new();
        let layers = build_text_layers(&tl, 120, 1920.0, 1080.0, &mut reg, &mut layout);
        assert!(layers.is_empty());
    }
}
