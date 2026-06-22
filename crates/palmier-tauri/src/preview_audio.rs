//! Real-time AUDIO playback transport — the seam that makes the timeline AUDIBLE.
//!
//! ## The product gap this closes
//! The preview showed VIDEO frames (`preview_render.rs`) but produced **no sound**.
//! This module adds the audio half: when the user presses Play it decodes the
//! timeline's audio clips, mixes them, and streams the result to the default output
//! device in real time — synchronized to the same playhead the video preview tracks.
//!
//! ## The pipeline (decode → mix → cpal)
//! ```text
//! shared EditorState snapshot (same source preview_render uses)
//!   → per audio-bearing clip: palmier-media::decode_audio_pcm (48 kHz stereo, cached)
//!   → slice to the clip's visible/trimmed portion, speed-retime to timeline length
//!   → palmier-engine::audio::mix_to_stereo_bus (envelope + fade + sum, per channel)
//!   → palmier-engine::audio::AudioPlayer (cpal output stream, real-time cursor)
//! ```
//! The mixer + player are the EXACT tested DSP/device stages from
//! `palmier-engine::audio`; this module is the timeline→buffer adapter + the Tauri
//! command surface.
//!
//! ## Where the timeline + media come from
//! The SAME shared `Arc<ToolExecutor>` (`AgentState.executor`) `preview_render_frame`
//! snapshots — so audio always reflects the live edit state and stays consistent with
//! the painted video frames. Audio is decoded from VIDEO and AUDIO clips (both can
//! carry an audio stream); image/text/lottie clips have none and are skipped.
//!
//! ## A/V sync model
//! cpal's stream clock advances the audio cursor; the video preview's rAF loop renders
//! at the wall-clock position. Both start from the same `from_frame` on Play, so they
//! stay roughly aligned (the video may lag on a slow box — audio is the smooth clock).
//! Exact sample-accurate lip-sync is NOT attempted here (preview monitoring). See the
//! sync-risk note in the story result.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use palmier_engine::audio::{AudioPlayer, AudioTrack, StereoClipAudio};
use palmier_engine::audio::envelope::{AudioClip, VolumeKeyframe};
use palmier_media::{AudioPcmCache, DecodedAudio, TARGET_SAMPLE_RATE_HZ};
use palmier_model::{Clip, ClipType, MediaSource, Timeline};
use tauri::State;

use crate::agent::AgentState;

/// Managed state: the process-lifetime cpal player + the per-asset decoded-PCM cache.
/// Built once at boot (`AudioPlayer::new()` probes the device but opens no stream).
pub struct PreviewAudioState {
    /// The real-time output player (cpal stream + cursor). Silent no-op without a device.
    pub player: Arc<AudioPlayer>,
    /// Decoded 48 kHz stereo PCM per asset path, so Play/seek don't re-decode.
    pub cache: AudioPcmCache,
}

impl PreviewAudioState {
    /// Construct the audio transport state (probes for an output device).
    #[must_use]
    pub fn new() -> Self {
        PreviewAudioState {
            player: Arc::new(AudioPlayer::new()),
            cache: AudioPcmCache::new(),
        }
    }
}

impl Default for PreviewAudioState {
    fn default() -> Self {
        PreviewAudioState::new()
    }
}

/// Snapshot of just what audio playback needs from the shared `EditorState`: the
/// timeline + the `media_ref → absolute path` map (same resolution `preview_render`
/// uses). Taken under the executor lock; the decode/mix runs lock-free after.
struct AudioSnapshot {
    timeline: Timeline,
    urls: HashMap<String, PathBuf>,
}

/// Pull the live timeline + media-path map out of the shared executor.
fn snapshot(agent: &AgentState) -> AudioSnapshot {
    agent.executor.with_state_ref(|state| {
        let timeline = state.library.timeline.clone();
        let mut urls = HashMap::new();
        for asset in &state.library.assets {
            if let Some(path) = asset_path(&asset.source) {
                urls.insert(asset.id.clone(), path);
            }
        }
        AudioSnapshot { timeline, urls }
    })
}

/// Resolve a [`MediaSource`] to an absolute path (mirrors `preview_render::asset_path`).
fn asset_path(source: &MediaSource) -> Option<PathBuf> {
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
/// projection the mixer consumes). Volume-track keyframes (dB, clip-relative) carry
/// over 1:1; static linear volume / fades / speed pass through.
fn to_audio_clip(clip: &Clip) -> AudioClip {
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
fn frames_to_samples(frames: i32, fps: i32) -> usize {
    if frames <= 0 || fps <= 0 {
        return 0;
    }
    (frames as f64 / fps as f64 * TARGET_SAMPLE_RATE_HZ as f64).round() as usize
}

/// Extract one channel's samples for a clip's VISIBLE portion from the asset's decoded
/// audio, retimed to the clip's timeline length. Linear-resamples when the source
/// length (after trim) differs from the played length (speed != 1.0). Pads with
/// silence if the asset is shorter than the clip needs.
fn slice_channel(
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

/// Build the engine mixer input (per-track stereo clip audio) from the timeline + the
/// decoded-PCM cache. Returns `(tracks, output_frames)` where `output_frames` is the
/// total bus length in sample-frames (the timeline length).
///
/// Audio-bearing clips are VIDEO + AUDIO types; visual-only clips (image/text/lottie)
/// and clips whose asset has no decodable audio are skipped. Decode happens here on the
/// calling (blocking) thread, cached per asset.
fn build_mixer_input(
    snap: &AudioSnapshot,
    cache: &AudioPcmCache,
) -> (Vec<(AudioTrack, Vec<StereoClipAudio>)>, usize) {
    let fps = snap.timeline.fps.max(1);
    let total_frames = snap.timeline.total_frames().max(0);
    let output_frames = frames_to_samples(total_frames, fps);

    let mut tracks: Vec<(AudioTrack, Vec<StereoClipAudio>)> = Vec::new();
    for track in &snap.timeline.tracks {
        let mut clip_audios: Vec<StereoClipAudio> = Vec::new();
        for clip in &track.clips {
            // Only VIDEO/AUDIO clips carry an audio stream.
            if !matches!(clip.media_type, ClipType::Video | ClipType::Audio) {
                continue;
            }
            let Some(path) = snap.urls.get(&clip.media_ref) else {
                continue; // offline / unresolvable asset → no audio for this clip.
            };
            let Some(decoded) = cache.get(path) else {
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

// ─── Tauri command surface ─────────────────────────────────────────────────────────

/// `preview_audio_play` — decode + mix the timeline audio and begin playing from
/// `from_frame`. Runs the (blocking) decode/mix on a worker so the UI stays responsive;
/// the cpal stream then plays in the background on the device clock. A no-op (logged)
/// when there is no output device.
#[tauri::command]
pub async fn preview_audio_play(
    agent: State<'_, AgentState>,
    audio: State<'_, PreviewAudioState>,
    from_frame: i32,
) -> Result<(), String> {
    let snap = snapshot(&agent);
    let cache = audio.cache.clone();
    let player = Arc::clone(&audio.player);
    let fps = snap.timeline.fps.max(1) as u32;

    tauri::async_runtime::spawn_blocking(move || {
        let (tracks, output_frames) = build_mixer_input(&snap, &cache);
        if output_frames == 0 || tracks.is_empty() {
            // Nothing audible — make sure any prior playback stops cleanly.
            player.stop();
            tracing::debug!(target: "audio", "preview_audio_play: no audio on timeline");
            return;
        }
        player.start(from_frame as i64, &tracks, fps, output_frames);
    })
    .await
    .map_err(|e| format!("audio play task failed: {e}"))?;
    Ok(())
}

/// `preview_audio_pause` — pause playback (keeps the cursor; stream stays open for an
/// instant resume on the next play).
#[tauri::command]
pub fn preview_audio_pause(audio: State<'_, PreviewAudioState>) {
    audio.player.pause();
}

/// `preview_audio_seek` — reposition the audio cursor to `frame` (during play or while
/// paused). Cheap (no re-decode): only moves the cursor on the already-mixed bus.
#[tauri::command]
pub fn preview_audio_seek(audio: State<'_, PreviewAudioState>, frame: i32) {
    audio.player.seek(frame as i64);
}

/// `preview_audio_stop` — stop playback and release the device (teardown / unmount).
#[tauri::command]
pub fn preview_audio_stop(audio: State<'_, PreviewAudioState>) {
    audio.player.stop();
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
        // Left channel rises (i), right falls (-i): first sample 0, and right is the
        // negation of left at the same index.
        assert_eq!(left[0], 0.0);
        assert!(right[100] < 0.0 && (right[100] + left[100]).abs() < 1e-3);
    }

    #[test]
    fn slice_channel_pads_when_asset_shorter_than_clip() {
        // Asset only 10 frames, clip wants 30 → tail is the last sample (no panic),
        // length still matches the clip.
        let decoded = decoded_ramp(frames_to_samples(10, 30));
        let clip = Clip::new("asset-1", 0, 30);
        let left = slice_channel(&decoded, &clip, 30, 0);
        assert_eq!(left.len(), frames_to_samples(30, 30));
    }

    #[test]
    fn build_mixer_input_skips_visual_only_and_offline() {
        // Timeline: one TEXT clip (no audio) + one AUDIO clip whose asset is offline.
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

        let snap = AudioSnapshot {
            timeline: tl,
            urls: HashMap::new(), // nothing resolves → offline.
        };
        let cache = AudioPcmCache::new();
        let (tracks, output_frames) = build_mixer_input(&snap, &cache);
        // Text clip skipped (no audio); audio clip skipped (offline). No tracks emitted.
        assert!(tracks.is_empty());
        assert_eq!(output_frames, frames_to_samples(30, 30));
    }
}
