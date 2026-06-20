//! Per-asset **metadata loader** (story E4-S1).
//!
//! Port of `MediaAsset.loadMetadata()` (`Sources/PalmierPro/Models/MediaAsset.swift`)
//! — the function that, after import, fills in an asset's `duration`, natural
//! `width`/`height` (rotation-corrected), `nominalFrameRate`, and audio-track
//! presence. The Swift original drives this off `AVURLAsset.load` (video/audio),
//! `ImageIO` (image), and the Lottie parser (lottie).
//!
//! ## Toolchain constraint (E4-S1): NO system FFmpeg
//! The Windows FFmpeg toolchain isn't provisioned yet — it lands with the
//! decode-heavy thumbnail/waveform stories (E4-S3/S4/S5). So this loader uses a
//! **lightweight, pure-Rust** path and *never* links `ffmpeg-next`:
//!
//! | type  | backend                | duration | w/h | fps | has_audio |
//! |-------|------------------------|:--------:|:---:|:---:|:---------:|
//! | image | `image` + `kamadak-exif`|   n/a*  | yes | n/a | n/a       |
//! | audio | `symphonia`            |   yes**  | n/a | n/a | yes(true) |
//! | video | `mp4` (ISO-BMFF)       |   yes    | yes |yes† | yes       |
//!
//! \* image duration is a fixed still-image default in the reference, set by the
//!    UI layer (`Defaults.imageDurationSeconds`), not probed here.
//! \** audio duration from the container/codec params when available.
//! † video fps is read from the `mp4` track when the container exposes a sane
//!    timescale/sample count; for `.mov`/`.m4v` variants where it can't be derived
//!    without a full decoder it is returned as `None` with a `// TODO(ffmpeg)`
//!    note — the E4-S3+ decode stories will backfill it via ffprobe/ffmpeg-next.
//!
//! Any field that *genuinely* needs a full decoder is `None` here, never faked.
//!
//! See `docs/reference/media-panel.md` §"macOS/Apple APIs to replace"
//! (AVURLAsset→ffprobe, ImageIO→`image`, DSWaveformImage→symphonia) and
//! `_bmad-output/implementation-artifacts/epic-04-media-panel.md` (E4-S1).

use std::path::Path;

use crate::clip::{clip_type_for_path, ClipType};

/// Metadata probed from a media file, mirroring the fields
/// `MediaAsset.loadMetadata()` populates (duration / sourceWidth / sourceHeight /
/// sourceFPS / hasAudio). This is intentionally a *subset* struct, not the full
/// `MediaAsset` — that richer model type lands in E2-S7; this loader only produces
/// the probed facts.
///
/// `width`/`height` are the **display** dimensions: the natural pixel size after
/// the container's display-matrix rotation is applied, so a portrait video that is
/// stored landscape + 90° rotation flag reports the *upright* W/H (matching the
/// Swift `naturalSize.applying(preferredTransform)` correction).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AssetMetadata {
    /// Duration in seconds. `None` when not applicable (images) or not derivable
    /// without a full decoder.
    pub duration: Option<f64>,
    /// Rotation-corrected natural width in pixels.
    pub width: Option<u32>,
    /// Rotation-corrected natural height in pixels.
    pub height: Option<u32>,
    /// Nominal frame rate (video only). `None` when the lightweight parser can't
    /// derive it — see `// TODO(ffmpeg)` in [`load_video_metadata`].
    pub fps: Option<f64>,
    /// Whether the asset carries an audio track. `true` for audio files; for video
    /// it reflects an actual audio track in the container.
    pub has_audio: bool,
}

/// Errors the metadata loader can surface. Callers treat these as "metadata
/// unavailable" — the asset still imports (matching the Swift `try?` shrug that
/// leaves fields at their defaults), it just renders without probed facts.
#[derive(Debug)]
pub enum MetadataError {
    /// The path's extension doesn't map to a supported [`ClipType`].
    UnsupportedType,
    /// Underlying I/O / parse failure (file missing, unreadable, malformed).
    Probe(String),
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetadataError::UnsupportedType => write!(f, "unsupported media type"),
            MetadataError::Probe(m) => write!(f, "metadata probe failed: {m}"),
        }
    }
}

impl std::error::Error for MetadataError {}

/// Load metadata for the file at `path`, classifying it by extension first.
///
/// Returns [`MetadataError::UnsupportedType`] if the extension isn't a known
/// media type. Otherwise dispatches to the per-type loader. The Lottie sniff is
/// *not* re-run here (classification/import gating is [`crate::clip::classify_path`]'s
/// job); a `.json`/`.lottie` is treated as lottie metadata.
pub fn load_metadata(path: impl AsRef<Path>) -> Result<AssetMetadata, MetadataError> {
    let path = path.as_ref();
    let clip_type = clip_type_for_path(path).ok_or(MetadataError::UnsupportedType)?;
    load_metadata_as(path, clip_type)
}

/// Load metadata treating `path` as the given [`ClipType`] (skips re-classification).
///
/// Useful when the import pipeline already classified the file (e.g. via
/// [`crate::clip::classify_path`]) and wants to avoid a second extension lookup.
pub fn load_metadata_as(
    path: impl AsRef<Path>,
    clip_type: ClipType,
) -> Result<AssetMetadata, MetadataError> {
    let path = path.as_ref();
    match clip_type {
        ClipType::Image => load_image_metadata(path),
        ClipType::Audio => load_audio_metadata(path),
        ClipType::Video => load_video_metadata(path),
        // Lottie metadata (first-frame dims + animation duration) needs the Lottie
        // renderer, which isn't ported yet (it arrives with the lottie thumbnail
        // pipeline). Return an empty struct rather than faking values.
        // TODO(lottie): parse `ip`/`op`/`fr`/`w`/`h` from the Lottie JSON for
        // duration + dimensions when the Lottie generator lands.
        ClipType::Lottie => Ok(AssetMetadata::default()),
        // Text clips have no source file metadata.
        ClipType::Text => Ok(AssetMetadata::default()),
    }
}

/// Image metadata: pixel dimensions via the `image` crate, corrected for EXIF
/// orientation (the reference relies on ImageIO's
/// `kCGImageSourceCreateThumbnailWithTransform`, which bakes orientation in). We
/// read the EXIF `Orientation` tag and swap W/H for the four "rotated 90°/270°"
/// orientations so a portrait photo stored landscape-with-rotation reports upright.
///
/// Image `duration` is left `None` — the reference sets a fixed still default in
/// the UI layer (`Defaults.imageDurationSeconds`), not in the probe.
fn load_image_metadata(path: &Path) -> Result<AssetMetadata, MetadataError> {
    let (mut width, mut height) = image::image_dimensions(path)
        .map_err(|e| MetadataError::Probe(format!("image dimensions: {e}")))?;

    if exif_orientation_swaps_axes(path) {
        std::mem::swap(&mut width, &mut height);
    }

    Ok(AssetMetadata {
        duration: None,
        width: Some(width),
        height: Some(height),
        fps: None,
        has_audio: false,
    })
}

/// Read the EXIF `Orientation` tag and report whether it rotates the image by
/// 90°/270° (orientations 5–8), which swaps the displayed width/height. Returns
/// `false` when there's no EXIF, no orientation tag, or an upright orientation.
fn exif_orientation_swaps_axes(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut reader = std::io::BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let Ok(exif) = exif_reader.read_from_container(&mut reader) else {
        return false;
    };
    let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY) else {
        return false;
    };
    // EXIF orientation values 5,6,7,8 are the 90°/270° rotations that swap axes.
    matches!(field.value.get_uint(0), Some(5..=8))
}

/// Audio metadata via `symphonia`: duration + the always-true `has_audio`. Probes
/// the format reader's default audio track and derives duration from
/// `n_frames / sample_rate` (or the codec's stated `time_base`) when present.
///
/// fps/width/height are `None` (not applicable to audio).
fn load_audio_metadata(path: &Path) -> Result<AssetMetadata, MetadataError> {
    use symphonia::core::codecs::CodecParameters;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file =
        std::fs::File::open(path).map_err(|e| MetadataError::Probe(format!("open: {e}")))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| MetadataError::Probe(format!("symphonia probe: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.tracks().first())
        .ok_or_else(|| MetadataError::Probe("no audio track".into()))?;

    // Sample rate for the `num_frames / sample_rate` fallback path.
    let sample_rate = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p.sample_rate,
        _ => None,
    };

    let duration = track_duration_seconds(
        track.num_frames,
        track.duration.map(|d| d.get()),
        track.time_base,
        sample_rate,
    );

    Ok(AssetMetadata {
        duration,
        width: None,
        height: None,
        fps: None,
        has_audio: true,
    })
}

/// Derive seconds from a track's timing fields. Symphonia 0.6 exposes both a
/// container-stated `duration` (in timebase units) and `num_frames`; we prefer
/// the timebase conversion (`duration` then `num_frames` via `time_base`), and
/// fall back to `num_frames / sample_rate`. Returns `None` if nothing usable is
/// present.
fn track_duration_seconds(
    num_frames: Option<u64>,
    duration_units: Option<u64>,
    time_base: Option<symphonia::core::units::TimeBase>,
    sample_rate: Option<u32>,
) -> Option<f64> {
    use symphonia::core::units::Timestamp;

    // Prefer the timebase conversion of the container's stated duration (else the
    // playable-frame count).
    if let (Some(tb), Some(units)) = (time_base, duration_units.or(num_frames))
        && let Ok(ts) = Timestamp::try_from(units)
        && let Some(time) = tb.calc_time(ts)
    {
        return Some(time.as_secs_f64());
    }
    // Fallback: frames / sample_rate.
    let frames = num_frames?;
    let sr = sample_rate?;
    if sr == 0 {
        return None;
    }
    Some(frames as f64 / sr as f64)
}

/// Video metadata via the pure-Rust `mp4` ISO-BMFF parser (covers `mp4`/`m4v`/`mov`,
/// which share the box structure). Reads:
/// - **duration** from the movie header;
/// - **display dimensions** with the track's display-matrix rotation applied, so
///   a portrait clip stored landscape + a 90° matrix reports upright W/H — the
///   parity-critical replacement for the Swift `naturalSize.applying(preferredTransform)`;
/// - **fps** from the video track when derivable (see TODO below);
/// - **has_audio** = an audio track exists in the container.
///
/// This pure-Rust path keeps the build free of any system-FFmpeg dependency for
/// E4-S1. Containers the `mp4` crate can't fully parse fall back to `None` fields.
fn load_video_metadata(path: &Path) -> Result<AssetMetadata, MetadataError> {
    let file =
        std::fs::File::open(path).map_err(|e| MetadataError::Probe(format!("open: {e}")))?;
    let size = file
        .metadata()
        .map_err(|e| MetadataError::Probe(format!("stat: {e}")))?
        .len();
    let reader = std::io::BufReader::new(file);

    let mp4 = mp4::Mp4Reader::read_header(reader, size)
        .map_err(|e| MetadataError::Probe(format!("mp4 parse: {e}")))?;

    let duration = {
        let secs = mp4.duration().as_secs_f64();
        if secs > 0.0 {
            Some(secs)
        } else {
            None
        }
    };

    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut fps: Option<f64> = None;
    let mut has_audio = false;

    for track in mp4.tracks().values() {
        match track.track_type() {
            Ok(mp4::TrackType::Video) => {
                let w = track.width() as u32;
                let h = track.height() as u32;
                // Apply the display-matrix rotation: tkhd.matrix encodes the
                // presentation rotation (the AVFoundation `preferredTransform`
                // equivalent). For a 90°/270° rotation the displayed W/H swap.
                let (dw, dh) = apply_display_rotation(w, h, track_rotation_degrees(track));
                width = Some(dw);
                height = Some(dh);

                // fps: the `mp4` crate exposes `frame_rate()` derived from
                // sample count / duration. It's reliable for constant-frame-rate
                // mp4; for some `.mov`/`.m4v` variants (variable frame rate, edit
                // lists) it can be 0/garbage and a precise value needs a full
                // decoder.
                // TODO(ffmpeg): replace with ffprobe `r_frame_rate` /
                // `avg_frame_rate` in the E4-S3+ decode stories for exact VFR fps.
                let r = track.frame_rate();
                if r.is_finite() && r > 0.0 {
                    fps = Some(r);
                }
            }
            Ok(mp4::TrackType::Audio) => has_audio = true,
            _ => {}
        }
    }

    Ok(AssetMetadata {
        duration,
        width,
        height,
        fps,
        has_audio,
    })
}

/// Rotation (in degrees, normalized to {0,90,180,270}) encoded by an mp4 track's
/// display matrix (`tkhd.matrix`). Mirrors the standard interpretation of the
/// 3×3 fixed-point matrix: the (a,b,c,d) sub-matrix is a rotation, read off as the
/// angle of the first basis vector. Defaults to 0 if the matrix is identity or
/// unrecognized.
fn track_rotation_degrees(track: &mp4::Mp4Track) -> u32 {
    // `mp4::Mp4Track` exposes the track header's matrix. The matrix values are
    // 16.16 / 2.30 fixed-point; we only need the sign/zero pattern of the (a,b)
    // top row to distinguish the four cardinal rotations.
    let m = &track.trak.tkhd.matrix;
    // a = m.a (16.16), b = m.b (16.16). atan2(b, a) gives the rotation.
    let a = m.a as f64 / 65536.0;
    let b = m.b as f64 / 65536.0;
    let angle = b.atan2(a).to_degrees();
    // Normalize to nearest 90° in [0,360).
    let normalized = ((angle.round() as i64 % 360) + 360) % 360;
    match normalized {
        90 => 90,
        180 => 180,
        270 => 270,
        _ => 0,
    }
}

/// Swap (w,h) for 90°/270° rotations; pass through for 0°/180°.
fn apply_display_rotation(w: u32, h: u32, rotation_degrees: u32) -> (u32, u32) {
    match rotation_degrees {
        90 | 270 => (h, w),
        _ => (w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_swaps_axes_for_portrait() {
        // Landscape-stored frame (1920×1080) with a 90° display rotation should
        // report upright portrait dimensions (1080×1920).
        assert_eq!(apply_display_rotation(1920, 1080, 90), (1080, 1920));
        assert_eq!(apply_display_rotation(1920, 1080, 270), (1080, 1920));
        // 0°/180° pass through unchanged.
        assert_eq!(apply_display_rotation(1920, 1080, 0), (1920, 1080));
        assert_eq!(apply_display_rotation(1920, 1080, 180), (1920, 1080));
    }

    #[test]
    fn track_duration_prefers_time_base_then_sample_rate() {
        use std::num::NonZero;
        use symphonia::core::units::TimeBase;
        let tb = TimeBase::new(NonZero::new(1).unwrap(), NonZero::new(44_100).unwrap());
        // time_base path, using the container's stated `duration` (44100 units @
        // 1/44100) ⇒ 1.0s.
        assert_eq!(
            track_duration_seconds(None, Some(44_100), Some(tb), Some(44_100)),
            Some(1.0)
        );
        // time_base path falls back to `num_frames` when `duration` is absent:
        // 88200 frames @ 1/44100 ⇒ 2.0s.
        assert_eq!(
            track_duration_seconds(Some(88_200), None, Some(tb), Some(44_100)),
            Some(2.0)
        );
        // No time_base ⇒ frames / sample_rate fallback: 88200 @ 44100 ⇒ 2.0s.
        assert_eq!(
            track_duration_seconds(Some(88_200), None, None, Some(44_100)),
            Some(2.0)
        );
        // No frame count and no time_base ⇒ None.
        assert_eq!(track_duration_seconds(None, None, None, Some(44_100)), None);
        // Zero sample rate guarded (fallback path).
        assert_eq!(track_duration_seconds(Some(100), None, None, Some(0)), None);
    }

    #[test]
    fn unsupported_path_errors() {
        let err = load_metadata("C:/x/file.zip").unwrap_err();
        assert!(matches!(err, MetadataError::UnsupportedType));
    }

    #[test]
    fn lottie_and_text_return_empty_metadata() {
        // No file read for these — empty struct, fields None, has_audio false.
        let lottie = load_metadata_as("C:/x/anim.lottie", ClipType::Lottie).unwrap();
        assert_eq!(lottie, AssetMetadata::default());
        let text = load_metadata_as("C:/x/whatever", ClipType::Text).unwrap();
        assert_eq!(text, AssetMetadata::default());
    }

    #[test]
    fn image_metadata_reads_dimensions() {
        use std::io::Write;
        // Build a valid 2×3 PNG (no EXIF) via the `image` crate so the fixture is
        // guaranteed decodable, then confirm dimensions read with no orientation swap.
        let mut tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        let buf = make_png(2, 3);
        tmp.write_all(&buf).unwrap();
        tmp.flush().unwrap();

        let meta = load_metadata(tmp.path()).unwrap();
        assert_eq!(meta.width, Some(2));
        assert_eq!(meta.height, Some(3));
        assert_eq!(meta.duration, None);
        assert!(!meta.has_audio);
    }

    #[test]
    fn image_metadata_swaps_dims_for_exif_orientation_6() {
        use std::io::Write;
        // A landscape (4×2) JPEG tagged EXIF Orientation=6 (rotate 90° CW) must
        // report upright portrait dimensions (2×4) — the EXIF-aware correction the
        // reference gets from ImageIO's transform flag.
        let jpeg = make_jpeg_with_orientation(4, 2, 6);
        let mut tmp = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        tmp.write_all(&jpeg).unwrap();
        tmp.flush().unwrap();

        // Sanity: the raw stored frame is 4×2…
        let (raw_w, raw_h) = image::image_dimensions(tmp.path()).unwrap();
        assert_eq!((raw_w, raw_h), (4, 2), "stored frame is landscape 4×2");
        // …and the orientation tag is detected as an axis-swapping rotation.
        assert!(exif_orientation_swaps_axes(tmp.path()), "orientation 6 swaps axes");

        // The loader applies the swap → upright 2×4.
        let meta = load_metadata(tmp.path()).unwrap();
        assert_eq!(meta.width, Some(2));
        assert_eq!(meta.height, Some(4));
    }

    /// Encode a solid-color RGBA PNG of the given size via the `image` crate, so
    /// the fixture is guaranteed valid for `image::image_dimensions`.
    fn make_png(w: u32, h: u32) -> Vec<u8> {
        use image::{ImageBuffer, ImageFormat, Rgba};
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(w, h, Rgba([10, 20, 30, 255]));
        let mut bytes: Vec<u8> = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// Encode a JPEG of the given pixel size and splice in a minimal EXIF APP1
    /// segment declaring the given `Orientation`. Built by hand so the test needs
    /// no pre-baked binary fixture and stays in-tree.
    fn make_jpeg_with_orientation(w: u32, h: u32, orientation: u16) -> Vec<u8> {
        use image::{ImageBuffer, ImageFormat, Rgb};
        // Base JPEG (RGB — JPEG has no alpha).
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(w, h, Rgb([120, 60, 200]));
        let mut base: Vec<u8> = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut base), ImageFormat::Jpeg)
            .unwrap();

        let app1 = build_exif_app1(orientation);
        // Insert APP1 right after SOI (the first two bytes, 0xFFD8).
        let mut out = Vec::with_capacity(base.len() + app1.len());
        out.extend_from_slice(&base[..2]); // SOI
        out.extend_from_slice(&app1);
        out.extend_from_slice(&base[2..]);
        out
    }

    /// Build an APP1 (EXIF) JPEG segment containing a single IFD0 `Orientation`
    /// tag (0x0112) with the given value, big-endian (`MM`) byte order.
    fn build_exif_app1(orientation: u16) -> Vec<u8> {
        // TIFF header (big-endian) + IFD0 with one entry.
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"MM"); // big-endian
        tiff.extend_from_slice(&0x002A_u16.to_be_bytes()); // magic 42
        tiff.extend_from_slice(&0x0000_0008_u32.to_be_bytes()); // IFD0 offset = 8

        // IFD0: 1 entry.
        tiff.extend_from_slice(&0x0001_u16.to_be_bytes()); // entry count
        tiff.extend_from_slice(&0x0112_u16.to_be_bytes()); // tag = Orientation
        tiff.extend_from_slice(&0x0003_u16.to_be_bytes()); // type = SHORT
        tiff.extend_from_slice(&0x0000_0001_u32.to_be_bytes()); // count = 1
        // value: a SHORT is left-justified in the 4-byte value field.
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0x00, 0x00]); // pad to 4 bytes
        tiff.extend_from_slice(&0x0000_0000_u32.to_be_bytes()); // next-IFD offset = 0

        // EXIF identifier prefix.
        let mut payload: Vec<u8> = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(&tiff);

        // APP1 marker + length (length covers the 2 length bytes + payload).
        let mut app1: Vec<u8> = Vec::new();
        app1.extend_from_slice(&[0xFF, 0xE1]); // APP1
        let len = (payload.len() + 2) as u16;
        app1.extend_from_slice(&len.to_be_bytes());
        app1.extend_from_slice(&payload);
        app1
    }
}
