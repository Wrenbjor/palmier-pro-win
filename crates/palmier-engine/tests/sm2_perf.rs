//! SM-2 sustained-FPS measurement — E5-S11 (PRD §7 SM-2, §10 Epic 5). **Gated behind
//! `wgpu-compositor`.**
//!
//! Drives the headless [`Compositor`] in a tight offscreen render loop over a
//! representative composition (5 clips / 2 layers, full-canvas, no keyframe motion —
//! the SM-2 spec) and measures **sustained frames-per-second** at the two SM-2 points:
//!   * **1080p60** — floor ≥ 60 fps
//!   * **4K30** (3840×2160 scrub) — floor ≥ 30 fps
//!
//! It renders into an OWNED offscreen texture (no swapchain / vsync) so the number is
//! the compositor's raw throughput, not a present-rate cap — the honest measurement of
//! whether the wgpu path clears the floor on this box's adapter.
//!
//! ## Honesty contract (no fake pass)
//! - On a box **with** a GPU adapter, it asserts the measured fps ≥ the floor. If the
//!   adapter cannot meet a floor, the assert FAILS loudly (the story's "don't fake a
//!   pass" rule) — that is the signal to take the FOUNDATION §3 CPU-fallback / SM-C1
//!   waiver path, recorded in the result, not silently passed.
//! - On a box **without** an adapter (headless CI), [`Compositor::new_headless`]
//!   returns `NoAdapter`; the test prints a skip notice and passes (FOUNDATION §11.1).
//! - It also respects `SM2_REPORT_ONLY=1` to measure + print WITHOUT asserting (for
//!   running on a sub-floor box to gather the number for the fallback decision).
//!
//! The render loop also includes a warmup (pipeline/texture upload amortized) so the
//! steady-state number isn't dragged down by first-frame device/cache costs.
//!
//! Run: `pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-engine \
//!   --features wgpu-compositor --test sm2_perf -- --nocapture`

#![cfg(feature = "wgpu-compositor")]

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use palmier_engine::compositor::gpu::Compositor;
use palmier_engine::RenderFrame;

#[path = "support/golden.rs"]
mod golden;
use golden::{render_frame_video, sm2_timeline, GradientProvider};

fn gpu_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn report_only() -> bool {
    std::env::var("SM2_REPORT_ONLY").map(|v| v == "1").unwrap_or(false)
}

/// Render `frame` `iters` times on `comp`, returning sustained fps (excludes a warmup).
fn measure_fps(comp: &mut Compositor, frame: &RenderFrame, provider: &GradientProvider, iters: u32) -> f64 {
    // Warmup: prime the pipeline + upload+cache the layer textures so the timed loop
    // measures steady-state composite, not first-frame device/cache cost.
    for _ in 0..8 {
        comp.render(frame, provider).expect("warmup render");
    }
    let start = Instant::now();
    for _ in 0..iters {
        comp.render(frame, provider).expect("render");
    }
    // A final readback forces the queue to drain so wall-time covers GPU work, not just
    // submission (write_texture/cache hits make submit nearly free otherwise).
    let _ = comp.read_back();
    let elapsed = start.elapsed().as_secs_f64();
    iters as f64 / elapsed.max(f64::MIN_POSITIVE)
}

/// One SM-2 point: build a representative composition at `w×h`, measure fps, report,
/// and (unless report-only) assert the floor.
fn run_point(label: &str, w: u32, h: u32, floor: f64, iters: u32) {
    let _g = gpu_lock();
    let mut comp = match Compositor::new_headless(w, h) {
        Ok(c) => {
            eprintln!("[SM-2 {label}] adapter: {}", c.adapter_summary());
            c
        }
        Err(e) => {
            eprintln!("[SM-2 {label}] no GPU adapter — skipping ({e}).");
            return;
        }
    };

    // 5 clips / 2 layers, full-canvas, no keyframe motion (the SM-2 spec composition).
    let tl = sm2_timeline(5, w as i32, h as i32);
    let frame = render_frame_video(&tl, 0, (w as f64, h as f64));
    let active = frame.composition.layers.len();
    let provider = GradientProvider::new(w, h);

    let fps = measure_fps(&mut comp, &frame, &provider, iters);
    eprintln!(
        "[SM-2 {label}] {w}x{h}, {active} active layer(s): {fps:.1} fps (floor {floor:.0}) — {}",
        if fps >= floor { "MEETS floor" } else { "BELOW floor" }
    );

    if report_only() {
        eprintln!("[SM-2 {label}] SM2_REPORT_ONLY set — not asserting.");
        return;
    }
    assert!(
        fps >= floor,
        "SM-2 {label}: measured {fps:.1} fps < floor {floor:.0} on this adapter — \
         take the FOUNDATION §3 CPU-fallback / SM-C1 waiver path, do not fake a pass"
    );
}

#[test]
fn sm2_1080p60_meets_60fps() {
    // 1080p60 → ≥ 60 fps. 600 iters ≈ 10 s budget at floor.
    run_point("1080p60", 1920, 1080, 60.0, 600);
}

#[test]
fn sm2_4k30_meets_30fps() {
    // 4K30 scrub → ≥ 30 fps. Fewer iters (4× the pixels) keeps wall-time bounded.
    run_point("4K30", 3840, 2160, 30.0, 300);
}
