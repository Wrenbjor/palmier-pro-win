//! Audio PCM decode for real-time preview playback (audio-playback story).
//!
//! The mixer (`palmier-engine::audio::mixer`) and the cpal output player consume
//! **already-decoded, already-resampled** sample buffers — the
//! `ClipAudio.samples` seam the E5-S6 mixer documented as "a thin adapter over real
//! files". This module IS that adapter: it decodes a media asset's audio stream to
//! interleaved **f32 PCM at the project rate (48 kHz) and a fixed channel count
//! (stereo)**, so a clip's samples drop straight onto the mix bus.
//!
//! ## Pipeline (per asset, cached)
//! ```text
//! symphonia decode (native rate/channels) → channel map to stereo → linear resample
//!   to 48 kHz → interleaved f32 [L,R,L,R,…]
//! ```
//! We reuse the same `symphonia` decode the E4-S4 waveform pipeline proved in this
//! crate, but keep **channels** (waveform collapses to mono) and resample to the
//! 48 kHz project rate so the buffer is mix-ready. The resample is a simple
//! linear interpolation — adequate for preview monitoring; the offline export path
//! (E6) can swap in a higher-quality `rubato` pass without touching this seam.
//!
//! ## Why decode here (not in palmier-engine)
//! `palmier-media` already owns every FFmpeg/symphonia dependency and the decode
//! conventions (the waveform + frame pipelines). `palmier-engine`'s audio module is
//! deliberately presentation-/decode-agnostic — it takes sample buffers. So the
//! decode lives here and the engine's player consumes [`DecodedAudio`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The fixed project output sample rate (Hz). Mirrors
/// `palmier_engine::audio::PROJECT_SAMPLE_RATE_HZ` (FOUNDATION §6.5). Kept as a local
/// const so `palmier-media` does not depend on `palmier-engine` (layering: engine
/// depends on media, not the reverse).
pub const TARGET_SAMPLE_RATE_HZ: u32 = 48_000;

/// The fixed project output channel count (stereo). The mixer + cpal player work in
/// stereo; mono sources are duplicated to both channels, >2-channel sources are
/// downmixed to the first two channels.
pub const TARGET_CHANNELS: usize = 2;

/// Decoded, resampled audio for one asset: interleaved f32 at [`TARGET_SAMPLE_RATE_HZ`]
/// / [`TARGET_CHANNELS`] (`[L, R, L, R, …]`). The "frame" count (sample-frames, i.e.
/// per-channel samples) is `interleaved.len() / channels`.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Interleaved f32 samples, `channels`-wide, at [`TARGET_SAMPLE_RATE_HZ`].
    pub interleaved: Vec<f32>,
    /// Channel count (always [`TARGET_CHANNELS`] for now).
    pub channels: usize,
    /// Sample rate in Hz (always [`TARGET_SAMPLE_RATE_HZ`]).
    pub sample_rate: u32,
}

impl DecodedAudio {
    /// Number of sample-frames (per-channel samples).
    #[must_use]
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.interleaved.len() / self.channels
        }
    }

    /// Extract one channel as a flat `Vec<f32>` (channel `0` == left). An out-of-range
    /// channel yields the last available channel (so a mono buffer asked for R returns
    /// the mono data). Used to feed the per-channel mono mixer.
    #[must_use]
    pub fn channel(&self, ch: usize) -> Vec<f32> {
        if self.channels == 0 {
            return Vec::new();
        }
        let ch = ch.min(self.channels - 1);
        self.interleaved
            .iter()
            .skip(ch)
            .step_by(self.channels)
            .copied()
            .collect()
    }
}

/// Errors the audio decode can surface.
#[derive(Debug)]
pub enum AudioDecodeError {
    /// symphonia probe/decode failure.
    Decode(String),
    /// No audio track in the container (a silent video / image — caller skips it).
    NoAudioTrack,
    /// I/O reading the source.
    Io(String),
}

impl std::fmt::Display for AudioDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioDecodeError::Decode(m) => write!(f, "audio decode: {m}"),
            AudioDecodeError::NoAudioTrack => write!(f, "no audio track"),
            AudioDecodeError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for AudioDecodeError {}

/// Decode `path`'s audio to interleaved f32 PCM at the native sample rate + channel
/// count. Returns `(interleaved, sample_rate, channels)`. Mirrors the E4-S4 waveform
/// decode loop but keeps the interleaved multi-channel stream instead of collapsing
/// to mono.
fn decode_interleaved_native(path: &Path) -> Result<(Vec<f32>, u32, usize), AudioDecodeError> {
    use symphonia::core::audio::GenericAudioBufferRef;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::codecs::CodecParameters;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|e| AudioDecodeError::Io(e.to_string()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| AudioDecodeError::Decode(format!("probe: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| {
            format
                .tracks()
                .iter()
                .find(|t| matches!(t.codec_params, Some(CodecParameters::Audio(_))))
        })
        .ok_or(AudioDecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let audio_params = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(AudioDecodeError::NoAudioTrack),
    };
    let sample_rate = audio_params.sample_rate.unwrap_or(44_100);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| AudioDecodeError::Decode(format!("make decoder: {e}")))?;

    // Append every decoded buffer's interleaved samples; `channels` is learned from the
    // first decoded buffer (codec params may not carry it pre-decode).
    let mut interleaved: Vec<f32> = Vec::new();
    let mut channels: usize = 0;

    /// Append one buffer's samples in interleaved channel order, learning the channel
    /// count from the buffer's plane count.
    fn append<S>(
        b: &symphonia::core::audio::AudioBuffer<S>,
        out: &mut Vec<f32>,
        channels: &mut usize,
    ) where
        S: symphonia::core::audio::sample::Sample + symphonia::core::audio::conv::IntoSample<f32>,
    {
        use symphonia::core::audio::Audio;
        let ch = b.num_planes().max(1);
        *channels = ch;
        out.reserve(b.frames() * ch);
        for s in b.iter_interleaved() {
            out.push(s.into_sample());
        }
    }

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(AudioDecodeError::Decode(format!("next packet: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A single bad packet is skipped (matches the waveform decode tolerance).
            Err(_) => continue,
        };
        match decoded {
            GenericAudioBufferRef::U8(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::U16(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::U24(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::U32(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::S8(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::S16(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::S24(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::S32(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::F32(b) => append(b, &mut interleaved, &mut channels),
            GenericAudioBufferRef::F64(b) => append(b, &mut interleaved, &mut channels),
        }
    }

    if channels == 0 {
        // Decoded nothing usable.
        return Err(AudioDecodeError::Decode("no decodable audio frames".into()));
    }
    Ok((interleaved, sample_rate, channels))
}

/// Map an interleaved `src_channels`-wide native buffer to exactly
/// [`TARGET_CHANNELS`] (stereo): mono → duplicate to L+R; ≥2 → take the first two
/// channels. Returns a stereo-interleaved buffer at the SAME (native) sample rate.
fn map_to_stereo(interleaved: &[f32], src_channels: usize) -> Vec<f32> {
    if src_channels == 0 {
        return Vec::new();
    }
    let frames = interleaved.len() / src_channels;
    let mut out = Vec::with_capacity(frames * TARGET_CHANNELS);
    for f in 0..frames {
        let base = f * src_channels;
        if src_channels == 1 {
            let s = interleaved[base];
            out.push(s);
            out.push(s);
        } else {
            out.push(interleaved[base]);
            out.push(interleaved[base + 1]);
        }
    }
    out
}

/// Linear-resample a stereo-interleaved buffer from `src_rate` to
/// [`TARGET_SAMPLE_RATE_HZ`]. A no-op when the rates already match. Linear
/// interpolation is adequate for preview monitoring (export uses higher-quality
/// resampling).
fn resample_stereo_linear(stereo: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == TARGET_SAMPLE_RATE_HZ || src_rate == 0 {
        return stereo.to_vec();
    }
    let src_frames = stereo.len() / TARGET_CHANNELS;
    if src_frames == 0 {
        return Vec::new();
    }
    let ratio = TARGET_SAMPLE_RATE_HZ as f64 / src_rate as f64;
    let out_frames = ((src_frames as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_frames * TARGET_CHANNELS);
    for of in 0..out_frames {
        // Position in source-frame space.
        let src_pos = of as f64 / ratio;
        let i0 = src_pos.floor() as usize;
        let frac = (src_pos - i0 as f64) as f32;
        let i1 = (i0 + 1).min(src_frames - 1);
        for ch in 0..TARGET_CHANNELS {
            let a = stereo[i0 * TARGET_CHANNELS + ch];
            let b = stereo[i1 * TARGET_CHANNELS + ch];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// Decode `path`'s audio to interleaved stereo f32 PCM at [`TARGET_SAMPLE_RATE_HZ`].
/// Pure-CPU; the cached entry point ([`AudioPcmCache::get`]) runs it on a blocking
/// thread. Returns [`AudioDecodeError::NoAudioTrack`] for a source with no audio
/// stream (the caller skips that clip).
pub fn decode_audio_pcm(path: &Path) -> Result<DecodedAudio, AudioDecodeError> {
    let (interleaved, sample_rate, channels) = decode_interleaved_native(path)?;
    let stereo = map_to_stereo(&interleaved, channels);
    let resampled = resample_stereo_linear(&stereo, sample_rate);
    Ok(DecodedAudio {
        interleaved: resampled,
        channels: TARGET_CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE_HZ,
    })
}

/// A process-lifetime cache of decoded asset audio, keyed by absolute source path, so
/// pressing Play does not re-decode every clip's audio on each play/seek. Cheap to
/// clone (`Arc` inside); a decode result is shared as `Arc<DecodedAudio>`.
///
/// A failed decode (no audio track / error) is cached as `None` so a silent/offline
/// asset is not retried on every transport action.
#[derive(Clone, Default)]
pub struct AudioPcmCache {
    inner: Arc<Mutex<HashMap<PathBuf, Option<Arc<DecodedAudio>>>>>,
}

impl AudioPcmCache {
    /// New empty cache.
    #[must_use]
    pub fn new() -> Self {
        AudioPcmCache::default()
    }

    /// Decoded audio for `path`, decoding (and caching) on first request. Returns
    /// `None` when the asset has no usable audio (cached so it is decoded at most once).
    pub fn get(&self, path: &Path) -> Option<Arc<DecodedAudio>> {
        // Fast path: already decoded (hit or known-empty).
        if let Some(entry) = self.inner.lock().expect("audio cache mutex").get(path) {
            return entry.clone();
        }
        // Decode outside the lock so concurrent decodes of DIFFERENT assets don't
        // serialize (a duplicate decode of the same asset is acceptable + rare).
        let decoded = decode_audio_pcm(path).ok().map(Arc::new);
        self.inner
            .lock()
            .expect("audio cache mutex")
            .insert(path.to_path_buf(), decoded.clone());
        decoded
    }

    /// Drop all cached audio (e.g. on project switch).
    pub fn clear(&self) {
        self.inner.lock().expect("audio cache mutex").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_to_stereo_duplicates_mono() {
        let mono = vec![0.1, 0.2, 0.3];
        let st = map_to_stereo(&mono, 1);
        assert_eq!(st, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn map_to_stereo_takes_first_two_of_multichannel() {
        // 3-channel (L, R, C) interleaved → keep L+R.
        let multi = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let st = map_to_stereo(&multi, 3);
        assert_eq!(st, vec![1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn resample_is_identity_at_target_rate() {
        let stereo = vec![0.0, 0.0, 1.0, -1.0];
        assert_eq!(
            resample_stereo_linear(&stereo, TARGET_SAMPLE_RATE_HZ),
            stereo
        );
    }

    #[test]
    fn resample_upsamples_frame_count_by_ratio() {
        // 100 stereo frames at 24 kHz → ~200 frames at 48 kHz.
        let frames = 100usize;
        let stereo: Vec<f32> = (0..frames * TARGET_CHANNELS).map(|i| i as f32).collect();
        let out = resample_stereo_linear(&stereo, 24_000);
        let out_frames = out.len() / TARGET_CHANNELS;
        assert_eq!(out_frames, 200, "2x upsample doubles the frame count");
        // First output frame equals the first source frame (frac 0).
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 1.0);
    }

    #[test]
    fn decoded_audio_channel_extract_and_frame_count() {
        let da = DecodedAudio {
            interleaved: vec![1.0, 2.0, 3.0, 4.0], // L=1,3  R=2,4
            channels: 2,
            sample_rate: TARGET_SAMPLE_RATE_HZ,
        };
        assert_eq!(da.frame_count(), 2);
        assert_eq!(da.channel(0), vec![1.0, 3.0]);
        assert_eq!(da.channel(1), vec![2.0, 4.0]);
        // Out-of-range channel clamps to the last.
        assert_eq!(da.channel(5), vec![2.0, 4.0]);
    }

    #[test]
    fn cache_caches_empty_for_missing_file() {
        let cache = AudioPcmCache::new();
        // A path that doesn't exist → decode error → cached None, no panic.
        let p = Path::new("Z:/does/not/exist/clip.wav");
        assert!(cache.get(p).is_none());
        // Second call returns the cached None too (no re-decode panic).
        assert!(cache.get(p).is_none());
    }
}
