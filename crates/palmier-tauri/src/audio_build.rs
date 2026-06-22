//! Shared timeline → mixer-input builder for BOTH preview playback and video export.
//!
//! ## Why this module exists
//! Real-time preview ([`crate::preview_audio`]) and offline video export
//! ([`crate::export`]) need the SAME thing: turn the active timeline's audio-bearing
//! clips into the engine mixer's per-track clip-audio input — decoding each clip's PCM,
//! slicing/retiming it to the clip's visible window, and carrying the volume / fade /
//! speed / dB-keyframe / mute envelope. Preview feeds the result to the cpal player
//! ([`mix_to_stereo_bus`](palmier_engine::audio::mix_to_stereo_bus)); export feeds it to
//! the render's AAC muxer ([`mix_to_bus`](palmier_engine::audio::mix_to_bus)).
//!
//! Both used to duplicate this construction (preview did; export passed an EMPTY input →
//! video-only files). This module is the single source of truth: the
//! decode/slice/retime/envelope mapping lives here ONCE, and the two callers project it
//! into the channel layout each needs.
//!
//! ## Channel layout
//! - **Preview** consumes `StereoClipAudio` (separate L/R buffers) — [`build_stereo`].
//! - **Export** consumes mono `ClipAudio` — the render's `AudioInput` mixes a single
//!   mono bus and duplicates it to both AAC channels — [`build_mono`]. The mono buffer
//!   is the per-sample average of L+R (a standard stereo→mono downmix).
//!
//! Both share [`build_clip_audios`], which does the per-clip decode + slice + envelope
//! mapping and yields the stereo slices; the mono path averages the two channels.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use palmier_engine::audio::envelope::{AudioClip, VolumeKeyframe};
use palmier_engine::audio::{AudioTrack, ClipAudio, StereoClipAudio};
use palmier_media::{AudioPcmCache, DecodedAudio, TARGET_SAMPLE_RATE_HZ};
use palmier_model::{Clip, ClipType, MediaSource, Timeline};

/// Just what the mixer-input build needs out of the shared `EditorState`: the timeline
/// + the `media_ref → absolute path` map (the same resolution preview/export snapshot).
/// Built under the executor lock by each caller; the decode/mix runs lock-free after.
pub(crate) struct AudioBuildInput {
    pub timeline: Timeline,
    pub urls: HashMap<String, PathBuf>,
}

/// Resolve a [`MediaSource`] to an absolute path (shared by preview + export snapshots).
pub(crate) fn asset_path(source: &MediaSource) -> Option<PathBuf> {
    let raw = match source {
        MediaSource::External { absolute_path } => absolute_path,
        MediaSource::Project { relative_path } => relative_path,
    };
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

/// Map a model [`Clip`] to the engine's [`AudioClip`] (the envelope/fade/speed
/// projection the mixer consumes). Volume-track keyframes (dB, clip-relative) carry over
/// 1:1; static linear volume / fades / speed pass through.
pub(crate) fn to_audio_clip(clip: &Clip) -> AudioClip {
    let volume_keyframes = clip
        .volume_track
        .as_ref()
        .map(|t| {
            t.keyframes
                .iter()
                .map(|kf| VolumeKeyframe {
                    frame: kf.frame,
                    db: kf.value,
                    interpolation_out: kf.interpolation_out,
                })
                .collect()
        })
        .unwrap_or_default();

    AudioClip {
        start_frame: clip.start_frame,
        duration_frames: clip.duration_frames,
        volume: clip.volume,
        speed: if clip.speed == 0.0 { 1.0 } else { clip.speed },
        volume_keyframes,
        fade_in_frames: clip.fade_in_frames,
        fade_out_frames: clip.fade_out_frames,
        fade_in_interpolation: clip.fade_in_interpolation,
        fade_out_interpolation: clip.fade_out_interpolation,
    }
}

/// Frames → 48 kHz sample-frame count (ties-away, matching the engine retime).
pub(crate) fn frames_to_samples(frames: i32, fps: i32) -> usize {
    if frames <= 0 || fps <= 0 {
        return 0;
    }
    (frames as f64 / fps as f64 * TARGET_SAMPLE_RATE_HZ as f64).round() as usize
}

/// Extract one channel's samples for a clip's VISIBLE portion from the asset's decoded
/// audio, retimed to the clip's timeline length. Linear-resamples when the source length
/// (after trim) differs from the played length (speed != 1.0). Pads with silence if the
/// asset is shorter than the clip needs.
pub(crate) fn slice_channel(
    decoded: &DecodedAudio,
    clip: &Clip,
    fps: i32,
    channel: usize,
) -> Vec<f32> {
    let out_len = frames_to_samples(clip.duration_frames, fps);
    if out_len == 0 {
        return Vec::new();
    }
    let full = decoded.channel(channel);
    if full.is_empty() {
        return vec![0.0; out_len];
    }

    // Source window: skip `trim_start_frame`, read `source_frames_consumed()` frames.
    let src_start = frames_to_samples(clip.trim_start_frame, fps);
    let src_len = frames_to_samples(clip.source_frames_consumed(), fps).max(1);

    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        // Position within the source window for output sample i (linear retime of the
        // source window onto the played length).
        let src_pos = src_start as f64 + (i as f64 / out_len as f64) * src_len as f64;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = full.get(idx).copied().unwrap_or(0.0);
        let b = full.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// The per-track build core: for each track, decode + slice each audio-bearing clip into
/// its [`StereoClipAudio`] (the engine `AudioClip` envelope + L/R sample buffers). Audio-
/// bearing clips are VIDEO + AUDIO types; visual-only clips (image/text/lottie) and clips
/// whose asset has no decodable audio are skipped. Decode happens here on the calling
/// (blocking) thread, cached per asset.
///
/// Returns `(tracks, output_frames)` where `output_frames` is the total bus length in
/// sample-frames (the timeline length). This is the single shared mapping both the
/// preview ([`build_stereo`]) and export ([`build_mono`]) paths build on.
///
/// `decode` resolves an absolute path to that asset's decoded PCM (typically
/// `|p| cache.get(p)`); injecting it as a closure keeps this core testable without a
/// real file decode.
pub(crate) fn build_clip_audios(
    input: &AudioBuildInput,
    decode: impl Fn(&Path) -> Option<Arc<DecodedAudio>>,
) -> (Vec<(AudioTrack, Vec<StereoClipAudio>)>, usize) {
    let fps = input.timeline.fps.max(1);
    let total_frames = input.timeline.total_frames().max(0);
    let output_frames = frames_to_samples(total_frames, fps);

    let mut tracks: Vec<(AudioTrack, Vec<StereoClipAudio>)> = Vec::new();
    for track in &input.timeline.tracks {
        let mut clip_audios: Vec<StereoClipAudio> = Vec::new();
        for clip in &track.clips {
            // Only VIDEO/AUDIO clips carry an audio stream.
            if !matches!(clip.media_type, ClipType::Video | ClipType::Audio) {
                continue;
            }
            let Some(path) = input.urls.get(&clip.media_ref) else {
                continue; // offline / unresolvable asset → no audio for this clip.
            };
            let Some(decoded) = decode(path) else {
                continue; // no decodable audio (silent video / image) → skip.
            };
            let left = slice_channel(&decoded, clip, fps, 0);
            let right = slice_channel(&decoded, clip, fps, 1);
            if left.is_empty() {
                continue;
            }
            clip_audios.push(StereoClipAudio {
                clip: to_audio_clip(clip),
                left,
                right,
            });
        }
        if !clip_audios.is_empty() {
            tracks.push((
                AudioTrack {
                    muted: track.muted,
                    clips: clip_audios.iter().map(|c| c.clip.clone()).collect(),
                },
                clip_audios,
            ));
        }
    }
    (tracks, output_frames)
}

/// Build the **stereo** mixer input for real-time PREVIEW playback (the cpal player
/// consumes `StereoClipAudio` and mixes via `mix_to_stereo_bus`). Thin alias over
/// [`build_clip_audios`].
pub(crate) fn build_stereo(
    input: &AudioBuildInput,
    cache: &AudioPcmCache,
) -> (Vec<(AudioTrack, Vec<StereoClipAudio>)>, usize) {
    build_clip_audios(input, |p| cache.get(p))
}

/// Build the **mono** mixer input for video EXPORT — the render's
/// `AudioInput = Vec<(AudioTrack, Vec<ClipAudio>)>`. The render mixes a single mono bus
/// (`mix_to_bus`) and duplicates it to both AAC channels, so we downmix each clip's
/// stereo slices to mono (per-sample average of L+R, the standard stereo→mono downmix).
///
/// Returns `(tracks, output_frames)`; `output_frames` is the 48 kHz timeline length.
pub(crate) fn build_mono(
    input: &AudioBuildInput,
    cache: &AudioPcmCache,
) -> (Vec<(AudioTrack, Vec<ClipAudio>)>, usize) {
    let (stereo_tracks, output_frames) = build_clip_audios(input, |p| cache.get(p));
    let tracks = stereo_tracks
        .into_iter()
        .map(|(track, stereo_clips)| {
            let mono_clips = stereo_clips
                .into_iter()
                .map(|sc| ClipAudio {
                    clip: sc.clip,
                    samples: downmix_to_mono(&sc.left, &sc.right),
                })
                .collect();
            (track, mono_clips)
        })
        .collect();
    (tracks, output_frames)
}

/// Average two equal-length channel buffers to a single mono buffer (`(L+R)/2`). Falls
/// back to the longer buffer's sample (treating the missing channel as equal) so a
/// degenerate mismatch never panics or truncates audio.
fn downmix_to_mono(left: &[f32], right: &[f32]) -> Vec<f32> {
    let len = left.len().max(right.len());
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let l = left.get(i).copied();
        let r = right.get(i).copied();
        let s = match (l, r) {
            (Some(l), Some(r)) => 0.5 * (l + r),
            (Some(l), None) => l,
            (None, Some(r)) => r,
            (None, None) => 0.0,
        };
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::Track;

    fn decoded_ramp(frames: usize) -> DecodedAudio {
        // Stereo: L = i, R = -i.
        let mut interleaved = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            interleaved.push(i as f32);
            interleaved.push(-(i as f32));
        }
        DecodedAudio {
            interleaved,
            channels: 2,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
        }
    }

    #[test]
    fn frames_to_samples_matches_rate() {
        // 30 fps, 30 frames = 1 s = 48000 samples.
        assert_eq!(frames_to_samples(30, 30), 48_000);
        assert_eq!(frames_to_samples(0, 30), 0);
        assert_eq!(frames_to_samples(30, 0), 0);
    }

    #[test]
    fn to_audio_clip_carries_volume_keyframes_in_db() {
        use palmier_model::{Interpolation, Keyframe, KeyframeTrack};
        let mut clip = Clip::new("asset-1", 0, 30);
        clip.volume = 0.5;
        clip.fade_in_frames = 5;
        let mut track = KeyframeTrack::new();
        track.upsert(Keyframe::with_interpolation(0, -6.0, Interpolation::Linear));
        track.upsert(Keyframe::with_interpolation(15, 0.0, Interpolation::Smooth));
        clip.volume_track = Some(track);

        let ac = to_audio_clip(&clip);
        assert_eq!(ac.volume, 0.5);
        assert_eq!(ac.fade_in_frames, 5);
        assert_eq!(ac.volume_keyframes.len(), 2);
        assert_eq!(ac.volume_keyframes[0].db, -6.0);
        assert_eq!(ac.volume_keyframes[1].frame, 15);
    }

    #[test]
    fn slice_channel_yields_clip_length_and_picks_channel() {
        // Asset is 60 frames of stereo ramp; clip is 30 frames at speed 1.0, no trim.
        let decoded = decoded_ramp(frames_to_samples(60, 30));
        let clip = Clip::new("asset-1", 0, 30);
        let left = slice_channel(&decoded, &clip, 30, 0);
        let right = slice_channel(&decoded, &clip, 30, 1);
        let expect_len = frames_to_samples(30, 30);
        assert_eq!(left.len(), expect_len);
        assert_eq!(right.len(), expect_len);
        assert_eq!(left[0], 0.0);
        assert!(right[100] < 0.0 && (right[100] + left[100]).abs() < 1e-3);
    }

    #[test]
    fn downmix_averages_channels() {
        let l = vec![1.0, 0.0, -1.0];
        let r = vec![1.0, 1.0, 1.0];
        assert_eq!(downmix_to_mono(&l, &r), vec![1.0, 0.5, 0.0]);
        // Length mismatch falls back to the present channel.
        assert_eq!(downmix_to_mono(&[2.0], &[]), vec![2.0]);
    }

    /// A timeline carrying one audio-bearing clip whose asset decodes must yield a
    /// NON-EMPTY export AudioInput (≥1 track with samples). This is the property the
    /// orchestrator's ffprobe-for-an-audio-stream check rests on: the shared build must
    /// hand the render real audio, not an empty slice (the old video-only behavior). The
    /// decode is injected as a closure so no real file is needed.
    #[test]
    fn build_yields_nonempty_input_for_audio_clip() {
        // One AUDIO track with one clip referencing asset "a1".
        let mut tl = Timeline::default();
        tl.fps = 30;
        let mut atrack = Track::new(ClipType::Audio);
        let mut clip = Clip::new("a1", 0, 30);
        clip.media_type = ClipType::Audio;
        atrack.clips.push(clip);
        tl.tracks.push(atrack);

        // Resolve "a1" to a path; the decode closure returns PCM for that path.
        let path = PathBuf::from("/virtual/a1.wav");
        let mut urls = HashMap::new();
        urls.insert("a1".to_string(), path.clone());
        let pcm = Arc::new(decoded_ramp(frames_to_samples(60, 30)));
        let decode = move |p: &Path| (p == path).then(|| Arc::clone(&pcm));

        let input = AudioBuildInput { timeline: tl, urls };

        // Stereo (preview) path: one track, one stereo clip with L/R samples.
        let (stereo_tracks, output_frames) = build_clip_audios(&input, &decode);
        assert_eq!(output_frames, frames_to_samples(30, 30));
        assert_eq!(stereo_tracks.len(), 1, "one audio track emitted");
        let (s_track, s_clips) = &stereo_tracks[0];
        assert!(!s_track.muted);
        assert_eq!(s_clips.len(), 1);
        assert!(!s_clips[0].left.is_empty() && !s_clips[0].right.is_empty());

        // Mono (export) projection: downmix L/R → non-empty mono samples (the render's
        // AudioInput). This is what export now passes instead of an empty slice.
        let mono_tracks: Vec<(AudioTrack, Vec<ClipAudio>)> = stereo_tracks
            .into_iter()
            .map(|(track, clips)| {
                let mono = clips
                    .into_iter()
                    .map(|sc| ClipAudio {
                        clip: sc.clip,
                        samples: downmix_to_mono(&sc.left, &sc.right),
                    })
                    .collect();
                (track, mono)
            })
            .collect();
        assert_eq!(mono_tracks.len(), 1, "one track in the export AudioInput");
        let (_m_track, m_clips) = &mono_tracks[0];
        assert_eq!(m_clips.len(), 1);
        assert!(!m_clips[0].samples.is_empty(), "clip carries mono samples");
        assert_eq!(m_clips[0].samples.len(), frames_to_samples(30, 30));
    }

    #[test]
    fn build_mono_skips_visual_only_and_offline() {
        // TEXT clip (no audio) + AUDIO clip whose asset is offline → no tracks.
        let mut tl = Timeline::default();
        tl.fps = 30;
        let mut vtrack = Track::new(ClipType::Text);
        let mut text_clip = Clip::new("text-asset", 0, 30);
        text_clip.media_type = ClipType::Text;
        vtrack.clips.push(text_clip);
        tl.tracks.push(vtrack);

        let mut atrack = Track::new(ClipType::Audio);
        let mut audio_clip = Clip::new("missing-asset", 0, 30);
        audio_clip.media_type = ClipType::Audio;
        atrack.clips.push(audio_clip);
        tl.tracks.push(atrack);

        let input = AudioBuildInput {
            timeline: tl,
            urls: HashMap::new(), // nothing resolves → offline.
        };
        let cache = AudioPcmCache::new();
        let (tracks, output_frames) = build_mono(&input, &cache);
        assert!(tracks.is_empty());
        assert_eq!(output_frames, frames_to_samples(30, 30));
    }

    #[test]
    fn asset_path_resolves_external_and_skips_empty() {
        let ext = MediaSource::External {
            absolute_path: "/clip.mp4".into(),
        };
        assert_eq!(asset_path(&ext), Some(PathBuf::from("/clip.mp4")));
        let empty = MediaSource::External {
            absolute_path: String::new(),
        };
        assert_eq!(asset_path(&empty), None);
    }
}
