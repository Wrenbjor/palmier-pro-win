//! Runnable proof for Spike S-1.
//!
//! Proves the *produce* side end-to-end (wgpu renders a frame we own on the real backend)
//! and MEASURES candidate (c) — the GPU->CPU readback fallback — at 1080p and 4K, so the
//! spike states the fallback's cost with a real number against the SM-2 FPS floors.
//!
//! It does NOT open a Tauri window: presenting into a live WebView2/WebKitGTK is an
//! app-shell task (E5-S8). The recommended zero-copy seam is pinned call-by-call in
//! `present.rs`; this bin nails down everything a headless spike can actually execute.
//!
//! Run (Windows): pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin readback_proof

use s1_wgpu_webview::present::{plan_for, ViewportRect};
use s1_wgpu_webview::readback::read_frame_to_cpu;
use s1_wgpu_webview::render::{render_frame, GpuContext};
use s1_wgpu_webview::TargetOs;

fn fps_budget_ms(fps: f64) -> f64 {
    1000.0 / fps
}

fn measure(ctx: &GpuContext, label: &str, w: u32, h: u32, target_fps: f64, frames: u32) {
    let mut total = std::time::Duration::ZERO;
    let mut worst = std::time::Duration::ZERO;
    for i in 0..frames {
        let frame = render_frame(ctx, w, h, i);
        let rb = read_frame_to_cpu(ctx, &frame);
        total += rb.gpu_to_cpu;
        worst = worst.max(rb.gpu_to_cpu);
        // sanity: prove we actually got pixels of the right size
        if i == 0 {
            let expected = (w * h * rb.bytes_per_pixel) as usize;
            assert_eq!(rb.pixels.len(), expected, "readback pixel count mismatch");
        }
    }
    let avg_ms = total.as_secs_f64() * 1000.0 / frames as f64;
    let worst_ms = worst.as_secs_f64() * 1000.0;
    let budget = fps_budget_ms(target_fps);
    let mbytes = (w * h * 4) as f64 / (1024.0 * 1024.0);
    let verdict = if avg_ms < budget { "WITHIN budget" } else { "BUSTS budget" };
    println!(
        "  {label:<14} {w}x{h}  frame={mbytes:6.1} MB  readback avg={avg_ms:6.2} ms  worst={worst_ms:6.2} ms  \
         (budget @ {target_fps:.0} fps = {budget:.2} ms)  -> {verdict}"
    );
    println!(
        "                 note: this is GPU->CPU ONLY; production adds IPC/serialize to the <canvas> on top."
    );
}

fn main() {
    println!("=== Spike S-1: wgpu -> WebView presentation proof ===\n");

    let ctx = match GpuContext::new_headless() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("GPU init failed (no adapter in this environment?): {e}");
            eprintln!("The skeleton still compiles; readback timing requires a real GPU.");
            std::process::exit(0);
        }
    };

    println!("[produce] wgpu adapter up: {}", ctx.adapter_summary());
    println!("[produce] rendered a frame to an owned wgpu::Texture — the palmier-engine seam.\n");

    // Recommended presentation plan, per platform.
    let target = if cfg!(windows) { TargetOs::Windows } else { TargetOs::Linux };
    let plan = plan_for(target);
    println!("[present] recommended plan for {:?}:", plan.target_os);
    println!("          mechanism = {:?}", plan.mechanism);
    println!("          timeline_shares_surface = {}", plan.timeline_shares_surface);
    println!("          {}", plan.notes);
    let _ = s1_wgpu_webview::present::windows::seam_call_path(ViewportRect { x: 0, y: 0, width: 1920, height: 1080 });
    let _ = s1_wgpu_webview::present::linux::seam_call_path(ViewportRect { x: 0, y: 0, width: 1920, height: 1080 });
    println!();

    // Candidate (c) measurement — the fallback cost.
    println!("[fallback (c)] GPU->CPU readback cost (the perf cliff to avoid):");
    measure(&ctx, "1080p60", 1920, 1080, 60.0, 30);
    measure(&ctx, "4K@30", 3840, 2160, 30.0, 30);

    println!("\n=== done. See FINDINGS.md for the decision record. ===");
}
