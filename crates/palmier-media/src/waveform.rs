//! Audio waveform pipeline (story E4-S4) — symphonia decode → downsample to the
//! reference sample density, cached as a raw `Vec<f32>` blob.
//!
//! Port of `MediaVisualCache.loadOrGenerateWaveform` + `waveformSampleCount` +
//! `loadWaveform`/`saveWaveform` (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`).
//! The macOS reference drives DSWaveformImage's `WaveformAnalyzer`; we replace it
//! with a `symphonia` decode + RMS downsample (`docs/reference/media-panel.md`
//! §"Waveform" / §"macOS/Apple APIs to replace").
//!
//! ## Sample density (ruling #16)
//! [`waveform_sample_count`]: `duration >= 20000/150 (≈133.3 s) → 20000`; else
//! `max(4000, round(duration * 150))`; `duration <= 0 → 4000`. = **150
//! samples/s, capped 20000**. FOUNDATION's "~2000/min" is superseded.
//!
//! ## Normalization (open question, defaulted)
//! Output is normalized **0 = loud … 1 = silence** to match the reference draw
//! axis. The exact DSWaveformImage curve (linear amplitude vs perceptual) is an
//! open question carried in the reference doc; we default to **linear-amplitude
//! RMS** (`1 - rms`, rms in [0,1]) and note the assumption inline. A silent
//! buffer → ~1.0; full-scale → ~0.0.
//!
//! ## Cache integration (#16)
//! Persisted as a little-endian `f32` `<key>.waveform` blob under the E4-S2
//! [`crate::cache::cache_key`], behind the **2-wide**
//! [`crate::cache::CacheKind::Waveform`] gate.

use std::path::{Path, PathBuf};

use crate::cache::{cache_key, CacheGates, CacheKind};

/// The cap on total waveform samples (ruling #16). Reached at
/// `duration >= CAP / SAMPLES_PER_SECOND` seconds.
pub const WAVEFORM_SAMPLE_CAP: usize = 20_000;
/// Target sample density: 150 samples per second of audio (ruling #16).
pub const WAVEFORM_SAMPLES_PER_SECOND: f64 = 150.0;
/// Floor on the sample count (short clips still get a usable resolution).
pub const WAVEFORM_SAMPLE_FLOOR: usize = 4_000;

/// Number of waveform samples to produce for a clip of `duration` seconds.
///
/// ```text
/// duration <= 0 (or non-finite) → 4000
/// duration >= 20000/150 (≈133.33s) → 20000
/// else                            → max(4000, round(duration * 150))
/// ```
///
/// The `>=` threshold is written as `CAP / SAMPLES_PER_SECOND` (matching the
/// reference `Double(20_000) / 150`) so the cap kicks in at exactly the duration
/// where `duration * 150` would reach 20000.
pub fn waveform_sample_count(duration: f64) -> usize {
    if !duration.is_finite() || duration <= 0.0 {
        return WAVEFORM_SAMPLE_FLOOR;
    }
    if duration >= WAVEFORM_SAMPLE_CAP as f64 / WAVEFORM_SAMPLES_PER_SECOND {
        return WAVEFORM_SAMPLE_CAP;
    }
    // Swift `Int(duration * 150)` truncates; we match with `as usize` (truncation
    // toward zero) for parity rather than rounding.
    let scaled = (duration * WAVEFORM_SAMPLES_PER_SECOND) as usize;
    scaled.max(WAVEFORM_SAMPLE_FLOOR)
}

/// Downsample interleaved mono PCM `samples` (each in `[-1.0, 1.0]`) to exactly
/// `count` waveform values, normalized **0 = loud … 1 = silence** via per-bucket
/// RMS.
///
/// Pure + public so the downsample is unit-tested without decoding a file.
/// Each output bucket covers `samples.len() / count` input samples; its value is
/// `1 - rms(bucket)` clamped to `[0, 1]`. An empty input yields `count` silence
/// values (1.0). `count == 0` yields an empty vec.
pub fn downsample_rms(samples: &[f32], count: usize) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    if samples.is_empty() {
        return vec![1.0; count]; // silence
    }
    let n = samples.len();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // Bucket [start, end) over the input — distribute samples evenly so the
        // last bucket reaches the end exactly (no samples dropped/duplicated).
        let start = i * n / count;
        let end = ((i + 1) * n / count).max(start + 1).min(n);
        let bucket = &samples[start..end];
        let sum_sq: f64 = bucket.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (sum_sq / bucket.len() as f64).sqrt();
        // Linear-amplitude RMS (open question: DSWaveformImage curve). Invert so
        // 0 = loud, 1 = silence; clamp in case of slight over-unity samples.
        let value = (1.0 - rms).clamp(0.0, 1.0);
        out.push(value as f32);
    }
    out
}

/// Errors the waveform pipeline can surface.
#[derive(Debug)]
pub enum WaveformError {
    /// symphonia probe/decode failure.
    Decode(String),
    /// No audio track in the container.
    NoAudioTrack,
    /// I/O reading the source or the cache blob.
    Io(String),
}

impl std::fmt::Display for WaveformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaveformError::Decode(m) => write!(f, "decode: {m}"),
            WaveformError::NoAudioTrack => write!(f, "no audio track"),
            WaveformError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for WaveformError {}

/// Decode all audio in `path` to a mono `f32` PCM stream (channels averaged),
/// returning `(samples, sample_rate)`. Uses symphonia's default codec set
/// (mp3/wav/aac/pcm/adpcm + isomp4 container, matching the metadata loader).
fn decode_mono_pcm(path: &Path) -> Result<(Vec<f32>, u32), WaveformError> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::codecs::CodecParameters;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|e| WaveformError::Io(e.to_string()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| WaveformError::Decode(format!("probe: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.tracks().iter().find(|t| {
            matches!(t.codec_params, Some(CodecParameters::Audio(_)))
        }))
        .ok_or(WaveformError::NoAudioTrack)?;
    let track_id = track.id;
    let audio_params = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(WaveformError::NoAudioTrack),
    };
    let sample_rate = audio_params.sample_rate.unwrap_or(44_100);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| WaveformError::Decode(format!("make decoder: {e}")))?;

    let mut mono: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => return Err(WaveformError::Decode(format!("next packet: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // Skip a decode error on a single packet rather than aborting.
            Err(_) => continue,
        };
        append_mono(&decoded, &mut mono);
    }

    Ok((mono, sample_rate))
}

/// Average an audio buffer's channels into mono `f32` and append to `out`.
///
/// Uses the `Audio` trait's `iter_interleaved()` (which yields samples already
/// converted to `f32` in interleaved channel order) plus `num_planes()` for the
/// channel count, then averages each group of `channels` consecutive samples into
/// one mono frame.
fn append_mono(buf: &symphonia::core::audio::GenericAudioBufferRef<'_>, out: &mut Vec<f32>) {
    use symphonia::core::audio::{Audio, GenericAudioBufferRef};

    /// Generic over the concrete sample type: average channels per frame from the
    /// interleaved stream.
    fn mix<S>(b: &symphonia::core::audio::AudioBuffer<S>, out: &mut Vec<f32>)
    where
        S: symphonia::core::audio::sample::Sample
            + symphonia::core::audio::conv::IntoSample<f32>,
    {
        let channels = b.num_planes().max(1);
        out.reserve(b.frames());
        let mut acc = 0.0_f32;
        let mut in_frame = 0usize;
        // `iter_interleaved` yields one `S` per (frame, channel) in canonical
        // channel order; fold each `channels`-run into a mono average.
        for s in b.iter_interleaved() {
            let v: f32 = s.into_sample();
            acc += v;
            in_frame += 1;
            if in_frame == channels {
                out.push(acc / channels as f32);
                acc = 0.0;
                in_frame = 0;
            }
        }
        // Tail guard: a partial frame (shouldn't happen for well-formed audio)
        // still flushes so no samples are silently dropped.
        if in_frame > 0 {
            out.push(acc / in_frame as f32);
        }
    }

    // `buf` is `&GenericAudioBufferRef`, whose variants already hold a
    // `&AudioBuffer<S>`; the `&b` pattern peels the outer ref so `b` is the inner
    // `&AudioBuffer<S>` `mix` wants.
    match buf {
        GenericAudioBufferRef::U8(b) => mix(b, out),
        GenericAudioBufferRef::U16(b) => mix(b, out),
        GenericAudioBufferRef::U24(b) => mix(b, out),
        GenericAudioBufferRef::U32(b) => mix(b, out),
        GenericAudioBufferRef::S8(b) => mix(b, out),
        GenericAudioBufferRef::S16(b) => mix(b, out),
        GenericAudioBufferRef::S24(b) => mix(b, out),
        GenericAudioBufferRef::S32(b) => mix(b, out),
        GenericAudioBufferRef::F32(b) => mix(b, out),
        GenericAudioBufferRef::F64(b) => mix(b, out),
    }
}

/// Cache path for a waveform `key` (`<key>.waveform`).
fn waveform_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.waveform"))
}

/// Serialize a waveform to a little-endian `f32` blob (the on-disk format,
/// matching the reference's raw `[Float]` write).
pub fn encode_waveform(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for &s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}

/// Parse a little-endian `f32` waveform blob back into samples. Returns `None`
/// if the byte count isn't a multiple of 4 or the blob is empty (parity with the
/// Swift `data.count % 4 == 0` guard).
pub fn decode_waveform(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Generate the waveform for `path` of the given `duration`: decode → downsample
/// to [`waveform_sample_count`] → normalize. Pure-CPU; the cached entry point
/// runs it on a blocking task under the 2-wide gate.
pub fn generate_waveform(path: &Path, duration: f64) -> Result<Vec<f32>, WaveformError> {
    let count = waveform_sample_count(duration);
    let (pcm, _sample_rate) = decode_mono_pcm(path)?;
    Ok(downsample_rms(&pcm, count))
}

/// Cache-integrated waveform generator. Wires [`generate_waveform`] through the
/// E4-S2 cache key (#16) + the **2-wide** [`CacheKind::Waveform`] gate,
/// persisting each waveform as a `.waveform` blob.
#[derive(Clone)]
pub struct WaveformCache {
    dir: PathBuf,
    gates: CacheGates<Option<Vec<f32>>>,
}

impl WaveformCache {
    /// Build a cache rooted at `dir` (typically [`crate::cache::media_visual_cache_dir`]).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        WaveformCache {
            dir: dir.into(),
            gates: CacheGates::new(),
        }
    }

    /// Cache directory this instance writes under.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Load the cached waveform for `path` if present + well-formed, else `None`.
    pub fn load(&self, path: &Path) -> Option<Vec<f32>> {
        let key = cache_key(path)?;
        let bytes = std::fs::read(waveform_path(&self.dir, &key)).ok()?;
        decode_waveform(&bytes)
    }

    /// Generate (or reuse) the waveform for `path`. Gated at 2 concurrent;
    /// concurrent same-key requests share one decode. A cache hit skips decode.
    pub async fn generate(&self, path: &Path, duration: f64) -> Result<Vec<f32>, WaveformError> {
        // Fast path: cached blob on disk.
        if let Some(cached) = self.load(path) {
            return Ok(cached);
        }
        let Some(key) = cache_key(path) else {
            // Keyless → decode uncached.
            let path = path.to_path_buf();
            return tokio::task::spawn_blocking(move || generate_waveform(&path, duration))
                .await
                .map_err(|e| WaveformError::Io(e.to_string()))?;
        };

        let dir = self.dir.clone();
        let path_buf = path.to_path_buf();
        let produced = self
            .gates
            .run(CacheKind::Waveform, &key, || {
                let dir = dir.clone();
                let key = key.clone();
                let path = path_buf.clone();
                async move {
                    tokio::task::spawn_blocking(move || {
                        let samples = generate_waveform(&path, duration).ok()?;
                        // Best-effort persist; failure to write still returns the
                        // samples in-memory.
                        if std::fs::create_dir_all(&dir).is_ok() {
                            let _ = std::fs::write(
                                waveform_path(&dir, &key),
                                encode_waveform(&samples),
                            );
                        }
                        Some(samples)
                    })
                    .await
                    .ok()
                    .flatten()
                }
            })
            .await;

        produced.ok_or_else(|| WaveformError::Decode("waveform generation failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_count_formula_at_boundaries() {
        // duration <= 0 / non-finite → floor 4000.
        assert_eq!(waveform_sample_count(0.0), 4000);
        assert_eq!(waveform_sample_count(-5.0), 4000);
        assert_eq!(waveform_sample_count(f64::NAN), 4000);
        // duration 5 → max(4000, 750) = 4000 (floor wins for short clips).
        assert_eq!(waveform_sample_count(5.0), 4000);
        // duration where 150*d just exceeds the floor: 4000/150 ≈ 26.67s →
        // 27s → 4050.
        assert_eq!(waveform_sample_count(27.0), (27.0 * 150.0) as usize);
        assert_eq!(waveform_sample_count(27.0), 4050);
        // duration 130 → 130*150 = 19500 (under cap).
        assert_eq!(waveform_sample_count(130.0), 19_500);
        // cap threshold = 20000/150 ≈ 133.333s. Just under → not capped.
        assert!(waveform_sample_count(133.0) < 20_000);
        assert_eq!(waveform_sample_count(133.0), (133.0 * 150.0) as usize); // 19950
        // At/over threshold → capped 20000.
        assert_eq!(waveform_sample_count(133.4), 20_000);
        assert_eq!(waveform_sample_count(200.0), 20_000);
        // Exactly the threshold.
        assert_eq!(waveform_sample_count(20_000.0 / 150.0), 20_000);
    }

    #[test]
    fn downsample_produces_exact_count() {
        let samples = vec![0.5_f32; 10_000];
        for &count in &[1usize, 4000, 4096, 20000] {
            assert_eq!(downsample_rms(&samples, count).len(), count);
        }
        assert!(downsample_rms(&samples, 0).is_empty());
    }

    #[test]
    fn normalization_direction_silence_is_one_fullscale_is_zero() {
        // Silent buffer → all ~1.0.
        let silence = vec![0.0_f32; 5000];
        let w = downsample_rms(&silence, 1000);
        assert!(w.iter().all(|&v| (v - 1.0).abs() < 1e-6), "silence ⇒ ~1.0");

        // Full-scale (|s| = 1.0) → RMS 1.0 → value ~0.0.
        let full: Vec<f32> = (0..5000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let w = downsample_rms(&full, 1000);
        assert!(w.iter().all(|&v| v.abs() < 1e-6), "full-scale ⇒ ~0.0");

        // Mid-level (constant 0.5) → RMS 0.5 → value ~0.5.
        let mid = vec![0.5_f32; 5000];
        let w = downsample_rms(&mid, 1000);
        assert!(
            w.iter().all(|&v| (v - 0.5).abs() < 1e-3),
            "rms 0.5 ⇒ value ~0.5"
        );
    }

    #[test]
    fn empty_input_is_treated_as_silence() {
        let w = downsample_rms(&[], 100);
        assert_eq!(w.len(), 100);
        assert!(w.iter().all(|&v| v == 1.0));
    }

    #[tokio::test]
    #[ignore = "needs a real audio fixture via PALMIER_TEST_AUDIO"]
    async fn generate_waveform_from_real_audio() {
        // Exercises the symphonia decode → downsample path end to end. Run with:
        //   PALMIER_TEST_AUDIO=C:\clip.wav cargo test -p palmier-media -- --ignored
        let Ok(path) = std::env::var("PALMIER_TEST_AUDIO") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let cache = WaveformCache::new(dir.path());
        let w = cache.generate(std::path::Path::new(&path), 30.0).await.unwrap();
        assert_eq!(w.len(), waveform_sample_count(30.0));
        assert!(w.iter().all(|&v| (0.0..=1.0).contains(&v)), "normalized 0..1");
        // Second call hits the cached blob.
        let w2 = cache.generate(std::path::Path::new(&path), 30.0).await.unwrap();
        assert_eq!(w, w2);
    }

    #[test]
    fn waveform_blob_round_trips_little_endian() {
        let samples = vec![0.0_f32, 0.25, 0.5, 1.0];
        let bytes = encode_waveform(&samples);
        assert_eq!(bytes.len(), 16);
        let back = decode_waveform(&bytes).unwrap();
        assert_eq!(back, samples);
        // Guards.
        assert!(decode_waveform(&[]).is_none(), "empty ⇒ None");
        assert!(decode_waveform(&[1, 2, 3]).is_none(), "non-multiple-of-4 ⇒ None");
    }
}
