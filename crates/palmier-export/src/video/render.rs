//! The video export render loop — E6-S5. **Feature-gated behind `gpu-export`.**
//!
//! Reproduces `ExportService.makeExportSession` on the Windows/Linux stack:
//!
//! ```text
//! for frame in 0..total:                    (cancellation checked each frame)
//!   build_frame(timeline, frame)            (the SHARED palmier-engine builder)
//!     → Compositor::render (offscreen wgpu)  (the E5-S8 readback path)
//!     → read_back() RGBA
//!     → FFmpeg video encoder (HW: NVENC/QSV/AMF/MF, or prores_ks)
//!   progress(frame / total)
//! mix audio (palmier-engine mixer) → AAC track
//! mux + finalize (.mp4 / .mov)
//! ```
//!
//! The engine compositor renders **offscreen** (no WebView presentation), so this
//! path is independent of Spike S-1. `palmier-export` never opens an
//! `AVFormatContext` for *decode* — it fetches decoded frames through
//! `palmier-media`'s `FrameSource` (the one-decode-owner contract). It *does* open
//! an FFmpeg **output** context for the encode/mux (that's the export sink, the
//! reference's `AVAssetExportSession`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use palmier_engine::audio::{mix_to_bus, AudioTrack, ClipAudio};
use palmier_engine::compositor::gpu::Compositor;
use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{
    build_frame, Canvas, QualityTarget, RenderFrame, RgbaImage, SourceResolver,
};
use palmier_model::Timeline;

use super::encoder::{ffmpeg_encoder_available, select_encoder, EncoderPlan, HwVendor};
use super::spec::{frame_count, render_size, ExportFormat, ExportResolution, BT709};
use super::ExportError;

use ffmpeg_next as ff;

/// A shared cancellation flag the caller sets to abort an in-flight export. The
/// pipeline checks it **at each frame boundary** (FOUNDATION §6.12); a set flag
/// produces a clean [`ExportError::Cancelled`] (the partial output is removed),
/// not an error condition (mirrors the reference `NSUserCancelledError`-as-cancel).
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A fresh, un-cancelled flag.
    pub fn new() -> Self {
        CancelFlag(Arc::new(AtomicBool::new(false)))
    }

    /// Request cancellation (idempotent). The next frame boundary observes it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The configuration for one video export.
#[derive(Debug, Clone)]
pub struct VideoExportConfig {
    /// Output codec/container (H.264/H.265 `.mp4` or ProRes 422 `.mov`).
    pub format: ExportFormat,
    /// Resolution preset (short-side px → even-snapped encode dims).
    pub resolution: ExportResolution,
    /// Where to write the file. The pipeline **deletes** an existing file here
    /// first (reference behavior; the FFmpeg muxer must not fail on an existing
    /// file).
    pub output_path: PathBuf,
    /// Output frame rate. For v1 this should equal the project fps; `0` means
    /// "use the project fps" (variable output fps is out of scope — Open
    /// Questions). The encoded stream's time base is `1/output_fps`.
    pub output_fps: u32,
}

/// What an export produced, for the caller's report / log.
#[derive(Debug, Clone)]
pub struct VideoExportOutcome {
    /// The file written.
    pub output_path: PathBuf,
    /// The encode dimensions (even-snapped render size).
    pub width: u32,
    /// The encode height.
    pub height: u32,
    /// Total frames encoded.
    pub frames: u64,
    /// The FFmpeg encoder that was selected (diagnostic).
    pub encoder: &'static str,
    /// Which HW path / encoder was used.
    pub vendor: HwVendor,
    /// Whether an audio (AAC) track was muxed.
    pub has_audio: bool,
}

/// Pre-decoded audio for the export: the per-track mute flag + each track's
/// already-resampled-and-stretched 48 kHz clip buffers, exactly what the engine
/// mixer ([`mix_to_bus`]) consumes. The caller (which owns the decode/resample/
/// stretch stage) supplies this; passing an **empty** slice exports a video-only
/// file (no audio track).
pub type AudioInput = Vec<(AudioTrack, Vec<ClipAudio>)>;

/// Export `timeline` to a video file per `config`.
///
/// * `geometry` resolves each `media_ref` to its [`SourceInfo`](palmier_engine::SourceInfo)
///   (natural size + preferred transform) for the composition build — the same
///   resolver the preview build uses.
/// * `frames` is the decode handle (`palmier-media`'s `FrameSource`, or any
///   [`FrameProvider`]) the compositor pulls pixels from (one-decode-owner).
/// * `audio` is the pre-decoded per-track audio to mix → AAC (empty ⇒ no audio).
/// * `progress` is called with `0.0..=1.0` after each encoded frame (the
///   reference polled `session.progress` every 200 ms; here it's exact per-frame).
/// * `cancel` is checked at each frame boundary.
///
/// Returns [`ExportError::Cancelled`] (cleanly) if cancelled,
/// [`ExportError::NoHardwareEncoder`] if H.264/H.265 has no HW encoder,
/// [`ExportError::NoGpu`] if the offscreen compositor can't be created.
#[allow(clippy::too_many_arguments)]
pub fn export_video<R, P>(
    timeline: &Timeline,
    geometry: &R,
    frames: &P,
    audio: &AudioInput,
    config: &VideoExportConfig,
    mut progress: impl FnMut(f64),
    cancel: &CancelFlag,
) -> Result<VideoExportOutcome, ExportError>
where
    R: SourceResolver,
    P: FrameProvider,
{
    ensure_ffmpeg_init()?;

    // 1. Select the encoder (HW for H.264/H.265, prores_ks for ProRes), erroring
    //    clearly if no HW encoder is available in this LGPL build.
    let plan = select_encoder(config.format, ffmpeg_encoder_available)?;

    // 2. Encode dimensions: even-snapped render size for the resolution preset.
    let (enc_w, enc_h) = render_size(timeline.width as u32, timeline.height as u32, config.resolution);

    // 3. Frame count (v1: project fps == output fps).
    let project_fps = timeline.fps.max(1) as u32;
    let output_fps = if config.output_fps == 0 { project_fps } else { config.output_fps };
    let total = frame_count(timeline.total_frames(), project_fps, output_fps);
    if total == 0 {
        return Err(ExportError::Empty);
    }

    // 4. Output precondition: delete any existing file (reference behavior).
    if config.output_path.exists() {
        std::fs::remove_file(&config.output_path).map_err(|e| ExportError::Io(e.to_string()))?;
    }

    // 5. Stand up the offscreen compositor at the encode size (no surface → no
    //    WebView presentation; independent of Spike S-1). NoAdapter ⇒ NoGpu so
    //    the caller can fall back to the CPU lane (R-8).
    let mut compositor = Compositor::new_headless(enc_w, enc_h)
        .map_err(|e| ExportError::NoGpu(e.to_string()))?;

    // Early cancel: if cancelled before we open the output, never create a file.
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }

    // 6. Open the FFmpeg output (muxer + video encoder, + AAC if audio present).
    let has_audio = !audio.is_empty();
    let mut sink = VideoSink::open(
        &config.output_path,
        config.format,
        &plan,
        enc_w,
        enc_h,
        output_fps,
        has_audio,
    )?;

    // 7. Per-frame loop: build → render → readback → encode. Cancellation is
    //    checked at the TOP of each frame (a frame boundary).
    let canvas = Canvas::new(enc_w, enc_h);
    for out_frame in 0..total {
        if cancel.is_cancelled() {
            // Drop the sink (closes the FFmpeg output context, which flushes on
            // drop) BEFORE removing the half-written file, else the Drop would
            // recreate it after we delete.
            sink.abort();
            drop(sink);
            let _ = std::fs::remove_file(&config.output_path);
            return Err(ExportError::Cancelled);
        }

        // For v1 (output_fps == project_fps) the timeline frame == out_frame.
        // With output fps scaling this maps output time back to a project frame.
        let project_frame = if output_fps == project_fps {
            out_frame as i32
        } else {
            ((out_frame as f64) * (project_fps as f64) / (output_fps as f64)).round() as i32
        };

        // Build the composition (SHARED engine builder — same path as preview).
        let composition = build_frame(timeline, project_frame, geometry);
        let render_frame = RenderFrame::new(composition, canvas, QualityTarget::Full);

        // Render offscreen (the E5-S8 readback path) + read back RGBA.
        compositor
            .render(&render_frame, frames)
            .map_err(|e| ExportError::Ffmpeg(format!("compositor render: {e}")))?;
        let rgba = compositor
            .read_back()
            .ok_or_else(|| ExportError::Ffmpeg("offscreen readback returned None".into()))?;

        // Encode the frame at presentation timestamp = out_frame.
        sink.encode_video_frame(&rgba, out_frame as i64)?;

        progress(((out_frame + 1) as f64) / (total as f64));
    }

    // 8. Audio: mix the pre-decoded tracks → 48 kHz bus → AAC track.
    if has_audio {
        let output_len = audio_output_len(total, output_fps);
        let bus = mix_to_bus(audio, output_fps, output_len);
        sink.encode_audio(&bus)?;
    }

    // 9. Flush encoders + write the trailer (finalize the container).
    sink.finalize()?;

    Ok(VideoExportOutcome {
        output_path: config.output_path.clone(),
        width: enc_w,
        height: enc_h,
        frames: total,
        encoder: plan.ffmpeg_name,
        vendor: plan.vendor,
        has_audio,
    })
}

/// Output sample count for the AAC track: `total_frames / fps` seconds at the
/// 48 kHz project rate.
fn audio_output_len(total_frames: u64, fps: u32) -> usize {
    if fps == 0 {
        return 0;
    }
    let seconds = total_frames as f64 / fps as f64;
    (seconds * palmier_engine::audio::PROJECT_SAMPLE_RATE_HZ as f64).round() as usize
}

/// Ensure ffmpeg is initialized exactly once.
fn ensure_ffmpeg_init() -> Result<(), ExportError> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    let mut err: Option<String> = None;
    INIT.call_once(|| {
        if let Err(e) = ff::init() {
            err = Some(e.to_string());
        }
    });
    match err {
        Some(e) => Err(ExportError::Ffmpeg(e)),
        None => Ok(()),
    }
}

// =============================================================================
// VideoSink — the FFmpeg output context (muxer + video/audio encoders).
// =============================================================================

/// The FFmpeg output: an output format context with a video stream (the selected
/// HW/ProRes encoder) and an optional AAC audio stream. Owns the encoders, the
/// muxer, and the RGBA→encoder-pixfmt scaler.
struct VideoSink {
    octx: ff::format::context::Output,
    video_encoder: ff::encoder::Video,
    video_stream_index: usize,
    /// RGBA → encoder pixel format (nv12 / yuv422p10le) converter.
    scaler: ff::software::scaling::Context,
    enc_w: u32,
    enc_h: u32,
    /// 1/output_fps video time base.
    video_time_base: ff::Rational,
    audio: Option<AudioEncoderState>,
    finalized: bool,
    aborted: bool,
}

/// The AAC encoder + its stream + the running sample-pts cursor.
struct AudioEncoderState {
    encoder: ff::encoder::Audio,
    stream_index: usize,
    next_pts: i64,
    time_base: ff::Rational,
}

impl VideoSink {
    /// Open the output container at `path`, add the video stream with `plan`'s
    /// encoder configured for `enc_w × enc_h @ output_fps` with BT.709 tags, and
    /// (if `with_audio`) an AAC stereo 48 kHz audio stream. Writes the header.
    fn open(
        path: &Path,
        format: ExportFormat,
        plan: &EncoderPlan,
        enc_w: u32,
        enc_h: u32,
        output_fps: u32,
        with_audio: bool,
    ) -> Result<Self, ExportError> {
        let mut octx = ff::format::output_as(&path, format.muxer())
            .map_err(|e| ExportError::Ffmpeg(format!("open output {}: {e}", path.display())))?;

        // --- Video stream + encoder ---
        let codec = ff::encoder::find_by_name(plan.ffmpeg_name).ok_or_else(|| {
            ExportError::Ffmpeg(format!("encoder {} vanished after probe", plan.ffmpeg_name))
        })?;
        let video_time_base = ff::Rational::new(1, output_fps as i32);
        let enc_pix_fmt = pixel_from_name(plan.pix_fmt)?;

        // Read the muxer's global-header requirement before borrowing octx via
        // `add_stream` (the stream handle holds that mutable borrow).
        let global_header = octx
            .format()
            .flags()
            .contains(ff::format::Flags::GLOBAL_HEADER);

        let mut vstream = octx
            .add_stream(codec)
            .map_err(|e| ExportError::Ffmpeg(format!("add video stream: {e}")))?;
        let video_stream_index = vstream.index();

        let mut venc = ff::codec::context::Context::from_parameters(vstream.parameters())
            .map_err(|e| ExportError::Ffmpeg(format!("video codec ctx: {e}")))?
            .encoder()
            .video()
            .map_err(|e| ExportError::Ffmpeg(format!("video encoder: {e}")))?;

        venc.set_width(enc_w);
        venc.set_height(enc_h);
        venc.set_format(enc_pix_fmt);
        venc.set_time_base(video_time_base);
        venc.set_frame_rate(Some(ff::Rational::new(output_fps as i32, 1)));
        // BT.709 color tags on the encoded stream (risk #5 single working space).
        // ffmpeg-next 7.1's safe encoder API exposes `set_colorspace` (the YCbCr
        // matrix — the load-bearing tag a decoder reads to convert YUV→RGB) and
        // `set_color_range`; it does NOT expose primaries/transfer setters on the
        // encoder, so those default to the muxer/codec's BT.709 for HD. The
        // matrix + range we set here are the parity-critical ones.
        venc.set_color_range(ff::color::Range::MPEG);
        venc.set_colorspace(ff::color::Space::BT709);
        let _ = BT709; // the BT.709 enum codes (primaries/transfer/matrix all = 1)

        // Some muxers require the global-header flag (mp4/mov).
        if global_header {
            venc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }

        // Encoder-specific options: balanced preset + bitrate for the HW encoders,
        // profile for ProRes 422.
        let opts = encoder_options(format, plan, enc_w, enc_h, output_fps);
        let video_encoder = venc
            .open_as_with(codec, opts)
            .map_err(|e| ExportError::Ffmpeg(format!("open video encoder {}: {e}", plan.ffmpeg_name)))?;
        vstream.set_parameters(&video_encoder);
        vstream.set_time_base(video_time_base);

        // --- RGBA → encoder pixfmt scaler ---
        let scaler = ff::software::scaling::Context::get(
            ff::format::Pixel::RGBA,
            enc_w,
            enc_h,
            enc_pix_fmt,
            enc_w,
            enc_h,
            ff::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| ExportError::Ffmpeg(format!("rgba scaler: {e}")))?;

        // --- Optional AAC audio stream ---
        let audio = if with_audio {
            Some(Self::open_audio(&mut octx)?)
        } else {
            None
        };

        octx.write_header()
            .map_err(|e| ExportError::Ffmpeg(format!("write header: {e}")))?;

        Ok(VideoSink {
            octx,
            video_encoder,
            video_stream_index,
            scaler,
            enc_w,
            enc_h,
            video_time_base,
            audio,
            finalized: false,
            aborted: false,
        })
    }

    /// Add a stereo 48 kHz AAC stream + encoder (LGPL-clean native AAC).
    fn open_audio(octx: &mut ff::format::context::Output) -> Result<AudioEncoderState, ExportError> {
        let codec = ff::encoder::find(ff::codec::Id::AAC)
            .ok_or_else(|| ExportError::Ffmpeg("native AAC encoder not found".into()))?;
        let sample_rate = palmier_engine::audio::PROJECT_SAMPLE_RATE_HZ as i32;
        let time_base = ff::Rational::new(1, sample_rate);

        // Read the muxer's global-header requirement before borrowing octx
        // mutably via `add_stream` (the stream handle holds that borrow).
        let global_header = octx
            .format()
            .flags()
            .contains(ff::format::Flags::GLOBAL_HEADER);

        let mut astream = octx
            .add_stream(codec)
            .map_err(|e| ExportError::Ffmpeg(format!("add audio stream: {e}")))?;
        let stream_index = astream.index();

        let mut aenc = ff::codec::context::Context::from_parameters(astream.parameters())
            .map_err(|e| ExportError::Ffmpeg(format!("audio codec ctx: {e}")))?
            .encoder()
            .audio()
            .map_err(|e| ExportError::Ffmpeg(format!("audio encoder: {e}")))?;

        aenc.set_rate(sample_rate);
        aenc.set_channel_layout(ff::channel_layout::ChannelLayout::STEREO);
        aenc.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
        aenc.set_time_base(time_base);
        aenc.set_bit_rate(192_000);

        if global_header {
            aenc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }

        let encoder = aenc
            .open_as(codec)
            .map_err(|e| ExportError::Ffmpeg(format!("open AAC encoder: {e}")))?;
        astream.set_parameters(&encoder);
        astream.set_time_base(time_base);

        Ok(AudioEncoderState {
            encoder,
            stream_index,
            next_pts: 0,
            time_base,
        })
    }

    /// Convert one readback RGBA frame → encoder pixfmt, set its pts, send to the
    /// encoder, and mux the resulting packets.
    fn encode_video_frame(&mut self, rgba: &RgbaImage, pts: i64) -> Result<(), ExportError> {
        // Wrap the readback bytes in an RGBA ffmpeg frame (tight rows).
        let mut src = ff::frame::Video::new(ff::format::Pixel::RGBA, self.enc_w, self.enc_h);
        copy_rgba_into_frame(rgba, &mut src);

        let mut dst = ff::frame::Video::empty();
        self.scaler
            .run(&src, &mut dst)
            .map_err(|e| ExportError::Ffmpeg(format!("scale frame: {e}")))?;
        dst.set_pts(Some(pts));

        self.video_encoder
            .send_frame(&dst)
            .map_err(|e| ExportError::Ffmpeg(format!("send video frame: {e}")))?;
        self.drain_video()?;
        Ok(())
    }

    /// Drain + mux any ready video packets.
    fn drain_video(&mut self) -> Result<(), ExportError> {
        let mut packet = ff::Packet::empty();
        while self.video_encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.video_stream_index);
            packet.rescale_ts(
                self.video_time_base,
                self.octx.stream(self.video_stream_index).unwrap().time_base(),
            );
            packet
                .write_interleaved(&mut self.octx)
                .map_err(|e| ExportError::Ffmpeg(format!("write video packet: {e}")))?;
        }
        Ok(())
    }

    /// Encode the mixed mono 48 kHz `bus` as a stereo AAC track (the mono bus is
    /// duplicated to both channels — the reference downmixes to a 2-channel
    /// output). Splits the bus into the encoder's frame size.
    fn encode_audio(&mut self, bus: &[f32]) -> Result<(), ExportError> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        let frame_size = if audio.encoder.frame_size() > 0 {
            audio.encoder.frame_size() as usize
        } else {
            1024
        };
        let rate = palmier_engine::audio::PROJECT_SAMPLE_RATE_HZ as i32;

        let mut offset = 0usize;
        while offset < bus.len() {
            let n = frame_size.min(bus.len() - offset);
            let mut frame = ff::frame::Audio::new(
                ff::format::Sample::F32(ff::format::sample::Type::Planar),
                n,
                ff::channel_layout::ChannelLayout::STEREO,
            );
            frame.set_rate(rate as u32);
            frame.set_pts(Some(audio.next_pts));
            // Planar f32: plane 0 = L, plane 1 = R (duplicate the mono bus).
            {
                let l: &mut [f32] = frame.plane_mut(0);
                l[..n].copy_from_slice(&bus[offset..offset + n]);
            }
            {
                let r: &mut [f32] = frame.plane_mut(1);
                r[..n].copy_from_slice(&bus[offset..offset + n]);
            }
            audio.next_pts += n as i64;

            audio
                .encoder
                .send_frame(&frame)
                .map_err(|e| ExportError::Ffmpeg(format!("send audio frame: {e}")))?;
            Self::drain_audio(&mut self.octx, audio)?;
            offset += n;
        }
        Ok(())
    }

    /// Drain + mux ready audio packets.
    fn drain_audio(
        octx: &mut ff::format::context::Output,
        audio: &mut AudioEncoderState,
    ) -> Result<(), ExportError> {
        let mut packet = ff::Packet::empty();
        while audio.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(audio.stream_index);
            packet.rescale_ts(
                audio.time_base,
                octx.stream(audio.stream_index).unwrap().time_base(),
            );
            packet
                .write_interleaved(octx)
                .map_err(|e| ExportError::Ffmpeg(format!("write audio packet: {e}")))?;
        }
        Ok(())
    }

    /// Flush the encoders (send EOF, drain) and write the container trailer.
    fn finalize(&mut self) -> Result<(), ExportError> {
        if self.finalized || self.aborted {
            return Ok(());
        }
        // Flush video.
        self.video_encoder
            .send_eof()
            .map_err(|e| ExportError::Ffmpeg(format!("video eof: {e}")))?;
        self.drain_video()?;
        // Flush audio.
        if let Some(audio) = self.audio.as_mut() {
            audio
                .encoder
                .send_eof()
                .map_err(|e| ExportError::Ffmpeg(format!("audio eof: {e}")))?;
            Self::drain_audio(&mut self.octx, audio)?;
        }
        self.octx
            .write_trailer()
            .map_err(|e| ExportError::Ffmpeg(format!("write trailer: {e}")))?;
        self.finalized = true;
        Ok(())
    }

    /// Mark the sink aborted (cancellation) so `Drop`/`finalize` don't try to
    /// write a trailer on a half-built file.
    fn abort(&mut self) {
        self.aborted = true;
    }
}

/// Encoder-specific options (balanced preset + bitrate for HW H.264/H.265,
/// profile for ProRes 422). Returns an FFmpeg `Dictionary` of option key→value.
fn encoder_options(
    format: ExportFormat,
    plan: &EncoderPlan,
    enc_w: u32,
    enc_h: u32,
    fps: u32,
) -> ff::Dictionary<'static> {
    let mut opts = ff::Dictionary::new();
    match format {
        ExportFormat::ProRes422 => {
            // prores_ks profile 2 = ProRes 422 (standard). LPCM audio is handled
            // by the .mov muxer + AAC/PCM choice; v1 ships AAC for size.
            opts.set("profile", "2");
        }
        ExportFormat::H264 | ExportFormat::H265 => {
            // A target bitrate keyed off resolution (the reference's preset table
            // maps to fixed bitrates; we approximate "balanced"). NVENC/QSV/AMF
            // all accept a numeric bitrate; the preset name differs per encoder,
            // so set the portable knobs and a per-vendor balanced preset.
            let bitrate = balanced_bitrate(enc_w, enc_h, fps, format);
            opts.set("b", &bitrate.to_string());
            match plan.vendor {
                HwVendor::Nvenc => {
                    // p4 == balanced quality/speed on NVENC's p1..p7 scale.
                    opts.set("preset", "p4");
                    opts.set("tune", "hq");
                }
                HwVendor::Qsv => {
                    opts.set("preset", "medium");
                }
                HwVendor::Amf => {
                    opts.set("quality", "balanced");
                }
                HwVendor::MediaFoundation => {
                    // MF exposes few knobs; bitrate is the portable one.
                }
                HwVendor::ProResKs => {}
            }
        }
    }
    opts
}

/// A "balanced" target bitrate (bits/sec) for the HW H.264/H.265 encode, scaled
/// by pixel count and codec (H.265 is ~half the bits of H.264 for parity). Rough
/// parity with the reference's per-resolution preset bitrates.
fn balanced_bitrate(w: u32, h: u32, fps: u32, format: ExportFormat) -> i64 {
    let pixels = (w as i64) * (h as i64);
    // ~0.10 bits per pixel per frame for H.264 balanced; H.265 ~0.05.
    let bpp = match format {
        ExportFormat::H265 => 0.05,
        _ => 0.10,
    };
    let per_frame = (pixels as f64) * bpp;
    (per_frame * fps.max(1) as f64) as i64
}

/// Resolve an FFmpeg pixel format by its short name.
fn pixel_from_name(name: &str) -> Result<ff::format::Pixel, ExportError> {
    match name {
        "nv12" => Ok(ff::format::Pixel::NV12),
        "yuv420p" => Ok(ff::format::Pixel::YUV420P),
        "yuv422p10le" => Ok(ff::format::Pixel::YUV422P10LE),
        other => Err(ExportError::Ffmpeg(format!("unknown encoder pixfmt {other}"))),
    }
}

/// Copy a tightly-packed RGBA readback image into an FFmpeg RGBA `Video` frame,
/// honoring the frame's row stride (FFmpeg pads rows for alignment).
fn copy_rgba_into_frame(rgba: &RgbaImage, frame: &mut ff::frame::Video) {
    let w = rgba.width as usize;
    let h = rgba.height as usize;
    let src_row = w * 4;
    let stride = frame.stride(0);
    let data = frame.data_mut(0);
    for y in 0..h {
        let src_start = y * src_row;
        let dst_start = y * stride;
        let src_end = src_start + src_row;
        if src_end > rgba.bytes.len() || dst_start + src_row > data.len() {
            break;
        }
        data[dst_start..dst_start + src_row].copy_from_slice(&rgba.bytes[src_start..src_end]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flag_round_trips() {
        let f = CancelFlag::new();
        assert!(!f.is_cancelled());
        let clone = f.clone();
        clone.cancel();
        assert!(f.is_cancelled(), "cancel visible across clones");
    }

    #[test]
    fn audio_output_len_is_seconds_times_rate() {
        // 1800 frames @ 30 fps = 60 s; 60 s × 48000 = 2_880_000 samples.
        assert_eq!(audio_output_len(1800, 30), 2_880_000);
        assert_eq!(audio_output_len(0, 30), 0);
        assert_eq!(audio_output_len(100, 0), 0);
    }

    #[test]
    fn balanced_bitrate_scales_with_pixels_and_codec() {
        let h264_1080 = balanced_bitrate(1920, 1080, 30, ExportFormat::H264);
        let h264_720 = balanced_bitrate(1280, 720, 30, ExportFormat::H264);
        let h265_1080 = balanced_bitrate(1920, 1080, 30, ExportFormat::H265);
        assert!(h264_1080 > h264_720, "more pixels → more bits");
        assert!(h265_1080 < h264_1080, "HEVC uses ~half the bits of H.264");
        assert!(h264_1080 > 0);
    }

    #[test]
    fn pixel_names_resolve() {
        assert_eq!(pixel_from_name("nv12").unwrap(), ff::format::Pixel::NV12);
        assert_eq!(
            pixel_from_name("yuv422p10le").unwrap(),
            ff::format::Pixel::YUV422P10LE
        );
        assert!(pixel_from_name("bogus").is_err());
    }
}
