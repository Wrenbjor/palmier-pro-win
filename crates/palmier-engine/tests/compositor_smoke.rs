//! Headless smoke render for the wgpu compositor — E5-S8.
//!
//! Builds a headless [`Compositor`] (offscreen `Rgba8Unorm` target, no window),
//! feeds it a [`RenderFrame`] with one full-canvas opaque RGBA layer, renders, and
//! reads the result back to assert:
//! - an **empty** composition reads back **black** (the opaque floor / risk #2), and
//! - a single full-canvas red layer paints the canvas **red** (textured-quad draw +
//!   premultiplied-alpha blend over black, the core compositor contract).
//!
//! This is the "smoke render" the story asks for — it exercises device creation, the
//! pipeline, the texture upload (`write_texture`), the per-layer quad draw, and the
//! readback end to end on a real GPU. It is **gated**: on a headless box with no
//! adapter, [`Compositor::new_headless`] returns `NoAdapter` and the test prints a
//! skip notice and passes (FOUNDATION §11.1: GPU paths run headless or are skipped).
//!
//! Frames are supplied through a **fake [`FrameProvider`]** (we can't seed
//! `palmier-media`'s private `FrameSource` cache from outside that crate, and the
//! production path is the same trait the compositor draws through).
//!
//! Run: `pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-engine \
//!       --features wgpu-compositor --test compositor_smoke -- --nocapture`

#![cfg(feature = "wgpu-compositor")]

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use palmier_engine::compositor::gpu::Compositor;
use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{
    Canvas, CompositionFrame, CropRect, FrameRef, LayerRender, Mat3, QualityTarget, RenderFrame,
    VisualLayer,
};
use palmier_media::decode::frame::{PixelLayout, Plane};
use palmier_media::{DecodedFrame, SeekMode};

/// A provider that returns a single solid-color RGBA frame for any request. Stands
/// in for `palmier-media`'s `FrameSource` so the GPU path runs without a media file.
struct SolidProvider {
    w: u32,
    h: u32,
    rgba: [u8; 4],
}

impl FrameProvider for SolidProvider {
    type Error = std::convert::Infallible;
    fn provide_frame(
        &self,
        _media_ref: &str,
        source_frame: u64,
        _mode: SeekMode,
        _active_layers: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        let mut bytes = Vec::with_capacity((self.w * self.h * 4) as usize);
        for _ in 0..(self.w * self.h) {
            bytes.extend_from_slice(&self.rgba);
        }
        Ok(DecodedFrame {
            layout: PixelLayout::Rgba8,
            width: self.w,
            height: self.h,
            has_alpha: false,
            planes: Arc::new(vec![Plane {
                bytes,
                stride: (self.w * 4) as usize,
                width: self.w,
                height: self.h,
            }]),
            source_frame,
        })
    }
}

/// A provider that always fails — used to confirm a missing source skips the layer
/// rather than killing the frame.
struct DeadProvider;
impl FrameProvider for DeadProvider {
    type Error = &'static str;
    fn provide_frame(
        &self,
        _m: &str,
        _f: u64,
        _mode: SeekMode,
        _a: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        Err("offline")
    }
}

fn full_canvas_layer(media_ref: &str, w: f64, h: f64, opacity: f64) -> LayerRender {
    LayerRender::Video(VisualLayer {
        clip_id: "c0".into(),
        frame: FrameRef::new(media_ref, 0),
        transform: Mat3::IDENTITY,
        opacity,
        crop: CropRect::full(w, h),
        natural_size: (w, h),
        has_alpha: false,
    })
}

/// Serialize all GPU tests: each builds its own wgpu device, and creating several
/// devices/adapters concurrently corrupts the heap on some Windows drivers
/// (the default test harness runs tests in parallel). A process-wide lock makes the
/// GPU section serial without needing `--test-threads=1` on the command line.
fn gpu_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn try_headless(cw: u32, ch: u32) -> Option<Compositor> {
    match Compositor::new_headless(cw, ch) {
        Ok(c) => {
            eprintln!("[E5-S8 smoke] adapter: {}", c.adapter_summary());
            Some(c)
        }
        Err(e) => {
            eprintln!("[E5-S8 smoke] no GPU adapter — skipping headless render ({e}).");
            None
        }
    }
}

#[test]
fn black_floor_and_red_layer_smoke_render() {
    let _guard = gpu_lock();
    let (cw, ch) = (64u32, 48u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // 1) Empty composition → black opaque floor.
    let empty =
        RenderFrame::new(CompositionFrame::empty(0), Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&empty, &DeadProvider).expect("render empty");
    let img = comp.read_back().expect("offscreen readback");
    assert_eq!(img.width, cw);
    assert_eq!(img.height, ch);
    let center = ((ch / 2 * cw + cw / 2) * 4) as usize;
    assert_eq!(&img.bytes[center..center + 4], &[0, 0, 0, 255], "empty → black floor");

    // 2) One full-canvas opaque-red layer → canvas reads back red.
    let src = SolidProvider { w: cw, h: ch, rgba: [255, 0, 0, 255] };
    let mut cf = CompositionFrame::empty(0);
    cf.layers.push(full_canvas_layer("m", cw as f64, ch as f64, 1.0));
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&frame, &src).expect("render red layer");
    let img = comp.read_back().expect("readback");
    let px = &img.bytes[center..center + 4];
    assert!(px[0] > 200, "red channel high: {px:?}");
    assert!(px[1] < 40, "green channel low: {px:?}");
    assert!(px[2] < 40, "blue channel low: {px:?}");
    eprintln!("[E5-S8 smoke] full-canvas red layer composited: center px = {px:?}");

    // The texture should now be cached (one resident texture under the VRAM ceiling).
    let stats = comp.cache_stats();
    assert_eq!(stats.texture_count, 1, "layer texture cached");
    assert!(stats.vram_bytes > 0);
}

#[test]
fn half_opacity_layer_blends_toward_black() {
    let _guard = gpu_lock();
    let (cw, ch) = (32u32, 32u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // A white layer at 50% opacity over black → ~mid gray (premultiplied blend).
    let src = SolidProvider { w: cw, h: ch, rgba: [255, 255, 255, 255] };
    let mut cf = CompositionFrame::empty(0);
    cf.layers.push(full_canvas_layer("w", cw as f64, ch as f64, 0.5));
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&frame, &src).expect("render");
    let img = comp.read_back().expect("readback");
    let center = ((ch / 2 * cw + cw / 2) * 4) as usize;
    let px = &img.bytes[center..center + 4];
    // 50% white over black ≈ 128 (allow wide tolerance for format/rounding).
    assert!((90..=165).contains(&px[0]), "half-opacity white → mid gray, got {px:?}");
}

#[test]
fn missing_source_skips_layer_not_frame() {
    let _guard = gpu_lock();
    let (cw, ch) = (16u32, 16u32);
    let Some(mut comp) = try_headless(cw, ch) else { return };

    // One layer whose source is offline → the layer is skipped, frame still renders
    // (black floor), render() returns Ok.
    let mut cf = CompositionFrame::empty(0);
    cf.layers.push(full_canvas_layer("gone", cw as f64, ch as f64, 1.0));
    let frame = RenderFrame::new(cf, Canvas::new(cw, ch), QualityTarget::Full);
    comp.render(&frame, &DeadProvider).expect("offline layer must not fail the frame");
    let img = comp.read_back().expect("readback");
    let center = ((ch / 2 * cw + cw / 2) * 4) as usize;
    assert_eq!(&img.bytes[center..center + 4], &[0, 0, 0, 255], "skipped layer → black");
}
