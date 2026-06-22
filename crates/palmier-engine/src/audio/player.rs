//! Real-time audio output player (cpal, behind `audio-device`).
//!
//! This is the live device stage the E5-S6 mixer module deferred to "the E5-S7
//! transport": a controllable cpal output stream that plays a **pre-mixed interleaved
//! stereo 48 kHz bus** to the default output device, advancing in real time on cpal's
//! own audio clock. The transport (palmier-tauri) decodes + mixes the timeline into the
//! bus (via [`mix_to_stereo_bus`](super::mixer::mix_to_stereo_bus)) and hands it here.
//!
//! ## The audio clock IS the playback clock
//! The cpal output callback pulls samples from the bus at a shared cursor (a sample-
//! frame index) and advances the cursor by exactly the number of frames it consumed.
//! So the cursor tracks real wall-clock playback at the device rate — it is the smooth,
//! continuous clock the video preview's rAF loop stays roughly in sync with (the video
//! loop renders at the wall-clock position; audio is the metronome). `current_frame()`
//! exposes the cursor as a TIMELINE frame so callers can read the audio position.
//!
//! ## Transport control
//! - [`AudioPlayer::start`] — (re)build the stream over a new bus and begin playing from
//!   a timeline frame. Replaces any prior stream.
//! - [`AudioPlayer::pause`] — keep the cursor; the callback emits silence (the stream
//!   stays open so resume is instant + glitch-free).
//! - [`AudioPlayer::resume`] — unpause from the current cursor.
//! - [`AudioPlayer::seek`] — reposition the cursor to a timeline frame (during play or
//!   paused).
//! - [`AudioPlayer::stop`] — pause + drop the stream + clear the bus (teardown).
//!
//! ## No-device degradation
//! If there is no default output device (headless CI / no audio hardware), [`new`] still
//! succeeds and every transport call is a logged no-op — audio is simply silent. This
//! mirrors FOUNDATION §11.1 (device paths degrade headlessly) so the app never fails to
//! launch or play because of audio hardware.
//!
//! [`new`]: AudioPlayer::new

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::mixer::{mix_to_stereo_bus, AudioTrack, StereoClipAudio};
use super::retime::PROJECT_SAMPLE_RATE_HZ;

/// Output channel count the player drives (stereo). The decoded/mixed bus is stereo;
/// if the device wants a different channel count we map (mono → both, >2 → first two).
const BUS_CHANNELS: usize = 2;

/// A snapshot of the player's transport state (for `status()` / tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerStatus {
    /// Whether a stream is currently open (a bus is loaded).
    pub has_stream: bool,
    /// Whether playback is active (not paused).
    pub playing: bool,
    /// Current cursor in TIMELINE frames.
    pub frame: i64,
    /// Whether a real output device was found at construction.
    pub device_available: bool,
}

/// The shared playback cursor + flags the cpal callback reads/advances. Kept in an
/// `Arc` so the (real-time, `Send`) callback owns a clone while the control thread
/// keeps another.
struct Shared {
    /// The interleaved stereo 48 kHz bus to play (`[L, R, …]`). Behind a `Mutex` so a
    /// new `start()` can swap it; the callback locks briefly per buffer. (A lock in the
    /// audio callback is not ideal for ultra-low-latency pro audio, but for preview
    /// monitoring at a normal buffer size the contention is negligible and the bus is
    /// only swapped on play/seek, never per buffer.)
    bus: Mutex<Arc<Vec<f32>>>,
    /// Playback cursor in **output sample-frames** from the start of the bus.
    cursor_frames: AtomicU64,
    /// Whether playback is active. When false the callback writes silence.
    playing: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Shared {
            bus: Mutex::new(Arc::new(Vec::new())),
            cursor_frames: AtomicU64::new(0),
            playing: AtomicBool::new(false),
        }
    }
}

/// A controllable real-time audio output player. Construct once (at app boot), then
/// drive it from the transport commands. Cheap to construct; the actual stream is built
/// on the first [`start`](Self::start).
pub struct AudioPlayer {
    shared: Arc<Shared>,
    /// The live cpal stream. `Some` while playing/paused; `None` after `stop()` / before
    /// the first `start()`. Held so it is not dropped (dropping closes the device).
    stream: Mutex<Option<cpal::Stream>>,
    /// Frames-per-second of the timeline the cursor maps to (set on `start`/`seek`).
    fps: AtomicU64,
    /// Whether a default output device existed at construction.
    device_available: bool,
}

// cpal::Stream is not Send on some platforms; we keep it behind a Mutex and never move
// it across threads after creation (all control happens on the command thread). Tauri's
// managed state requires Send+Sync, and the stream is only touched under the Mutex on
// the same thread that built it in practice. SAFETY: the stream handle is only created,
// played, paused, and dropped under `stream`'s Mutex; we never share the inner stream.
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

impl AudioPlayer {
    /// Construct the player. Probes for a default output device but does NOT open a
    /// stream yet (the first [`start`](Self::start) does). Never fails — a box with no
    /// audio device gets a player whose transport calls are silent no-ops.
    #[must_use]
    pub fn new() -> Self {
        let device_available = cpal::default_host().default_output_device().is_some();
        if !device_available {
            tracing::warn!(
                target: "audio",
                "no default output device; audio playback will be a silent no-op"
            );
        }
        AudioPlayer {
            shared: Arc::new(Shared::new()),
            stream: Mutex::new(None),
            fps: AtomicU64::new(30),
            device_available,
        }
    }

    /// Whether a real output device was found at construction.
    #[must_use]
    pub fn device_available(&self) -> bool {
        self.device_available
    }

    /// Convert a timeline frame to an output sample-frame index given `fps`.
    fn frame_to_sample(frame: i64, fps: u64) -> u64 {
        if fps == 0 {
            return 0;
        }
        let f = frame.max(0) as f64;
        (f / fps as f64 * PROJECT_SAMPLE_RATE_HZ as f64).round() as u64
    }

    /// Convert an output sample-frame index back to a timeline frame.
    fn sample_to_frame(sample: u64, fps: u64) -> i64 {
        (sample as f64 / PROJECT_SAMPLE_RATE_HZ as f64 * fps as f64).round() as i64
    }

    /// Build (or rebuild) the bus from the timeline's audio tracks and begin playing
    /// from `from_frame`. `output_frames` is the total length of the bus in sample-
    /// frames (typically the timeline length); the mixer zero-pads beyond the clips.
    ///
    /// Replaces any existing stream. A no-op (logged) when there is no output device.
    pub fn start(
        &self,
        from_frame: i64,
        tracks: &[(AudioTrack, Vec<StereoClipAudio>)],
        fps: u32,
        output_frames: usize,
    ) {
        self.fps.store(fps.max(1) as u64, Ordering::SeqCst);
        let bus = mix_to_stereo_bus(tracks, fps, output_frames);
        self.load_bus_and_play(from_frame, bus);
    }

    /// Load a pre-mixed interleaved-stereo bus and begin playing from `from_frame`. The
    /// lower-level entry point `start` builds on; exposed for callers that mix elsewhere.
    pub fn load_bus_and_play(&self, from_frame: i64, bus: Vec<f32>) {
        if !self.device_available {
            tracing::debug!(target: "audio", "load_bus_and_play: no device, no-op");
            return;
        }
        let fps = self.fps.load(Ordering::SeqCst);
        let start_sample = Self::frame_to_sample(from_frame, fps);

        // Install the new bus + cursor BEFORE (re)building the stream so the first
        // callback already sees them.
        *self.shared.bus.lock().expect("bus mutex") = Arc::new(bus);
        self.shared.cursor_frames.store(start_sample, Ordering::SeqCst);
        self.shared.playing.store(true, Ordering::SeqCst);

        match self.build_stream() {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    tracing::warn!(target: "audio", error = %e, "failed to start audio stream");
                }
                *self.stream.lock().expect("stream mutex") = Some(stream);
                tracing::info!(target: "audio", from_frame, "audio playback started");
            }
            Err(e) => {
                tracing::warn!(target: "audio", error = %e, "failed to build audio output stream");
            }
        }
    }

    /// Pause playback (the cursor is kept; the callback emits silence). The stream stays
    /// open so [`resume`](Self::resume) is instant.
    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
        tracing::debug!(target: "audio", "audio paused");
    }

    /// Resume playback from the current cursor.
    pub fn resume(&self) {
        if self.device_available {
            self.shared.playing.store(true, Ordering::SeqCst);
        }
    }

    /// Reposition the cursor to `frame` (works while playing or paused). Does not change
    /// the play/pause state.
    pub fn seek(&self, frame: i64) {
        let fps = self.fps.load(Ordering::SeqCst);
        let sample = Self::frame_to_sample(frame, fps);
        self.shared.cursor_frames.store(sample, Ordering::SeqCst);
        tracing::debug!(target: "audio", frame, "audio seek");
    }

    /// Stop playback and tear the stream down (releases the device). The bus is cleared.
    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
        *self.stream.lock().expect("stream mutex") = None;
        *self.shared.bus.lock().expect("bus mutex") = Arc::new(Vec::new());
        self.shared.cursor_frames.store(0, Ordering::SeqCst);
        tracing::debug!(target: "audio", "audio stopped + stream torn down");
    }

    /// The current cursor as a TIMELINE frame (for A/V sync reads).
    #[must_use]
    pub fn current_frame(&self) -> i64 {
        let fps = self.fps.load(Ordering::SeqCst);
        let sample = self.shared.cursor_frames.load(Ordering::SeqCst);
        Self::sample_to_frame(sample, fps)
    }

    /// A snapshot of the transport state.
    #[must_use]
    pub fn status(&self) -> PlayerStatus {
        PlayerStatus {
            has_stream: self.stream.lock().expect("stream mutex").is_some(),
            playing: self.shared.playing.load(Ordering::SeqCst),
            frame: self.current_frame(),
            device_available: self.device_available,
        }
    }

    /// Build a cpal output stream over the shared bus/cursor. The callback pulls stereo
    /// samples from the bus at the cursor, advances the cursor, and writes them to the
    /// device (mapping to the device's channel count). When paused or past the end of
    /// the bus it writes silence.
    fn build_stream(&self) -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_string())?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("default_output_config: {e}"))?;
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let device_channels = stream_config.channels as usize;

        let shared = Arc::clone(&self.shared);
        let err_fn = |err| tracing::warn!(target: "audio", error = %err, "audio stream error");

        // Only f32 device streams are supported in this preview path (the common case on
        // WASAPI/CoreAudio). Other formats degrade to a silent no-op stream rather than
        // failing the whole transport.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    &stream_config,
                    move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        fill_output(out, device_channels, &shared);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build_output_stream(f32): {e}"))?,
            other => {
                return Err(format!(
                    "unsupported output sample format {other:?} (preview audio needs f32)"
                ));
            }
        };
        Ok(stream)
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        AudioPlayer::new()
    }
}

/// Fill one device output buffer from the shared bus at the cursor, advancing the
/// cursor by the number of sample-frames consumed. Maps the stereo bus to the device's
/// channel count. Writes silence when paused or past the end of the bus.
fn fill_output(out: &mut [f32], device_channels: usize, shared: &Shared) {
    // Always start from silence so an early-return leaves a clean buffer.
    for s in out.iter_mut() {
        *s = 0.0;
    }
    if device_channels == 0 {
        return;
    }
    if !shared.playing.load(Ordering::SeqCst) {
        return; // paused → silence, cursor frozen.
    }

    let bus = shared.bus.lock().expect("bus mutex").clone();
    let bus_frames = bus.len() / BUS_CHANNELS;
    if bus_frames == 0 {
        return;
    }

    let frames_wanted = out.len() / device_channels;
    let mut cursor = shared.cursor_frames.load(Ordering::SeqCst) as usize;

    for f in 0..frames_wanted {
        if cursor >= bus_frames {
            break; // past the end — remaining output stays silent (the bus ran out).
        }
        let l = bus[cursor * BUS_CHANNELS];
        let r = bus[cursor * BUS_CHANNELS + 1];
        let base = f * device_channels;
        match device_channels {
            1 => out[base] = 0.5 * (l + r), // mono device → downmix.
            _ => {
                out[base] = l;
                out[base + 1] = r;
                // Any extra channels stay silent (already zeroed).
            }
        }
        cursor += 1;
    }

    shared
        .cursor_frames
        .store(cursor as u64, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests exercise the player's STATE MACHINE + the pure fill_output mapping
    // WITHOUT a real device. `AudioPlayer::new()` may or may not find a device on the
    // test box; the device-gated control paths (start/resume/load_bus_and_play) no-op
    // cleanly when no device is present, so the state assertions below avoid asserting
    // `playing == true` after a device-gated call (it depends on hardware). The
    // device-INDEPENDENT transitions (pause, seek, stop, cursor math, fill_output) are
    // asserted unconditionally.

    #[test]
    fn frame_sample_round_trip() {
        // 30 fps, frame 30 → 1.0 s → 48000 samples → back to frame 30.
        let s = AudioPlayer::frame_to_sample(30, 30);
        assert_eq!(s, 48_000);
        assert_eq!(AudioPlayer::sample_to_frame(s, 30), 30);
        // fps 0 guards.
        assert_eq!(AudioPlayer::frame_to_sample(10, 0), 0);
    }

    #[test]
    fn fill_output_advances_cursor_when_playing() {
        let shared = Shared::new();
        // 4 stereo frames: L=1,2,3,4  R=-1,-2,-3,-4
        *shared.bus.lock().unwrap() =
            Arc::new(vec![1.0, -1.0, 2.0, -2.0, 3.0, -3.0, 4.0, -4.0]);
        shared.playing.store(true, Ordering::SeqCst);
        shared.cursor_frames.store(0, Ordering::SeqCst);

        // Stereo device, request 2 frames (4 samples).
        let mut out = [0.0f32; 4];
        fill_output(&mut out, 2, &shared);
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);
        assert_eq!(shared.cursor_frames.load(Ordering::SeqCst), 2);

        // Next 2 frames continue from the cursor.
        let mut out2 = [0.0f32; 4];
        fill_output(&mut out2, 2, &shared);
        assert_eq!(out2, [3.0, -3.0, 4.0, -4.0]);
        assert_eq!(shared.cursor_frames.load(Ordering::SeqCst), 4);

        // Past the end → silence, cursor does not run away.
        let mut out3 = [0.0f32; 4];
        fill_output(&mut out3, 2, &shared);
        assert_eq!(out3, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn fill_output_paused_is_silent_and_frozen() {
        let shared = Shared::new();
        *shared.bus.lock().unwrap() = Arc::new(vec![1.0, 1.0, 1.0, 1.0]);
        shared.playing.store(false, Ordering::SeqCst);
        shared.cursor_frames.store(0, Ordering::SeqCst);
        let mut out = [9.9f32; 4];
        fill_output(&mut out, 2, &shared);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0], "paused ⇒ silence");
        assert_eq!(shared.cursor_frames.load(Ordering::SeqCst), 0, "cursor frozen");
    }

    #[test]
    fn fill_output_mono_device_downmixes() {
        let shared = Shared::new();
        // 1 stereo frame L=1.0 R=0.0 → mono = 0.5.
        *shared.bus.lock().unwrap() = Arc::new(vec![1.0, 0.0]);
        shared.playing.store(true, Ordering::SeqCst);
        let mut out = [0.0f32; 1];
        fill_output(&mut out, 1, &shared);
        assert!((out[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pause_seek_stop_state_transitions_are_device_independent() {
        let player = AudioPlayer::new();
        player.fps.store(30, Ordering::SeqCst);

        // Seek moves the cursor regardless of device.
        player.seek(30);
        assert_eq!(player.current_frame(), 30);

        // Pause clears playing.
        player.shared.playing.store(true, Ordering::SeqCst);
        player.pause();
        assert!(!player.status().playing);

        // Stop tears down: no stream, cursor reset, bus cleared.
        player.stop();
        let st = player.status();
        assert!(!st.has_stream);
        assert_eq!(st.frame, 0);
        assert!(player.shared.bus.lock().unwrap().is_empty());
    }

    #[test]
    fn status_reports_device_availability() {
        let player = AudioPlayer::new();
        // Mirrors construction-time probe; we don't assert a specific value (CI may or
        // may not have a device) — just that it is consistent.
        assert_eq!(player.status().device_available, player.device_available());
    }
}
