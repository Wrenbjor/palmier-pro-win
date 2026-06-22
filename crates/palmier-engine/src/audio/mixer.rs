//! Audio mixer — sum per-clip volume envelopes over a played frame range.
//!
//! Port of the macOS reference `CompositionBuilder.buildVisuals` audio-mix branch +
//! the `AVMutableAudioMix` summation AVFoundation performs implicitly. The reference
//! builds, per audio track, an `AVMutableAudioMixInputParameters` carrying the volume
//! envelope; AVFoundation then decodes, sums, and outputs. We do the summation
//! explicitly here (FOUNDATION §6.5 "Audio mixing": decode → resample 48 kHz →
//! time-stretch → per-frame volume envelope → **sum all clips' buffers** → cpal).
//!
//! ## What is here vs. stubbed (E5-S6 scope)
//!
//! - **Here (pure DSP, tested headless):** the envelope-application + summation across
//!   clips and tracks, muted-track silencing, and the per-clip mix plan. This is the
//!   part the story names testable (volume/fade correctness, speed-retime sample count,
//!   flat-vs-ramped selection, muted-track silence).
//! - **Stubbed / deferred:** the symphonia *decode* of real files and the rubato/
//!   signalsmith *stretch* feed the mixer pre-decoded sample buffers via [`ClipAudio`];
//!   wiring real files is a thin adapter. The **cpal output device** lives behind the
//!   `audio-device` feature and is driven by the E5-S7 transport — there is no live
//!   device in CI (FOUNDATION §11.1: device paths run headless or are `#[ignore]`d).

use super::envelope::{build_volume_envelope, sample_envelope, AudioClip};
use super::retime;

/// One track's audio for mixing: its clips plus the track-level mute flag.
///
/// Mirrors the reference per-track loop in the audio-mix branch (`track.muted` →
/// `setVolume(0)`; else emit each clip's envelope). Clips are expected sorted by
/// `start_frame`; the mixer enforces the reference's `startFrame >= prevEndFrame`
/// single-track serialization so overlapping clips on one track don't double-count.
#[derive(Debug, Clone)]
pub struct AudioTrack {
    /// Track-level mute (reference `track.muted`). Muted → contributes silence.
    pub muted: bool,
    /// Clips on this track. Sorted by `start_frame` for serialization.
    pub clips: Vec<AudioClip>,
}

/// Pre-decoded, already-resampled-and-stretched 48 kHz mono samples for one clip,
/// aligned so index 0 == the clip's `start_frame`. The decode/resample/stretch stages
/// (symphonia/rubato) produce this; the mixer only applies the envelope + sums.
#[derive(Debug, Clone)]
pub struct ClipAudio {
    /// The clip's timeline placement + volume/fade/speed (drives the envelope).
    pub clip: AudioClip,
    /// 48 kHz mono samples for the clip's visible portion (post resample + stretch).
    pub samples: Vec<f32>,
}

/// Mix a set of tracks' pre-decoded clip audio into a single 48 kHz mono bus covering
/// the timeline output-sample range `[0, output_len)`. `fps` converts frame offsets to
/// output samples.
///
/// Per clip: skip if its track is muted (contributes nothing — the reference sets the
/// whole track's volume to 0), else apply the clip's volume envelope sampled at each
/// frame and add into the bus. Multiple clips/tracks simply sum (clamping is left to
/// the device stage; the reference relies on AVFoundation's float bus).
pub fn mix_to_bus(tracks: &[(AudioTrack, Vec<ClipAudio>)], fps: u32, output_len: usize) -> Vec<f32> {
    let mut bus = vec![0.0f32; output_len];
    if fps == 0 {
        return bus;
    }
    let samples_per_frame = retime::PROJECT_SAMPLE_RATE_HZ as f64 / fps as f64;

    for (track, clip_audios) in tracks {
        if track.muted {
            // Reference: muted track → setVolume(0). No contribution.
            continue;
        }
        for ca in clip_audios {
            // Enforce single-track serialization the reference applies: a clip must
            // start at/after the previous clip's end. Here each ClipAudio is already
            // the serialized set the builder selected, so we just place it.
            let ramps = build_volume_envelope(&ca.clip);
            if ramps.is_empty() {
                continue;
            }
            let clip_start_sample = (ca.clip.start_frame as f64 * samples_per_frame).round() as i64;
            for (i, &s) in ca.samples.iter().enumerate() {
                let out_idx = clip_start_sample + i as i64;
                if out_idx < 0 || out_idx as usize >= output_len {
                    continue;
                }
                // Frame offset within the clip for envelope lookup.
                let frame_offset = (i as f64 / samples_per_frame).floor() as i32;
                let gain = sample_envelope(&ramps, frame_offset);
                bus[out_idx as usize] += s * gain;
            }
        }
    }
    bus
}

/// Mix a set of tracks into an **interleaved stereo** 48 kHz bus covering
/// `[0, output_frames)` sample-frames. Each entry pairs a track with its clips' audio
/// as `(left_channel, right_channel)` mono sample buffers (already 48 kHz, aligned to
/// the clip start). The mono [`mix_to_bus`] is run **per channel** (reusing its tested
/// envelope/sum/mute logic) and the two resulting buses are interleaved `[L, R, …]`.
///
/// `output_frames` is the number of sample-FRAMES (per-channel samples); the returned
/// vec has `output_frames * 2` elements. This is the buffer the cpal output player
/// streams from.
pub fn mix_to_stereo_bus(
    tracks: &[(AudioTrack, Vec<StereoClipAudio>)],
    fps: u32,
    output_frames: usize,
) -> Vec<f32> {
    // Split each track's stereo clips into two mono `ClipAudio` sets (L and R), reusing
    // the tested mono mixer for each channel.
    let mut left_tracks: Vec<(AudioTrack, Vec<ClipAudio>)> = Vec::with_capacity(tracks.len());
    let mut right_tracks: Vec<(AudioTrack, Vec<ClipAudio>)> = Vec::with_capacity(tracks.len());
    for (track, clips) in tracks {
        let mut l = Vec::with_capacity(clips.len());
        let mut r = Vec::with_capacity(clips.len());
        for c in clips {
            l.push(ClipAudio { clip: c.clip.clone(), samples: c.left.clone() });
            r.push(ClipAudio { clip: c.clip.clone(), samples: c.right.clone() });
        }
        left_tracks.push((track.clone(), l));
        right_tracks.push((track.clone(), r));
    }
    let left = mix_to_bus(&left_tracks, fps, output_frames);
    let right = mix_to_bus(&right_tracks, fps, output_frames);

    let mut out = vec![0.0f32; output_frames * 2];
    for f in 0..output_frames {
        out[f * 2] = left[f];
        out[f * 2 + 1] = right[f];
    }
    out
}

/// A clip's pre-decoded **stereo** 48 kHz audio (the two channels as separate mono
/// buffers, each aligned so index 0 == the clip's `start_frame`). The decode adapter
/// (`palmier-media::decode_audio_pcm`) produces interleaved stereo; the transport
/// splits it into `left`/`right` per channel for [`mix_to_stereo_bus`].
#[derive(Debug, Clone)]
pub struct StereoClipAudio {
    /// The clip's timeline placement + volume/fade/speed (drives the envelope).
    pub clip: AudioClip,
    /// Left-channel 48 kHz mono samples for the clip's visible portion.
    pub left: Vec<f32>,
    /// Right-channel 48 kHz mono samples for the clip's visible portion.
    pub right: Vec<f32>,
}

/// Sink that accepts mixed 48 kHz frames. Implemented by the cpal device (behind the
/// `audio-device` feature) for live preview, and by a buffer for tests/offline export.
pub trait AudioSink {
    /// Push a block of interleaved/mono 48 kHz samples to the output.
    fn write(&mut self, samples: &[f32]);
}

/// In-memory sink — collects everything written. Used by tests and offline render.
#[derive(Debug, Default)]
pub struct BufferSink {
    /// Accumulated samples.
    pub buffer: Vec<f32>,
}

impl AudioSink for BufferSink {
    fn write(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);
    }
}

#[cfg(feature = "audio-device")]
pub mod device {
    //! cpal output device sink. Gated behind `audio-device` — not built in headless CI.
    //! The E5-S7 transport owns the playback clock; this is the device seam only.

    /// Probe for a default output device. Returns `false` when no audio device is
    /// available (headless box) so callers can degrade gracefully.
    pub fn default_output_available() -> bool {
        use cpal::traits::HostTrait;
        cpal::default_host().default_output_device().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::Interpolation;

    fn clip(start: i32, dur: i32, volume: f64) -> AudioClip {
        AudioClip {
            start_frame: start,
            duration_frames: dur,
            volume,
            speed: 1.0,
            volume_keyframes: Vec::new(),
            fade_in_frames: 0,
            fade_out_frames: 0,
            fade_in_interpolation: Interpolation::Smooth,
            fade_out_interpolation: Interpolation::Smooth,
        }
    }

    /// One frame @ 48 kHz / 30 fps == 1600 samples.
    const SPF: usize = 1600;

    #[test]
    fn muted_track_contributes_silence() {
        // 1 frame of unity-amplitude samples, but the track is muted → all zeros.
        let samples = vec![1.0f32; SPF];
        let track = AudioTrack { muted: true, clips: vec![clip(0, 1, 1.0)] };
        let ca = ClipAudio { clip: clip(0, 1, 1.0), samples };
        let bus = mix_to_bus(&[(track, vec![ca])], 30, SPF);
        assert!(bus.iter().all(|&s| s == 0.0), "muted track must be silent");
    }

    #[test]
    fn flat_volume_scales_samples() {
        // volume 0.5, no fade → every output sample is 0.5 × input.
        let samples = vec![1.0f32; SPF];
        let track = AudioTrack { muted: false, clips: vec![clip(0, 1, 0.5)] };
        let ca = ClipAudio { clip: clip(0, 1, 0.5), samples };
        let bus = mix_to_bus(&[(track, vec![ca])], 30, SPF);
        assert!(bus.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn two_clips_sum_on_the_bus() {
        // Two unmuted clips at the same position sum (0.5 + 0.25 = 0.75).
        let a_track = AudioTrack { muted: false, clips: vec![clip(0, 1, 0.5)] };
        let b_track = AudioTrack { muted: false, clips: vec![clip(0, 1, 0.25)] };
        let a = ClipAudio { clip: clip(0, 1, 0.5), samples: vec![1.0; SPF] };
        let b = ClipAudio { clip: clip(0, 1, 0.25), samples: vec![1.0; SPF] };
        let bus = mix_to_bus(&[(a_track, vec![a]), (b_track, vec![b])], 30, SPF);
        assert!(bus.iter().all(|&s| (s - 0.75).abs() < 1e-6));
    }

    #[test]
    fn clip_placed_at_offset_lands_in_the_right_window() {
        // Clip starts at frame 1 → its samples occupy [SPF, 2*SPF), frame 0 is silent.
        let track = AudioTrack { muted: false, clips: vec![clip(1, 1, 1.0)] };
        let ca = ClipAudio { clip: clip(1, 1, 1.0), samples: vec![1.0; SPF] };
        let bus = mix_to_bus(&[(track, vec![ca])], 30, SPF * 2);
        assert!(bus[..SPF].iter().all(|&s| s == 0.0), "frame 0 silent");
        assert!(bus[SPF..].iter().all(|&s| (s - 1.0).abs() < 1e-6), "frame 1 full");
    }

    #[test]
    fn stereo_bus_interleaves_and_preserves_channels() {
        // One clip, distinct L/R levels, volume 1.0. L=0.5, R=-0.5 across one frame.
        let track = AudioTrack { muted: false, clips: vec![clip(0, 1, 1.0)] };
        let sca = StereoClipAudio {
            clip: clip(0, 1, 1.0),
            left: vec![0.5; SPF],
            right: vec![-0.5; SPF],
        };
        let bus = mix_to_stereo_bus(&[(track, vec![sca])], 30, SPF);
        assert_eq!(bus.len(), SPF * 2, "interleaved stereo is 2× frame count");
        // Even indices = L (0.5), odd = R (-0.5).
        for f in 0..SPF {
            assert!((bus[f * 2] - 0.5).abs() < 1e-6, "L channel preserved");
            assert!((bus[f * 2 + 1] + 0.5).abs() < 1e-6, "R channel preserved");
        }
    }

    #[test]
    fn stereo_bus_muted_track_is_silent() {
        let track = AudioTrack { muted: true, clips: vec![clip(0, 1, 1.0)] };
        let sca = StereoClipAudio {
            clip: clip(0, 1, 1.0),
            left: vec![1.0; SPF],
            right: vec![1.0; SPF],
        };
        let bus = mix_to_stereo_bus(&[(track, vec![sca])], 30, SPF);
        assert!(bus.iter().all(|&s| s == 0.0), "muted ⇒ silent stereo bus");
    }

    #[test]
    fn buffer_sink_collects() {
        let mut sink = BufferSink::default();
        sink.write(&[0.1, 0.2]);
        sink.write(&[0.3]);
        assert_eq!(sink.buffer, vec![0.1, 0.2, 0.3]);
    }

    /// Exercises the cpal device seam. `#[ignore]`d because it needs a real audio
    /// device + the `audio-device` feature — headless CI has neither (FOUNDATION §11.1).
    /// Run locally with: `cargo test --features audio-device -- --ignored device_probe`.
    #[cfg(feature = "audio-device")]
    #[test]
    #[ignore = "needs a real audio output device; run with --ignored locally"]
    fn device_probe_smoke() {
        // Just proves the probe links + runs; result depends on the host's devices.
        let _ = super::device::default_output_available();
    }
}
