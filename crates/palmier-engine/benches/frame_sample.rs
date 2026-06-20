//! Per-frame composition SAMPLE benchmark — FOUNDATION §11.4 (E5-S11 gate).
//!
//! `composition_build.rs` benches the *structural* build at 50/200/1000 clips.
//! This bench isolates the **animated-property sampler** cost the story calls for —
//! "per-frame sampling for animated clips" — by sweeping the sampled frame across a
//! fully-keyframed timeline and timing both:
//!   * `build_frame` (full rebuild + sample) at a swept frame, and
//!   * `refresh_visuals` (the risk #8 fast path: re-sample transform/opacity/crop on
//!     an existing layer skeleton WITHOUT a structural rebuild).
//!
//! Holding it next to the build bench makes the two-tier split (rebuild vs. revisit)
//! legible as a perf baseline. Pure-CPU; no GPU/device, always buildable + runnable
//! (`cargo bench --no-run`). Run:
//! `pwsh -File scripts/with-msvc.ps1 cargo bench --package palmier-engine --bench frame_sample`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use palmier_engine::composition::{build_frame, refresh_visuals, SourceInfo, SourceResolver};
use palmier_model::{
    AnimPair, Clip, ClipType, Crop, Interpolation, Keyframe, KeyframeTrack, Timeline, Track,
};

/// A timeline whose every clip carries the full animatable property set (position,
/// scale, rotation, opacity, crop) on Smooth/Linear/Hold tracks — so sampling at any
/// frame exercises every interpolation path the reference uses. Clips are laid
/// end-to-end (no on-track overlap) across `track_count` tracks.
fn animated_timeline(clip_count: usize, track_count: usize) -> Timeline {
    let mut tl = Timeline::new();
    tl.fps = 30;
    tl.width = 1920;
    tl.height = 1080;

    let dur = 120;
    let per_track = clip_count.div_ceil(track_count);
    for t in 0..track_count {
        let mut track = Track::new(ClipType::Video);
        for i in 0..per_track {
            let global = t * per_track + i;
            if global >= clip_count {
                break;
            }
            let start = (i as i32) * dur;
            let mut clip = Clip::new(format!("media-{global}"), start, dur);
            clip.id = format!("clip-{global}");

            let mut pos = KeyframeTrack::new();
            pos.upsert(Keyframe::with_interpolation(
                0,
                AnimPair::new(0.0, 0.0),
                Interpolation::Linear,
            ));
            pos.upsert(Keyframe::with_interpolation(
                dur,
                AnimPair::new(0.5, 0.5),
                Interpolation::Linear,
            ));
            clip.position_track = Some(pos);

            let mut scale = KeyframeTrack::new();
            scale.upsert(Keyframe::with_interpolation(
                0,
                AnimPair::new(0.5, 0.5),
                Interpolation::Smooth,
            ));
            scale.upsert(Keyframe::with_interpolation(
                dur,
                AnimPair::new(1.0, 1.0),
                Interpolation::Smooth,
            ));
            clip.scale_track = Some(scale);

            let mut rot = KeyframeTrack::new();
            rot.upsert(Keyframe::with_interpolation(0, 0.0_f64, Interpolation::Linear));
            rot.upsert(Keyframe::with_interpolation(dur, 90.0_f64, Interpolation::Linear));
            clip.rotation_track = Some(rot);

            let mut op = KeyframeTrack::new();
            op.upsert(Keyframe::with_interpolation(0, 0.0_f64, Interpolation::Smooth));
            op.upsert(Keyframe::with_interpolation(dur / 2, 1.0_f64, Interpolation::Smooth));
            op.upsert(Keyframe::with_interpolation(dur, 0.0_f64, Interpolation::Smooth));
            clip.opacity_track = Some(op);

            let mut crop = KeyframeTrack::new();
            crop.upsert(Keyframe::with_interpolation(0, Crop::default(), Interpolation::Smooth));
            crop.upsert(Keyframe::with_interpolation(
                dur,
                Crop { left: 0.1, top: 0.1, right: 0.1, bottom: 0.1 },
                Interpolation::Smooth,
            ));
            clip.crop_track = Some(crop);

            track.clips.push(clip);
        }
        tl.tracks.push(track);
    }
    tl
}

fn resolver() -> impl SourceResolver {
    |_r: &str| Some(SourceInfo::upright((1920.0, 1080.0)))
}

fn bench_frame_sample(c: &mut Criterion) {
    let res = resolver();
    let track_count = 5;

    // Full rebuild+sample, swept across the keyframed range so the bench averages over
    // on-key / between-key / smooth-midpoint sample positions.
    let mut build = c.benchmark_group("frame_sample/build");
    for &clip_count in &[50usize, 200] {
        let tl = animated_timeline(clip_count, track_count);
        build.bench_with_input(
            BenchmarkId::from_parameter(clip_count),
            &clip_count,
            |b, _| {
                let mut frame = 0i32;
                b.iter(|| {
                    frame = (frame + 7) % 120;
                    let cf = build_frame(&tl, frame, &res);
                    std::hint::black_box(cf);
                });
            },
        );
    }
    build.finish();

    // The risk #8 fast path: re-sample visuals on an existing frame graph (no
    // structural rebuild). Built once outside the timed loop.
    let mut visuals = c.benchmark_group("frame_sample/refresh_visuals");
    for &clip_count in &[50usize, 200] {
        let tl = animated_timeline(clip_count, track_count);
        let mut cf = build_frame(&tl, 30, &res);
        visuals.bench_with_input(
            BenchmarkId::from_parameter(clip_count),
            &clip_count,
            |b, _| {
                let mut frame = 0i32;
                b.iter(|| {
                    frame = (frame + 7) % 120;
                    cf.frame_index = frame;
                    refresh_visuals(&mut cf, &tl, &res);
                    std::hint::black_box(&cf);
                });
            },
        );
    }
    visuals.finish();
}

criterion_group!(benches, bench_frame_sample);
criterion_main!(benches);
