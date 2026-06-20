//! Composition-graph build benchmark — FOUNDATION §11.4 (E5-S4 gate).
//!
//! Benches [`build_frame`] at **50 / 200 / 1000 clips** (the 1000-clip case is the
//! explicit Epic 5 acceptance item). E5-S4 owns the benchmark; E5-S11 wires the
//! perf baseline + CI assertion. Run with:
//! `pwsh -File scripts/with-msvc.ps1 cargo bench --package palmier-engine`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use palmier_engine::composition::{build_frame, SourceInfo, SourceResolver};
use palmier_model::{Clip, ClipType, Timeline, Track};

/// A timeline of `clip_count` clips spread across `track_count` tracks, each clip
/// 30 frames long laid end-to-end on its track (no on-track overlap so all survive
/// the `previousEndFrame` gate). Half carry a position keyframe track so the
/// sampler exercises the animated path too.
fn timeline_of(clip_count: usize, track_count: usize) -> Timeline {
    use palmier_model::{AnimPair, Interpolation, Keyframe, KeyframeTrack};

    let mut tl = Timeline::new();
    tl.fps = 30;
    tl.width = 1920;
    tl.height = 1080;

    let per_track = clip_count.div_ceil(track_count);
    for t in 0..track_count {
        let mut track = Track::new(ClipType::Video);
        for i in 0..per_track {
            let global = t * per_track + i;
            if global >= clip_count {
                break;
            }
            let start = (i as i32) * 30;
            let mut clip = Clip::new(format!("media-{global}"), start, 30);
            clip.id = format!("clip-{global}");
            if global % 2 == 0 {
                let mut pos = KeyframeTrack::new();
                pos.upsert(Keyframe::with_interpolation(
                    0,
                    AnimPair::new(0.0, 0.0),
                    Interpolation::Smooth,
                ));
                pos.upsert(Keyframe::with_interpolation(
                    30,
                    AnimPair::new(0.5, 0.5),
                    Interpolation::Smooth,
                ));
                clip.position_track = Some(pos);
            }
            track.clips.push(clip);
        }
        tl.tracks.push(track);
    }
    tl
}

fn resolver() -> impl SourceResolver {
    |_r: &str| Some(SourceInfo::upright((1920.0, 1080.0)))
}

fn bench_composition_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("composition_build");
    let res = resolver();
    // A few tracks so multiple layers stack at the sampled frame.
    let track_count = 5;
    for &clip_count in &[50usize, 200, 1000] {
        let tl = timeline_of(clip_count, track_count);
        // Sample near the front where several tracks have an active clip.
        let frame = 15;
        group.bench_with_input(
            BenchmarkId::from_parameter(clip_count),
            &clip_count,
            |b, _| {
                b.iter(|| {
                    let cf = build_frame(&tl, frame, &res);
                    std::hint::black_box(cf);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_composition_build);
criterion_main!(benches);
