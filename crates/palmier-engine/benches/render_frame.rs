//! SM-2 GPU render-frame throughput benchmark — FOUNDATION §11.4 / §10 SM-2 (E5-S11).
//!
//! The Criterion companion to `sm2_perf.rs`: where the test asserts the floor, this
//! bench produces a stable *throughput baseline* for the offscreen wgpu render-frame
//! at **1080p60** and **4K30**, on this box's adapter, over the representative SM-2
//! composition (5 clips / 2 layers, full-canvas). Criterion's statistics make
//! regressions visible run-over-run.
//!
//! **Feature-gated:** the GPU body only compiles under `wgpu-compositor`. With the
//! feature off (the default-features build the gate uses for `cargo bench --no-run`),
//! `main` is an empty stub that prints a skip line — so the bench target always
//! **compiles and runs GPU-free**, satisfying the build gate, and does the real GPU
//! measurement only when the feature is on:
//! `pwsh -File scripts/with-msvc.ps1 cargo bench --package palmier-engine \
//!   --features wgpu-compositor --bench render_frame`.

#[cfg(not(feature = "wgpu-compositor"))]
fn main() {
    eprintln!(
        "render_frame bench is a no-op without --features wgpu-compositor \
         (no GPU compositor compiled). Skipping."
    );
}

#[cfg(feature = "wgpu-compositor")]
#[path = "../tests/support/golden.rs"]
mod golden;

#[cfg(feature = "wgpu-compositor")]
mod gpu_bench {
    use super::golden::{render_frame_video, sm2_timeline, GradientProvider};
    use criterion::{criterion_group, BenchmarkId, Criterion};
    use palmier_engine::compositor::gpu::Compositor;

    pub fn bench_render_frame(c: &mut Criterion) {
        let mut group = c.benchmark_group("render_frame");
        // Long enough samples that device submission cost is amortized; these are real
        // GPU ops so keep the sample budget modest.
        group.sample_size(20);

        for &(label, w, h) in &[("1080p60", 1920u32, 1080u32), ("4K30", 3840, 2160)] {
            let mut comp = match Compositor::new_headless(w, h) {
                Ok(c) => {
                    eprintln!("[render_frame {label}] adapter: {}", c.adapter_summary());
                    c
                }
                Err(e) => {
                    eprintln!("[render_frame {label}] no GPU adapter — skipping ({e}).");
                    continue;
                }
            };
            let tl = sm2_timeline(5, w as i32, h as i32);
            let frame = render_frame_video(&tl, 0, (w as f64, h as f64));
            let provider = GradientProvider::new(w, h);
            // Warm the pipeline + texture cache so the timed iterations are steady-state.
            for _ in 0..8 {
                comp.render(&frame, &provider).expect("warmup");
            }
            group.bench_with_input(BenchmarkId::from_parameter(label), &label, |b, _| {
                b.iter(|| {
                    comp.render(&frame, &provider).expect("render");
                });
            });
        }
        group.finish();
    }

    criterion_group!(benches, bench_render_frame);
}

#[cfg(feature = "wgpu-compositor")]
fn main() {
    // `criterion_group!` generates `benches()`, which builds a `Criterion` from CLI
    // args, runs the group, and emits the final summary.
    gpu_bench::benches();
    criterion::Criterion::default().configure_from_args().final_summary();
}
