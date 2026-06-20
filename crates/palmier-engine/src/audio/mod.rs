//! Audio mixer (E5-S6) — FOUNDATION §6.5 "Audio mixing".
//!
//! Replaces the macOS reference's `AVMutableAudioMix` pipeline with a pure-Rust mixer:
//!
//! ```text
//! symphonia decode → rubato resample 48 kHz → speed time-stretch (pitch-preserving)
//!   → per-frame volume envelope (static × keyframe × fade) → sum all clips → cpal out
//! ```
//!
//! ## Module layout
//! - [`volume_scale`] — dB ↔ linear gain (−60…+15 dB, reconciliation ruling #9).
//! - [`envelope`] — the offset-set / piecewise-linear volume envelope with 8-segment
//!   smoothstep parity (the load-bearing curve port; shared algorithm with E5-S5 opacity).
//! - [`retime`] — speed sample-count math (`f64::round` ties-away) + 48 kHz resample plan.
//! - [`mixer`] — sum per-clip envelopes across tracks into a 48 kHz bus; cpal sink seam
//!   (device behind the `audio-device` feature).
//!
//! ## Presentation-agnostic
//! Per the epic spike note, this story is deliberately presentation-agnostic — it
//! produces/consumes sample buffers and never touches wgpu or a window. It runs in
//! parallel with Spike S-1. The live cpal device + the transport clock that drives it
//! belong to E5-S7; the symphonia/rubato decode-resample-stretch *file* wiring is a
//! thin adapter over [`mixer::ClipAudio`].

pub mod envelope;
pub mod mixer;
pub mod retime;
pub mod volume_scale;

pub use envelope::{
    build_volume_envelope, sample_envelope, AudioClip, VolumeKeyframe, VolumeRamp, SMOOTH_SEGMENTS,
};
pub use mixer::{mix_to_bus, AudioSink, AudioTrack, BufferSink, ClipAudio};
pub use retime::{plan, ResamplePlan, PROJECT_SAMPLE_RATE_HZ};
pub use volume_scale::{db_from_linear, linear_from_db, CEILING_DB, FLOOR_DB};
