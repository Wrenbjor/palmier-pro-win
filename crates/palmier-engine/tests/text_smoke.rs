//! Headless GPU smoke render for the text pass — E5-S9 (mirrors E5-S8's
//! `compositor_smoke.rs`, gated behind `wgpu-compositor`).
//!
//! Builds a headless [`Compositor`], feeds it a [`RenderFrame`] whose composition
//! carries a single `LayerRender::Text` (a white caption, built via
//! [`build_text_layers`] from a one-text-clip timeline), renders, and reads back to
//! assert the text pass actually painted **non-empty** pixels over the black floor.
//!
//! It exercises: cosmic-text layout (`palmier-text`), the lazy `TextPass` build, the
//! glyph atlas rasterization + upload (`SwashCache` → R8 atlas), and the glyph quad
//! draw + premultiplied-alpha blend over the cleared target.
//!
//! Gated: on a headless box with no GPU adapter, [`Compositor::new_headless`]
//! returns `NoAdapter` and the test prints a skip notice and passes (FOUNDATION
//! §11.1: GPU paths run headless or are skipped).
//!
//! Run: `pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-engine \
//!       --features wgpu-compositor --test text_smoke -- --nocapture`

#![cfg(feature = "wgpu-compositor")]

use std::sync::{Mutex, MutexGuard, OnceLock};

use palmier_engine::compositor::gpu::Compositor;
use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{
    build_text_layers, Canvas, CompositionFrame, FontRegistry, QualityTarget, RenderFrame,
    TextLayout,
};
use palmier_media::{DecodedFrame, SeekMode};
use palmier_model::{Clip, ClipType, Rgba, TextStyle, Timeline, Track, Transform};

/// No layer in these frames resolves a media frame; a never-called provider suffices.
struct NoProvider;
impl FrameProvider for NoProvider {
    type Error = &'static str;
    fn provide_frame(
        &self,
        _m: &str,
        _f: u64,
        _mode: SeekMode,
        _a: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        Err("no video in a text-only frame")
    }
}

fn gpu_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn try_headless(cw: u32, ch: u32) -> Option<Compositor> {
    match Compositor::new_headless(cw, ch) {
        Ok(c) => {
            eprintln!("[E5-S9 text smoke] adapter: {}", c.adapter_summary());
            Some(c)
        }
        Err(e) => {
            eprintln!("[E5-S9 text smoke] no GPU adapter — skipping ({e}).");
            None
        }
    }
}

/// A timeline with one full-canvas white text clip active at the test frame.
fn text_timeline(content: &str) -> Timeline {
    let mut clip = Clip::new("text", 0, 100);
    clip.id = "caption".into();
    clip.media_type = ClipType::Text;
    clip.text_content = Some(content.into());
    let mut style = TextStyle::default();
    style.color = Rgba::new(1.0, 1.0, 1.0, 1.0); // opaque white
    style.font_size = 200.0; // large so glyphs cover plenty of pixels
    style.shadow.enabled = false; // isolate the glyph fill
    clip.text_style = Some(style);
    clip.transform = Transform::default(); // full canvas, centered

    let mut tl = Timeline::default();
    tl.width = 256;
    tl.height = 128;
    let mut track = Track::new(ClipType::Text);
    track.clips = vec![clip];
    tl.tracks = vec![track];
    tl
}

fn nonblack_pixels(img: &palmier_engine::RgbaImage) -> usize {
    img.bytes
        .chunks_exact(4)
        .filter(|px| px[0] > 8 || px[1] > 8 || px[2] > 8)
        .count()
}

#[test]
fn text_layer_renders_nonempty_pixels() {
    let _guard = gpu_lock();
    let (cw, ch) = (256u32, 128u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // Build the text layer (frame 30 is inside the clip → visible, full opacity).
    let tl = text_timeline("HELLO");
    let mut reg = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let text_layers = build_text_layers(&tl, 30, cw as f64, ch as f64, &mut reg, &mut layout);
    assert_eq!(text_layers.len(), 1, "one text layer built");

    let mut cf = CompositionFrame::empty(30);
    cf.layers.extend(text_layers);
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);

    comp.render(&frame, &NoProvider).expect("render text frame");
    let img = comp.read_back().expect("headless readback");
    assert_eq!(img.width, cw);
    assert_eq!(img.height, ch);

    let lit = nonblack_pixels(&img);
    eprintln!("[E5-S9 text smoke] non-black pixels: {lit} / {}", cw * ch);
    assert!(lit > 50, "text glyphs should paint many non-black pixels, got {lit}");
}

#[test]
fn empty_text_clip_stays_black() {
    let _guard = gpu_lock();
    let (cw, ch) = (128u32, 64u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // Empty content + no background/border/shadow → nothing to draw → black floor.
    let tl = text_timeline("");
    let mut reg = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let text_layers = build_text_layers(&tl, 30, cw as f64, ch as f64, &mut reg, &mut layout);

    let mut cf = CompositionFrame::empty(30);
    cf.layers.extend(text_layers);
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&frame, &NoProvider).expect("render empty text");
    let img = comp.read_back().expect("readback");
    assert_eq!(nonblack_pixels(&img), 0, "empty text → black floor");
}

#[test]
fn preroll_leadin_draws_nothing() {
    let _guard = gpu_lock();
    let (cw, ch) = (128u32, 64u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // A clip starting at frame 100; at frame 80 it is in the preroll window
    // (materialized) but NOT visible → opacity 0 → no pixels.
    let mut clip = Clip::new("text", 100, 60);
    clip.id = "late".into();
    clip.media_type = ClipType::Text;
    clip.text_content = Some("LATER".into());
    let mut style = TextStyle::default();
    style.color = Rgba::new(1.0, 1.0, 1.0, 1.0);
    style.shadow.enabled = false;
    clip.text_style = Some(style);
    clip.transform = Transform::default();
    let mut tl = Timeline::default();
    tl.width = cw as i32;
    tl.height = ch as i32;
    let mut track = Track::new(ClipType::Text);
    track.clips = vec![clip];
    tl.tracks = vec![track];

    let mut reg = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();
    let text_layers = build_text_layers(&tl, 80, cw as f64, ch as f64, &mut reg, &mut layout);
    assert_eq!(text_layers.len(), 1, "materialized during preroll");

    let mut cf = CompositionFrame::empty(80);
    cf.layers.extend(text_layers);
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&frame, &NoProvider).expect("render preroll");
    let img = comp.read_back().expect("readback");
    assert_eq!(nonblack_pixels(&img), 0, "preroll lead-in (opacity 0) → black");
}
