//! The FFmpeg-backed [`Decoder`] — one per source URL (story E5-S2).
//!
//! Ports the decode side of the macOS reference's `AVPlayer`/`AVAssetReader`
//! usage (`VideoEngine.swift`, generators) to an `ffmpeg-next` format context
//! plus codec context. Per the Glossary one-decode-owner contract there is
//! exactly one decoder per distinct source URL (the [`DecoderPool`] enforces
//! it); `palmier-engine` never opens its own format context.
//!
//! ## HW vs CPU decode
//! The decoder *attempts* a hardware device (d3d11va / dxva2 on Windows, vaapi
//! on Linux) and **falls back to CPU** when none initializes — the codec still
//! decodes, just on the CPU. We attempt the HW device via the `ffmpeg-next`
//! FFI (`av_hwdevice_ctx_create`) because the safe wrapper does not expose it on
//! 7.1; the attempt is best-effort and never fatal. HW *surfaces* still get
//! transferred down to CPU planes here (the engine's texture upload is E5-S8) —
//! we are a CPU-frame producer either way; HW decode only offloads the entropy/
//! IDCT work from the CPU. [`HwDecodeStatus`] records which path engaged.
//!
//! ## Source-frame addressing
//! A request is `(media_ref, source_frame)`. We convert the frame index to a
//! stream timestamp (`source_frame / fps` in the stream time base), seek to the
//! nearest keyframe at/under it, then decode forward until the frame whose index
//! equals the target. `f64::round` (ties away from zero) is used for every
//! time↔frame conversion, matching the carry-forward rounding rule.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffmpeg_next as ff;

use super::frame::{DecodedFrame, PixelLayout, Plane};

/// Which decode path the decoder ended up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwDecodeStatus {
    /// A hardware device initialized and is attached to the codec context.
    Hardware(HwKind),
    /// No HW device initialized (or none was attempted); decoding on the CPU.
    Cpu,
}

impl HwDecodeStatus {
    /// True if a hardware device is engaged.
    pub fn is_hardware(self) -> bool {
        matches!(self, HwDecodeStatus::Hardware(_))
    }
}

/// Hardware device families we attempt, in platform-preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwKind {
    /// Direct3D 11 Video Acceleration (preferred on Windows).
    D3d11va,
    /// DXVA2 (Windows fallback).
    Dxva2,
    /// VA-API (Linux).
    Vaapi,
}

impl HwKind {
    /// The `ffmpeg-next` device type for this kind.
    fn av_type(self) -> ff::ffi::AVHWDeviceType {
        match self {
            HwKind::D3d11va => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
            HwKind::Dxva2 => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_DXVA2,
            HwKind::Vaapi => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
        }
    }

    /// Platform-preferred HW kinds, most-preferred first.
    #[cfg(windows)]
    fn platform_order() -> &'static [HwKind] {
        &[HwKind::D3d11va, HwKind::Dxva2]
    }

    /// Platform-preferred HW kinds, most-preferred first.
    #[cfg(not(windows))]
    fn platform_order() -> &'static [HwKind] {
        &[HwKind::Vaapi]
    }
}

/// Errors the decoder can surface.
#[derive(Debug)]
pub enum DecodeError {
    /// ffmpeg open/decode/scale failure.
    Ffmpeg(String),
    /// No decodable video stream in the container.
    NoVideoStream,
    /// The requested source frame is past the end of the stream.
    FrameOutOfRange,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Ffmpeg(m) => write!(f, "ffmpeg: {m}"),
            DecodeError::NoVideoStream => write!(f, "no video stream"),
            DecodeError::FrameOutOfRange => write!(f, "source frame out of range"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<ff::Error> for DecodeError {
    fn from(e: ff::Error) -> Self {
        DecodeError::Ffmpeg(e.to_string())
    }
}

/// Ensure ffmpeg is initialized exactly once.
fn ensure_ffmpeg_init() -> Result<(), DecodeError> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    let mut err: Option<String> = None;
    INIT.call_once(|| {
        if let Err(e) = ff::init() {
            err = Some(e.to_string());
        }
    });
    match err {
        Some(e) => Err(DecodeError::Ffmpeg(e)),
        None => Ok(()),
    }
}

/// Map an ffmpeg pixel format to our [`PixelLayout`] tag, preferring the native
/// planar YUV layouts and tagging alpha from the **pixfmt** descriptor (risk #3).
/// Returns `None` for formats we don't carry natively (caller scales to RGBA).
fn layout_for(format: ff::format::Pixel) -> Option<(PixelLayout, bool)> {
    use ff::format::Pixel as P;
    match format {
        P::YUV420P | P::YUVJ420P => Some((PixelLayout::Yuv420P, false)),
        P::YUV422P | P::YUVJ422P => Some((PixelLayout::Yuv422P, false)),
        P::YUV444P | P::YUVJ444P => Some((PixelLayout::Yuv444P, false)),
        P::RGBA | P::BGRA | P::ARGB | P::ABGR => Some((PixelLayout::Rgba8, true)),
        _ => None,
    }
}

/// Whether an ffmpeg pixel format descriptor advertises an alpha channel —
/// the codec/pixfmt has-alpha check that replaces `CMFormatDescription`'s alpha
/// extension (risk #3: trust the codec, not the container).
fn pixfmt_has_alpha(format: ff::format::Pixel) -> bool {
    // SAFETY: `av_pix_fmt_desc_get` returns a pointer into a static table (or
    // null for an unknown format); we only read the flags field.
    unsafe {
        let desc = ff::ffi::av_pix_fmt_desc_get(format.into());
        if desc.is_null() {
            return false;
        }
        ((*desc).flags & ff::ffi::AV_PIX_FMT_FLAG_ALPHA as u64) != 0
    }
}

/// A single-source video decoder: owns one format context + codec context and
/// decodes any requested source frame on demand. **Not** `Send` across threads
/// concurrently — the [`DecoderPool`] gives each URL its own decoder; wrap in a
/// thread/`Mutex` for cross-thread use (the transport runs one decode owner per
/// URL).
pub struct Decoder {
    url: PathBuf,
    ictx: ff::format::context::Input,
    stream_index: usize,
    decoder: ff::decoder::Video,
    /// Stream time base (for frame↔pts conversion).
    time_base: ff::Rational,
    /// Source frames per second.
    fps: f64,
    hw_status: HwDecodeStatus,
    /// `_hw_device` keeps the HW device context alive for the codec's lifetime.
    _hw_device: Option<HwDevice>,
}

/// RAII wrapper over an `AVBufferRef` HW device context (freed on drop).
struct HwDevice {
    ptr: *mut ff::ffi::AVBufferRef,
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: `ptr` was produced by `av_hwdevice_ctx_create` and is non-null
        // here; `av_buffer_unref` nulls the pointer and is the matching free.
        unsafe {
            if !self.ptr.is_null() {
                ff::ffi::av_buffer_unref(&mut self.ptr);
            }
        }
    }
}

impl Decoder {
    /// Open `url`, attempting a platform HW device and falling back to CPU.
    pub fn open(url: impl AsRef<Path>) -> Result<Self, DecodeError> {
        ensure_ffmpeg_init()?;
        let url = url.as_ref().to_path_buf();
        let ictx = ff::format::input(&url)?;

        let stream = ictx
            .streams()
            .best(ff::media::Type::Video)
            .ok_or(DecodeError::NoVideoStream)?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let fps = rate_to_f64(stream.avg_frame_rate())
            .or_else(|| rate_to_f64(stream.rate()))
            .filter(|f| *f > 0.0)
            .unwrap_or(30.0);

        let parameters = stream.parameters();

        // Attempt a HW device, attaching it to the codec context before opening.
        // Best-effort: any failure leaves us on the CPU path. We pass parameters
        // (not a built context) so the CPU fallback can rebuild cleanly after a
        // failed HW attach (`.decoder()` consumes the context).
        let (decoder, hw_status, hw_device) = open_video_decoder(&parameters)?;

        Ok(Decoder {
            url,
            ictx,
            stream_index,
            decoder,
            time_base,
            fps,
            hw_status,
            _hw_device: hw_device,
        })
    }

    /// The source URL this decoder owns.
    pub fn url(&self) -> &Path {
        &self.url
    }

    /// Source frames per second.
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Which decode path engaged (HW kind or CPU).
    pub fn hw_status(&self) -> HwDecodeStatus {
        self.hw_status
    }

    /// Decode the frame at `source_frame`, returning CPU planes. Seeks to the
    /// nearest keyframe at/under the target then decodes forward to the exact
    /// frame. The returned [`DecodedFrame`] carries `source_frame` so the cache
    /// can key it.
    pub fn decode_frame(&mut self, source_frame: u64) -> Result<DecodedFrame, DecodeError> {
        // Convert the frame index to a stream timestamp: t = frame / fps, then
        // express it in the stream's time base (round ties-away).
        let target_ts = self.frame_to_stream_ts(source_frame);

        // Seek to slightly before the target so the decoded GOP starts at/under
        // it; the open range end `..target_ts` lands on the first keyframe ≤
        // target (matching the thumbnail path's backward-seek semantics).
        let _ = self.ictx.seek(target_ts, ..target_ts);
        self.decoder.flush();

        // Copy the conversion params so the inner loop borrows neither `self`
        // nor `self.decoder` immutably (the decoder is borrowed mutably below).
        let time_base = self.time_base;
        let fps = self.fps;
        let stream_index = self.stream_index;

        let mut decoded = ff::frame::Video::empty();
        let mut best: Option<ff::frame::Video> = None;

        // Decode forward, tracking the frame whose index is closest at/under the
        // target; stop once we pass the target. Returns Ok(true) when the answer
        // is settled (exact hit or we passed the target).
        let mut process =
            |decoder: &mut ff::decoder::Video, best: &mut Option<ff::frame::Video>| -> bool {
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let idx = pts_to_frame_with(best_pts(&decoded), time_base, fps);
                    if idx <= source_frame {
                        *best = Some(decoded.clone());
                        if idx == source_frame {
                            return true; // exact hit
                        }
                    } else {
                        // Passed the target; the previous `best` is the answer.
                        return true;
                    }
                }
                false
            };

        let mut done = false;
        // Borrow split: collect packets we need, then feed the decoder.
        let packets: Vec<ff::codec::packet::Packet> = self
            .ictx
            .packets()
            .filter_map(|(s, p)| (s.index() == stream_index).then_some(p))
            .collect();
        for packet in &packets {
            self.decoder.send_packet(packet)?;
            if process(&mut self.decoder, &mut best) {
                done = true;
                break;
            }
        }
        if !done {
            self.decoder.send_eof()?;
            let _ = process(&mut self.decoder, &mut best);
        }

        let frame = best.ok_or(DecodeError::FrameOutOfRange)?;
        self.frame_to_decoded(frame, source_frame)
    }

    /// Convert a source frame index to a stream timestamp (in `time_base`).
    fn frame_to_stream_ts(&self, source_frame: u64) -> i64 {
        let seconds = source_frame as f64 / self.fps;
        let tb = self.time_base;
        // ts = seconds * (den / num)  (time_base = num/den seconds-per-unit)
        let ts = seconds * tb.denominator() as f64 / tb.numerator() as f64;
        ts.round() as i64
    }

    /// Convert a decoded ffmpeg frame into our [`DecodedFrame`] CPU-plane form.
    /// Native planar YUV is copied plane-by-plane; anything else is scaled to
    /// RGBA8 (the universal fallback). HW surfaces are transferred to a CPU
    /// frame first.
    fn frame_to_decoded(
        &self,
        frame: ff::frame::Video,
        source_frame: u64,
    ) -> Result<DecodedFrame, DecodeError> {
        // If the frame lives on a HW surface, pull it down to system memory.
        let frame = transfer_if_hw(frame)?;

        let format = frame.format();
        let width = frame.width();
        let height = frame.height();

        if let Some((layout, _)) = layout_for(format).filter(|(l, _)| l.is_planar_yuv()) {
            let planes = copy_planar_yuv(&frame, layout);
            let has_alpha = pixfmt_has_alpha(format);
            return Ok(DecodedFrame {
                layout,
                width,
                height,
                has_alpha,
                planes: Arc::new(planes),
                source_frame,
            });
        }

        // Fallback: scale whatever we got to RGBA8.
        let has_alpha = pixfmt_has_alpha(format);
        let mut scaler = ff::software::scaling::Context::get(
            format,
            width,
            height,
            ff::format::Pixel::RGBA,
            width,
            height,
            ff::software::scaling::Flags::BILINEAR,
        )?;
        let mut rgba = ff::frame::Video::empty();
        scaler.run(&frame, &mut rgba)?;
        let plane = copy_interleaved(&rgba, 4);
        Ok(DecodedFrame {
            layout: PixelLayout::Rgba8,
            width,
            height,
            has_alpha,
            planes: Arc::new(vec![plane]),
            source_frame,
        })
    }
}

/// Best-effort PTS for a decoded frame (`pts` then `best_effort_timestamp`).
fn best_pts(frame: &ff::frame::Video) -> i64 {
    frame.pts().or_else(|| frame.timestamp()).unwrap_or(0)
}

/// Convert a stream pts (in `time_base`) to a source frame index at `fps`,
/// rounding ties away from zero. Free function so the decode loop can call it
/// without borrowing `self` (the decoder is borrowed mutably there).
fn pts_to_frame_with(pts: i64, time_base: ff::Rational, fps: f64) -> u64 {
    let seconds = pts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
    let idx = (seconds * fps).round();
    if idx < 0.0 { 0 } else { idx as u64 }
}

/// If `frame` is on a HW surface (`format` is a HW pixel format), transfer it to
/// a software frame; otherwise return it unchanged.
fn transfer_if_hw(frame: ff::frame::Video) -> Result<ff::frame::Video, DecodeError> {
    // A HW frame has a non-null `hw_frames_ctx`. Use the FFI transfer to pull it
    // down to a software frame the planes-copy can read.
    // SAFETY: `frame.as_ptr()` is a valid AVFrame for the call's duration.
    let is_hw = unsafe { !(*frame.as_ptr()).hw_frames_ctx.is_null() };
    if !is_hw {
        return Ok(frame);
    }
    let mut sw = ff::frame::Video::empty();
    // SAFETY: both frames are valid AVFrames; av_hwframe_transfer_data fills `sw`
    // from the HW surface in `frame`.
    let ret = unsafe {
        ff::ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), frame.as_ptr(), 0)
    };
    if ret < 0 {
        return Err(DecodeError::Ffmpeg(format!(
            "av_hwframe_transfer_data failed ({ret})"
        )));
    }
    // Carry timestamps across the transfer.
    sw.set_pts(frame.pts());
    Ok(sw)
}

/// Copy the three planar-YUV planes (each with its own sub-sampled dims/stride)
/// into tightly-strided [`Plane`]s.
fn copy_planar_yuv(frame: &ff::frame::Video, layout: PixelLayout) -> Vec<Plane> {
    let (cw_div, ch_div) = match layout {
        PixelLayout::Yuv420P => (2u32, 2u32),
        PixelLayout::Yuv422P => (2, 1),
        PixelLayout::Yuv444P => (1, 1),
        PixelLayout::Rgba8 => (1, 1),
    };
    let w = frame.width();
    let h = frame.height();
    let dims = [
        (w, h),                                  // Y
        (w.div_ceil(cw_div), h.div_ceil(ch_div)), // U
        (w.div_ceil(cw_div), h.div_ceil(ch_div)), // V
    ];
    let mut planes = Vec::with_capacity(3);
    for (i, (pw, ph)) in dims.into_iter().enumerate() {
        let stride = frame.stride(i);
        let data = frame.data(i);
        let row = pw as usize;
        let mut bytes = Vec::with_capacity(row * ph as usize);
        for y in 0..ph as usize {
            let start = y * stride;
            let end = (start + row).min(data.len());
            if start >= data.len() {
                break;
            }
            bytes.extend_from_slice(&data[start..end]);
        }
        planes.push(Plane {
            bytes,
            stride: row,
            width: pw,
            height: ph,
        });
    }
    planes
}

/// Copy an interleaved frame (e.g. RGBA, `bpp` bytes/pixel) into one tightly-
/// packed [`Plane`].
fn copy_interleaved(frame: &ff::frame::Video, bpp: usize) -> Plane {
    let w = frame.width();
    let h = frame.height();
    let stride = frame.stride(0);
    let data = frame.data(0);
    let row = w as usize * bpp;
    let mut bytes = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        let start = y * stride;
        let end = (start + row).min(data.len());
        if start >= data.len() {
            break;
        }
        bytes.extend_from_slice(&data[start..end]);
    }
    Plane {
        bytes,
        stride: row,
        width: w,
        height: h,
    }
}

/// Rational → f64, guarding the zero denominator ffmpeg uses for "unknown".
fn rate_to_f64(r: ff::Rational) -> Option<f64> {
    let (n, d) = (r.numerator(), r.denominator());
    if d == 0 || n == 0 {
        None
    } else {
        Some(n as f64 / d as f64)
    }
}

/// Open a video decoder from stream parameters, attempting a platform HW device
/// first and falling back to CPU. Returns the decoder, the engaged status, and
/// the HW device (kept alive alongside the codec).
///
/// Taking `parameters` (rather than a built context) lets us rebuild a fresh
/// context for the CPU fallback after a failed HW attach — `.decoder()` consumes
/// the context, so a failed HW open can't hand its context back.
fn open_video_decoder(
    parameters: &ff::codec::Parameters,
) -> Result<(ff::decoder::Video, HwDecodeStatus, Option<HwDevice>), DecodeError> {
    for &kind in HwKind::platform_order() {
        let Some(dev) = create_hw_device(kind) else {
            continue;
        };
        // Build a fresh context for the HW attempt.
        let ctx = ff::codec::context::Context::from_parameters(parameters.clone())?;
        // SAFETY: `ctx.as_ptr()` is a valid, unopened AVCodecContext. `av_buffer_ref`
        // adds a reference the codec owns; our `HwDevice` keeps the original alive.
        // Setting `hw_device_ctx` before opening enables HW decode for codecs that
        // support the device.
        unsafe {
            let raw = ctx.as_ptr() as *mut ff::ffi::AVCodecContext;
            if !raw.is_null() && !dev.ptr.is_null() {
                (*raw).hw_device_ctx = ff::ffi::av_buffer_ref(dev.ptr);
            }
        }
        if let Ok(decoder) = ctx.decoder().video() {
            return Ok((decoder, HwDecodeStatus::Hardware(kind), Some(dev)));
        }
        // HW open failed (codec doesn't support this device, etc.) — drop the
        // device and fall through to the CPU path with a fresh context.
    }

    // CPU path: a fresh context from the same parameters.
    let ctx = ff::codec::context::Context::from_parameters(parameters.clone())?;
    let decoder = ctx.decoder().video()?;
    Ok((decoder, HwDecodeStatus::Cpu, None))
}

/// Create a HW device context of `kind`, or `None` if it doesn't initialize on
/// this machine (no driver, no GPU, unsupported type).
fn create_hw_device(kind: HwKind) -> Option<HwDevice> {
    let mut ptr: *mut ff::ffi::AVBufferRef = std::ptr::null_mut();
    // SAFETY: out-param `ptr` is null-initialized; `av_hwdevice_ctx_create`
    // either fills it and returns 0, or leaves it null and returns < 0.
    let ret = unsafe {
        ff::ffi::av_hwdevice_ctx_create(
            &mut ptr,
            kind.av_type(),
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ret < 0 || ptr.is_null() {
        None
    } else {
        Some(HwDevice { ptr })
    }
}

// `Decoder` holds raw ffmpeg contexts; it is safe to *move* to another thread
// (the DecoderPool runs one per URL, accessed behind a Mutex), so mark it Send.
// It is NOT Sync — never share &Decoder across threads.
// SAFETY: ffmpeg decode/format contexts are not internally thread-shared here;
// we only ever access a Decoder from one thread at a time (Mutex-guarded).
unsafe impl Send for Decoder {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_tags_planar_and_rgba() {
        use ff::format::Pixel as P;
        assert_eq!(layout_for(P::YUV420P), Some((PixelLayout::Yuv420P, false)));
        assert_eq!(layout_for(P::YUV444P), Some((PixelLayout::Yuv444P, false)));
        assert_eq!(layout_for(P::RGBA), Some((PixelLayout::Rgba8, true)));
        assert!(layout_for(P::GBRP).is_none());
    }

    #[test]
    fn rgba_pixfmt_reports_alpha() {
        // RGBA advertises an alpha channel; YUV420P does not.
        assert!(pixfmt_has_alpha(ff::format::Pixel::RGBA));
        assert!(!pixfmt_has_alpha(ff::format::Pixel::YUV420P));
    }

    #[test]
    fn rate_to_f64_guards_unknown() {
        assert_eq!(rate_to_f64(ff::Rational::new(30000, 1001)), Some(30000.0 / 1001.0));
        assert_eq!(rate_to_f64(ff::Rational::new(0, 0)), None);
        assert_eq!(rate_to_f64(ff::Rational::new(25, 0)), None);
    }

    #[test]
    fn platform_order_is_nonempty() {
        assert!(!HwKind::platform_order().is_empty());
    }

    // --- Real-video decode (ignored: needs a committed-or-env fixture) ---
    //
    // No media is committed. Point PALMIER_TEST_VIDEO at a real file to exercise
    // the full open → seek → decode → CPU-plane path and the HW/CPU selection:
    //   PALMIER_TEST_VIDEO=C:\clip.mp4 cargo test -p palmier-media -- --ignored
    #[test]
    #[ignore = "needs a real video fixture via PALMIER_TEST_VIDEO"]
    fn decode_first_frame_of_real_video() {
        let Ok(path) = std::env::var("PALMIER_TEST_VIDEO") else {
            return;
        };
        let mut dec = Decoder::open(&path).expect("open video");
        assert!(dec.fps() > 0.0);
        let frame = dec.decode_frame(0).expect("decode frame 0");
        assert!(frame.width > 0 && frame.height > 0);
        assert!(!frame.planes.is_empty());
        eprintln!(
            "decoded {}x{} layout={:?} hw={:?}",
            frame.width,
            frame.height,
            frame.layout,
            dec.hw_status()
        );
    }
}
