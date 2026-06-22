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
use std::sync::Arc;

use palmier_engine::audio::AudioPlayer;
use palmier_media::AudioPcmCache;
use tauri::State;

use crate::agent::AgentState;
use crate::audio_build::{asset_path, build_stereo, AudioBuildInput};

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

/// Pull the live timeline + media-path map out of the shared executor into the SHARED
/// [`AudioBuildInput`] (the same `media_ref → path` resolution `preview_render` uses).
/// Taken under the executor lock; the decode/mix runs lock-free after.
fn snapshot(agent: &AgentState) -> AudioBuildInput {
    agent.executor.with_state_ref(|state| {
        let timeline = state.library.timeline.clone();
        let mut urls = HashMap::new();
        for asset in &state.library.assets {
            if let Some(path) = asset_path(&asset.source) {
                urls.insert(asset.id.clone(), path);
            }
        }
        AudioBuildInput { timeline, urls }
    })
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
        let (tracks, output_frames) = build_stereo(&snap, &cache);
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

// Unit coverage for the timeline → mixer-input build (frames_to_samples, to_audio_clip,
// slice_channel, the per-track build, and the export mono projection) lives with the
// shared helper in `crate::audio_build` — this module is now just the playback transport
// + Tauri command surface over that helper.
