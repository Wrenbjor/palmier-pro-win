//! FFmpeg audio extraction + whisper.cpp engine run (E10-S2).
//!
//! Clean-room parity port of the macOS reference
//! `Sources/PalmierPro/Transcription/Transcription.swift` —
//! `extractAudioTrack`, `transcribe(fileURL:…)`, `transcribeVideoAudio`, and
//! `decodeResults`. The reference drives Apple `SpeechAnalyzer`/`SpeechTranscriber`
//! over an `AVAssetReader`-extracted PCM file; the port drives **whisper.cpp via
//! `whisper-rs` 0.16** over an **FFmpeg**-extracted PCM file
//! (`docs/reference/transcription.md` §A/§B, §"macOS APIs to replace").
//!
//! ## Audio extraction ([`extract_audio_track`])
//! FFmpeg-decode the asset's first audio track and **resample** it to the exact
//! format Whisper expects — **16 000 Hz, 1 channel (mono), 16-bit signed,
//! little-endian, non-float, interleaved** (FOUNDATION §6.9) — written to a temp
//! `.wav` file at `<tempdir>/palmier-stt-<UUID>.wav`. The downmix/resample is
//! **forced** on the output via swresample, not merely requested: the reference
//! gotcha is that `AVAssetReader` yields the source channel layout *before* output
//! settings apply, so the port must force, not request, mono/16 kHz
//! (`docs/reference/transcription.md` §"Port risks" — AVAssetReader channel
//! layout). The returned [`TempAudioFile`] RAII guard deletes the temp file on
//! drop, mirroring the reference `defer { removeItem }`.
//!
//! ## Engine run ([`transcribe`] / [`transcribe_video_audio`])
//! Extract (whole file or a `range`), load the bundled Whisper model, run
//! `WhisperState::full(...)` with word-level token timestamps, then
//! [`decode_results`] walks the segments/words into a [`TranscriptionResult`] in
//! **source seconds**. When a `range` is given, the result is
//! [`TranscriptionResult::offsetting`]-shifted by `range.start()` back into source
//! time (parity with the reference `offsetting(by: range.lowerBound)`).
//!
//! ## Backend
//! CPU is the parity-safe baseline (whisper-rs default features = none; this box is
//! AMD). Vulkan/CUDA are opt-in cargo features wired at the `whisper-rs` dep and are
//! off in the default build (`spikes/whisper-setup/FINDINGS.md` §"Backend decision").
//!
//! ## E10-S3 seam (locale + profanity)
//! Locale **resolution** and profanity **censoring** are E10-S3's job (it owns
//! `locale.rs` / `profanity.rs`). This module accepts the `locale` /
//! `censor_profanity` params and threads them, but resolves the locale with a
//! minimal stub ([`resolve_locale_stub`]) that returns BCP-47 `"en"` for the bundled
//! `.en` model. When E10-S3 lands its real resolver, replace the
//! [`resolve_locale_stub`] call site in [`transcribe`] with the S3 resolver and pass
//! `censor_profanity` into the suppression path — the signatures here already carry
//! both params. See the inline `// E10-S3 SEAM` markers.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use ffmpeg_next as ff;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::error::TranscriptionError;
use crate::model::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};

/// The PCM format Whisper consumes — must match exactly (FOUNDATION §6.9).
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Environment variable naming the directory that holds the bundled GGML model
/// files. The spike placed `ggml-small.en.bin` under
/// `%LOCALAPPDATA%\palmier-pro\models\`; the shipped app resolves the bundled
/// Tauri resource path. Default model file name below.
const MODEL_DIR_ENV: &str = "PALMIER_MODEL_DIR";
/// The bundled default model (FOUNDATION §6.9: bundle `small.en`).
const DEFAULT_MODEL_FILE: &str = "ggml-small.en.bin";

/// A temp PCM file that deletes itself on drop — RAII parity with the reference
/// `defer { try? FileManager.default.removeItem(at: tempAudioURL) }`.
///
/// The caller never has to remember to clean up: the extracted WAV is removed when
/// this guard goes out of scope (success *or* error path), exactly like the Swift
/// `defer`.
#[derive(Debug)]
pub struct TempAudioFile {
    path: PathBuf,
}

impl TempAudioFile {
    /// The on-disk path of the extracted PCM file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempAudioFile {
    fn drop(&mut self) {
        // Best-effort delete (reference uses `try?` — failure is swallowed).
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Ensure ffmpeg is initialized exactly once for this process.
///
/// `ffmpeg_next::init()` is itself idempotent, but we guard it behind a `Once` so a
/// transient init error surfaces consistently and we don't repeatedly retry. This
/// mirrors `palmier-media`'s private `ensure_ffmpeg_init` (we share the same
/// `ffmpeg-next` 7 dependency through that crate; we do not open a *second video*
/// decode path — audio-to-16k-mono extraction has no helper in `palmier-media`).
fn ensure_ffmpeg_init() -> Result<(), TranscriptionError> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    let mut err: Option<String> = None;
    INIT.call_once(|| {
        if let Err(e) = ff::init() {
            err = Some(e.to_string());
        }
    });
    match err {
        Some(e) => Err(TranscriptionError::AudioExtractionFailed(format!(
            "ffmpeg init failed: {e}"
        ))),
        None => Ok(()),
    }
}

/// Decode the asset's first audio track to a temp PCM `.wav` file in
/// **16 000 Hz / mono / 16-bit signed LE / interleaved**, the format Whisper
/// expects. Optional `range` (source seconds, inclusive) limits extraction to
/// `[lower, upper]`.
///
/// Returns a [`TempAudioFile`] RAII guard whose `Drop` deletes the file (reference
/// `defer`). Any failure → [`TranscriptionError::AudioExtractionFailed`] with a
/// descriptive reason (parity with the reference's `audioExtractionFailed(reason)`).
///
/// The mono/16 kHz conversion is **forced** via swresample on the output — the
/// reference gotcha is that the reader yields the source layout before output
/// settings apply, so the resample must force, not merely request, the downmix.
pub fn extract_audio_track(
    file_url: &Path,
    range: Option<&RangeInclusive<f64>>,
) -> Result<TempAudioFile, TranscriptionError> {
    ensure_ffmpeg_init()?;

    let mut ictx = ff::format::input(&file_url).map_err(|e| {
        TranscriptionError::AudioExtractionFailed(format!(
            "Cannot open {}: {e}",
            file_url.display()
        ))
    })?;

    // Find the first (best) audio stream — parity with `loadTracks(.audio).first`.
    // Scope the immutable stream borrow so the later `ictx.seek`/`ictx.packets()`
    // mutable borrows are valid (we extract index/time_base + build the decoder from
    // cloned codec parameters here, then drop the borrow).
    let (stream_index, time_base, mut decoder) = {
        let audio_stream = ictx
            .streams()
            .best(ff::media::Type::Audio)
            .ok_or_else(|| {
                TranscriptionError::AudioExtractionFailed(format!(
                    "No audio track in {}",
                    file_name(file_url)
                ))
            })?;
        let stream_index = audio_stream.index();
        let time_base = f64::from(audio_stream.time_base());

        let decoder = ff::codec::context::Context::from_parameters(audio_stream.parameters())
            .and_then(|c| c.decoder().audio())
            .map_err(|e| {
                TranscriptionError::AudioExtractionFailed(format!("audio decoder init: {e}"))
            })?;
        (stream_index, time_base, decoder)
    };

    // Range → seek + bound. Seek to `lower` (in stream micro-seconds via AV_TIME_BASE)
    // and stop emitting once a frame's start passes `upper`.
    let (lo, hi) = match range {
        Some(r) => (Some(*r.start()), Some(*r.end())),
        None => (None, None),
    };
    if let Some(lo) = lo {
        // ffmpeg seek timestamp is in AV_TIME_BASE units (microseconds). Seek to a
        // keyframe at/under `lo`; we then drop pre-`lo` samples by timestamp.
        let ts = (lo * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        // Best-effort seek; a failure just means we decode from the start and filter.
        let _ = ictx.seek(ts, ..ts);
    }

    let mut pcm: Vec<i16> = Vec::new();
    let mut got_any = false;
    // The resampler is built **lazily from the first decoded frame's actual**
    // format/layout/rate. swresample's `run` rejects an input whose params differ
    // from the context's (the "Input changed" error); building from the decoder's
    // *advertised* params can mismatch the per-frame descriptor, so we configure off
    // the real frame instead. Output is FORCED to 16 kHz / mono / s16 packed.
    let mut resampler: Option<ff::software::resampling::Context> = None;

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet).map_err(|e| {
            TranscriptionError::AudioExtractionFailed(format!("decode: {e}"))
        })?;
        drain_decoder(
            &mut decoder,
            &mut resampler,
            time_base,
            lo,
            hi,
            &mut pcm,
            &mut got_any,
        )?;
    }
    // Flush the decoder (EOF) then the resampler.
    decoder.send_eof().map_err(|e| {
        TranscriptionError::AudioExtractionFailed(format!("decode eof: {e}"))
    })?;
    drain_decoder(
        &mut decoder,
        &mut resampler,
        time_base,
        lo,
        hi,
        &mut pcm,
        &mut got_any,
    )?;
    // Flush any buffered resampler output.
    if let Some(resampler) = resampler.as_mut() {
        loop {
            let mut out = ff::frame::Audio::empty();
            match resampler.flush(&mut out) {
                Ok(delay) => {
                    append_i16(&out, &mut pcm);
                    if delay.is_none() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    if !got_any || pcm.is_empty() {
        return Err(TranscriptionError::AudioExtractionFailed(format!(
            "No audio samples in {}",
            file_name(file_url)
        )));
    }

    // Write a canonical 16-bit PCM mono 16 kHz WAV to a UUID temp path.
    let out_path = std::env::temp_dir().join(format!(
        "palmier-stt-{}.wav",
        uuid::Uuid::new_v4()
    ));
    write_wav_s16_mono(&out_path, &pcm, WHISPER_SAMPLE_RATE).map_err(|e| {
        TranscriptionError::AudioExtractionFailed(format!("write wav: {e}"))
    })?;

    Ok(TempAudioFile { path: out_path })
}

/// Pull all decoded frames currently buffered in `decoder`, resample each to the
/// forced 16 kHz/mono/s16 format, and append the PCM to `pcm`. Frames outside the
/// optional `[lo, hi]` source-seconds window are dropped; `got_any` records whether
/// any output samples were produced. Returns early (stops draining) once a frame's
/// start passes `hi`.
#[allow(clippy::too_many_arguments)]
fn drain_decoder(
    decoder: &mut ff::decoder::Audio,
    resampler: &mut Option<ff::software::resampling::Context>,
    time_base: f64,
    lo: Option<f64>,
    hi: Option<f64>,
    pcm: &mut Vec<i16>,
    got_any: &mut bool,
) -> Result<(), TranscriptionError> {
    let mut decoded = ff::frame::Audio::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        // Frame source-time in seconds (for range filtering).
        let frame_secs = decoded
            .timestamp()
            .map(|t| t as f64 * time_base)
            .unwrap_or(f64::NAN);

        // Drop frames entirely before the requested lower bound.
        if let Some(lo) = lo {
            if frame_secs.is_finite() && frame_secs + frame_duration(&decoded) < lo {
                continue;
            }
        }
        // Stop once we've passed the upper bound.
        if let Some(hi) = hi {
            if frame_secs.is_finite() && frame_secs > hi {
                return Ok(());
            }
        }

        // Normalize the frame's channel layout to a concrete `default(channels)`
        // mask. WAV/PCM decoders often leave `channel_layout` UNSPECIFIED on the
        // frame; `swr_convert_frame` then raises AVERROR_INPUT_CHANGED because the
        // frame descriptor doesn't match the context's configured layout. Stamping
        // the frame (and building the context from the SAME default below) makes
        // them agree.
        let canon_layout = ff::ChannelLayout::default(decoded.channels() as i32);
        decoded.set_channel_layout(canon_layout);

        // Lazily build the resampler from this (normalized) frame's params. Output
        // is FORCED to 16 kHz / mono / s16 packed.
        if resampler.is_none() {
            *resampler = Some(build_resampler(&decoded)?);
        }

        let mut out = ff::frame::Audio::empty();
        let mut run_res = resampler.as_mut().unwrap().run(&decoded, &mut out);
        // If a later frame's params still differ (e.g. a genuine rate/format
        // change), reconfigure from the new frame and retry once — FFmpeg's
        // recommended handling of AVERROR_INPUT_CHANGED.
        if matches!(run_res, Err(ff::Error::InputChanged)) {
            *resampler = Some(build_resampler(&decoded)?);
            out = ff::frame::Audio::empty();
            run_res = resampler.as_mut().unwrap().run(&decoded, &mut out);
        }
        run_res
            .map_err(|e| TranscriptionError::AudioExtractionFailed(format!("resample: {e}")))?;
        append_i16(&out, pcm);
        *got_any = true;
    }
    Ok(())
}

/// Build a swresample context that converts `frame`'s actual format/layout/rate to
/// the FORCED Whisper output: 16 kHz / mono / s16 packed (interleaved).
fn build_resampler(
    frame: &ff::frame::Audio,
) -> Result<ff::software::resampling::Context, TranscriptionError> {
    // Use the canonical `default(channels)` mask — the caller stamps the same mask
    // onto each frame so `swr_convert_frame`'s descriptor check matches.
    let src_layout = ff::ChannelLayout::default(frame.channels() as i32);
    ff::software::resampling::Context::get(
        frame.format(),
        src_layout,
        frame.rate(),
        ff::format::Sample::I16(ff::format::sample::Type::Packed),
        ff::ChannelLayout::MONO,
        WHISPER_SAMPLE_RATE,
    )
    .map_err(|e| TranscriptionError::AudioExtractionFailed(format!("resampler init: {e}")))
}

/// Duration of an audio frame in seconds (`samples / sample_rate`).
fn frame_duration(frame: &ff::frame::Audio) -> f64 {
    let rate = frame.rate();
    if rate == 0 {
        return 0.0;
    }
    frame.samples() as f64 / f64::from(rate)
}

/// Append an interleaved signed-16 packed audio frame's samples to `out`.
fn append_i16(frame: &ff::frame::Audio, out: &mut Vec<i16>) {
    // Packed I16 → all samples live in plane 0 as `[i16]` (interleaved). Mono here,
    // so interleave is trivially one channel.
    if frame.samples() == 0 {
        return;
    }
    let data: &[i16] = frame.plane(0);
    // `plane()` returns `samples * channels` for packed layouts; mono → `samples`.
    out.extend_from_slice(&data[..frame.samples().min(data.len())]);
}

/// Write a minimal canonical WAV (PCM, 16-bit, mono) for `samples` at `sample_rate`.
fn write_wav_s16_mono(path: &Path, samples: &[i16], sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36 + data_bytes;

    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&riff_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    // fmt chunk
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    f.write_all(&1u16.to_le_bytes())?; // audio format 1 = PCM
    f.write_all(&num_channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&bits_per_sample.to_le_bytes())?;
    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<file>")
        .to_string()
}

/// Read a 16-bit PCM mono WAV into the `f32` samples whisper.cpp expects
/// (`[-1.0, 1.0]`). Mirrors the proven spike WAV reader
/// (`spikes/whisper-probe/src/main.rs`).
fn read_wav_i16_to_f32(path: &Path) -> Result<Vec<f32>, TranscriptionError> {
    let bytes = std::fs::read(path).map_err(|e| {
        TranscriptionError::AudioExtractionFailed(format!("read wav: {e}"))
    })?;
    let data_start = find_data_chunk(&bytes).ok_or_else(|| {
        TranscriptionError::AudioExtractionFailed("no data chunk in extracted wav".to_string())
    })?;
    Ok(bytes[data_start..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect())
}

fn find_data_chunk(bytes: &[u8]) -> Option<usize> {
    let mut i = 12usize; // skip RIFF header
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size =
            u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        if id == b"data" {
            return Some(i + 8);
        }
        i += 8 + size;
    }
    None
}

/// Resolve the model file path: `$PALMIER_MODEL_DIR/ggml-small.en.bin`, else the
/// spike location `%LOCALAPPDATA%\palmier-pro\models\ggml-small.en.bin`.
fn resolve_model_path() -> Result<PathBuf, TranscriptionError> {
    if let Ok(dir) = std::env::var(MODEL_DIR_ENV) {
        let p = PathBuf::from(dir).join(DEFAULT_MODEL_FILE);
        if p.exists() {
            return Ok(p);
        }
        return Err(TranscriptionError::ModelInstallFailed(format!(
            "model not found at {}",
            p.display()
        )));
    }
    // Fallback: the spike's known app-data location.
    if let Some(local) = dirs::data_local_dir() {
        let p = local
            .join("palmier-pro")
            .join("models")
            .join(DEFAULT_MODEL_FILE);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(TranscriptionError::ModelInstallFailed(format!(
        "{DEFAULT_MODEL_FILE} not found (set {MODEL_DIR_ENV})"
    )))
}

/// E10-S3 SEAM — minimal locale resolver stub.
///
/// Returns the BCP-47 tag for the bundled `.en` model (`"en"`) regardless of the
/// requested `preferred_locale`, which is all the default `ggml-small.en` build
/// supports. E10-S3 owns the real resolution algorithm (`match_locale` /
/// `best_supported_locale` over `sys-locale`); when it lands, replace this call in
/// [`transcribe`] with the S3 resolver. Kept private + tiny so the swap is a
/// one-line change at the call site.
fn resolve_locale_stub(_preferred_locale: Option<&str>) -> String {
    "en".to_string()
}

/// Transcribe an audio file. Mirrors the reference
/// `transcribe(fileURL:censorProfanity:preferredLocale:sourceRange:)`.
///
/// * `file_url` — the audio/video asset to transcribe.
/// * `censor_profanity` — threaded to the (E10-S3-owned) suppression path; the S2
///   stub does not yet censor. See `// E10-S3 SEAM`.
/// * `locale` — preferred BCP-47 locale (e.g. `"en-US"`); resolved via the S3 seam.
/// * `range` — optional source-seconds window; when given, the extracted range is
///   transcribed and the result is `offsetting`-shifted by `range.start()`.
pub fn transcribe(
    file_url: &Path,
    censor_profanity: bool,
    locale: Option<&str>,
    range: Option<&RangeInclusive<f64>>,
) -> Result<TranscriptionResult, TranscriptionError> {
    // Range path: extract the window, transcribe it (no nested range), then offset
    // back into source time — parity with the reference recursive `if let range`.
    if let Some(range) = range {
        let temp = extract_audio_track(file_url, Some(range))?;
        let result = transcribe_extracted(temp.path(), censor_profanity, locale)?;
        return Ok(result.offsetting(*range.start()));
        // `temp` drops here → file deleted (reference `defer`).
    }

    // Whole-file path: still extract (force the 16 kHz/mono/s16 PCM Whisper needs),
    // then transcribe the extracted PCM. The reference reads the already-PCM temp
    // file directly; we always normalize through FFmpeg so any input container works.
    let temp = extract_audio_track(file_url, None)?;
    transcribe_extracted(temp.path(), censor_profanity, locale)
}

/// Extract + transcribe a clip's audio, offsetting by the range lower bound (0 when
/// no range). Mirrors the reference `transcribeVideoAudio`.
pub fn transcribe_video_audio(
    video_url: &Path,
    censor_profanity: bool,
    locale: Option<&str>,
    range: Option<&RangeInclusive<f64>>,
) -> Result<TranscriptionResult, TranscriptionError> {
    let temp = extract_audio_track(video_url, range)?;
    let result = transcribe_extracted(temp.path(), censor_profanity, locale)?;
    let offset = range.map(|r| *r.start()).unwrap_or(0.0);
    Ok(result.offsetting(offset))
}

/// Run whisper.cpp over an already-extracted 16 kHz mono PCM `.wav` file and decode
/// the segments/words into a [`TranscriptionResult`].
fn transcribe_extracted(
    pcm_wav: &Path,
    censor_profanity: bool,
    locale: Option<&str>,
) -> Result<TranscriptionResult, TranscriptionError> {
    // E10-S3 SEAM: resolve the locale (stub → "en" for the .en model). Swap this
    // call for the S3 resolver when it lands; `censor_profanity` is threaded for the
    // S3 token-suppression path (no-op in S2).
    let _ = censor_profanity;
    let resolved_locale = resolve_locale_stub(locale);

    let model_path = resolve_model_path()?;
    let model_path_str = model_path.to_string_lossy().to_string();

    let ctx = WhisperContext::new_with_params(&model_path_str, WhisperContextParameters::default())
        .map_err(|e| TranscriptionError::ModelInstallFailed(format!("load model: {e}")))?;
    let mut state = ctx
        .create_state()
        .map_err(|e| TranscriptionError::AnalysisFailed(format!("create state: {e}")))?;

    let samples = read_wav_i16_to_f32(pcm_wav)?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    // Word-level token timestamps — needed for per-word start/end (dominant-track
    // midpoint logic downstream). `.en` model → English; resolved locale is `en`.
    params.set_token_timestamps(true);
    // Force English for the bundled `.en` model (S3 will generalize per locale).
    params.set_language(Some("en"));

    state
        .full(params, &samples)
        .map_err(|e| TranscriptionError::AnalysisFailed(format!("whisper full: {e}")))?;

    decode_results(&state, &resolved_locale)
}

/// Decode whisper.cpp state into a [`TranscriptionResult`]. Parity port of the
/// reference `decodeResults`:
/// * For each segment: append raw text to `full_text`; a trimmed, non-empty segment
///   text → a [`TranscriptionSegment`] with the segment's `[start, end]` (seconds).
/// * Walk per-word token runs: trim each; skip empty; `start = run.start`,
///   `end = run.start + duration`; push a [`TranscriptionWord`].
/// * `result.text = full_text.trim()`, `result.language = <resolved bcp47>`.
///
/// whisper.cpp timestamps are **centiseconds** (1/100 s) — converted to seconds.
fn decode_results(
    state: &whisper_rs::WhisperState,
    locale_bcp47: &str,
) -> Result<TranscriptionResult, TranscriptionError> {
    let n_segments = state.full_n_segments();

    let mut words: Vec<TranscriptionWord> = Vec::new();
    let mut segments: Vec<TranscriptionSegment> = Vec::new();
    let mut full_text = String::new();

    for i in 0..n_segments {
        let Some(seg) = state.get_segment(i) else {
            continue;
        };
        let raw = seg.to_str_lossy().map_err(|_| TranscriptionError::DecodeFailed)?;
        // Append the raw segment text (with its leading space, as whisper emits it)
        // to full_text — parity with `fullText += String(attributed.characters)`.
        full_text.push_str(&raw);

        let seg_start = seg.start_timestamp() as f64 / 100.0;
        let seg_end = seg.end_timestamp() as f64 / 100.0;

        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            segments.push(TranscriptionSegment {
                text: trimmed.to_string(),
                start: seg_start,
                end: seg_end,
            });
        }

        // Per-word token runs. whisper-rs 0.16 exposes per-token text + timestamps
        // via `WhisperSegment::get_token(i) -> Option<WhisperToken>`; `token_data()`
        // carries `t0`/`t1` in centiseconds. token_timestamps was enabled in params.
        let n_tokens = seg.n_tokens();
        for t in 0..n_tokens {
            let Some(token) = seg.get_token(t) else {
                continue;
            };
            let Ok(token_text) = token.to_str_lossy() else {
                continue;
            };
            let trimmed_tok = token_text.trim();
            if trimmed_tok.is_empty() {
                continue;
            }
            // Skip whisper's special tokens (e.g. `[_BEG_]`, `<|...|>`) which carry no
            // spoken text — they would otherwise pollute the word list. The
            // bracketed-text check is the robust, binding-stable filter.
            if trimmed_tok.starts_with("[_") || trimmed_tok.starts_with("<|") {
                continue;
            }
            let data = token.token_data();
            // run.start = t0 (centiseconds → seconds); end = t0 + duration = t1.
            // Guard against the sentinel `<0` whisper uses for "no timestamp".
            let (start, end) = if data.t0 < 0 || data.t1 < 0 {
                (None, None)
            } else {
                (Some(data.t0 as f64 / 100.0), Some(data.t1 as f64 / 100.0))
            };
            words.push(TranscriptionWord {
                text: trimmed_tok.to_string(),
                start,
                end,
            });
        }
    }

    Ok(TranscriptionResult {
        text: full_text.trim().to_string(),
        language: Some(locale_bcp47.to_string()),
        words,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WAV writer + reader round-trip a known signal byte-exactly (s16le mono).
    #[test]
    fn wav_round_trips_s16_mono() {
        let samples: Vec<i16> = vec![0, 16384, -16384, 32767, -32768, 100];
        let tmp = std::env::temp_dir().join(format!("palmier-stt-test-{}.wav", uuid::Uuid::new_v4()));
        write_wav_s16_mono(&tmp, &samples, WHISPER_SAMPLE_RATE).expect("write");
        let back = read_wav_i16_to_f32(&tmp).expect("read");
        assert_eq!(back.len(), samples.len());
        for (orig, got) in samples.iter().zip(back.iter()) {
            assert!((*got - (*orig as f32 / 32768.0)).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&tmp);
    }

    /// `TempAudioFile` deletes its file on drop (RAII parity with the reference defer).
    #[test]
    fn temp_audio_file_deletes_on_drop() {
        let tmp = std::env::temp_dir().join(format!("palmier-stt-drop-{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, b"x").expect("seed");
        assert!(tmp.exists());
        {
            let guard = TempAudioFile { path: tmp.clone() };
            assert_eq!(guard.path(), tmp.as_path());
        }
        assert!(!tmp.exists(), "temp file should be deleted on drop");
    }

    /// The locale stub resolves to `"en"` for the bundled `.en` model regardless of
    /// the requested locale (E10-S3 owns the real resolver).
    #[test]
    fn locale_stub_is_en() {
        assert_eq!(resolve_locale_stub(None), "en");
        assert_eq!(resolve_locale_stub(Some("fr-FR")), "en");
    }

    /// Missing audio track / unreadable input → `AudioExtractionFailed`.
    #[test]
    fn extract_missing_file_errors() {
        let missing = std::env::temp_dir().join("palmier-stt-does-not-exist-xyz.mp4");
        let err = extract_audio_track(&missing, None).expect_err("should fail");
        assert!(matches!(err, TranscriptionError::AudioExtractionFailed(_)));
    }

    // ---- LIVE whisper transcription (gated on the model + a generated fixture) ----
    //
    // Runs only when the bundled model resolves (PALMIER_MODEL_DIR or the spike
    // location) AND ffmpeg can generate a fixture. On CI/dev boxes without the model
    // this self-skips (the model is ~466 MB and not committed). The CPU lane records
    // its own timing and is NOT held to SM-9's 2-min/RTX-4060 bar (story note).

    /// Generate a short speech WAV by speaking text isn't available offline, so we
    /// use the committed-free path: synthesize via ffmpeg is silence-only and yields
    /// no words. Instead this live test transcribes a real fixture **iff** one is
    /// provided via `PALMIER_TEST_AUDIO` (a path to any audio/video file). It asserts
    /// the pipeline runs end-to-end and produces a non-error result.
    #[test]
    fn live_transcribe_real_fixture() {
        let Ok(fixture) = std::env::var("PALMIER_TEST_AUDIO") else {
            eprintln!("SKIP live_transcribe_real_fixture: set PALMIER_TEST_AUDIO to a media file");
            return;
        };
        if resolve_model_path().is_err() {
            eprintln!("SKIP live_transcribe_real_fixture: model not found (set PALMIER_MODEL_DIR)");
            return;
        }
        let path = PathBuf::from(fixture);
        let t0 = std::time::Instant::now();
        let result = transcribe(&path, false, Some("en-US"), None).expect("transcription");
        let elapsed = t0.elapsed();
        eprintln!(
            "LIVE transcribe: {} chars, {} words, {} segments, lang={:?}, CPU time={:?}",
            result.text.len(),
            result.words.len(),
            result.segments.len(),
            result.language,
            elapsed
        );
        eprintln!("LIVE text: {}", result.text);
        assert!(!result.text.is_empty(), "expected non-empty transcript");
        assert_eq!(result.language.as_deref(), Some("en"));
    }
}
