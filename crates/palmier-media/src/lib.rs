//! # palmier-media
//!
//! FFmpeg decode/encode, thumbnail extraction, and audio waveform generation
//! (FOUNDATION §4, §6.2). Wraps `ffmpeg-next` (decode/encode) and `symphonia`
//! (audio decode); those heavy deps are added per-story, not in this skeleton.
//!
//! The [`cache`] module (story E4-S2) provides the media visual-cache
//! infrastructure — SHA256 cache key, cache-directory resolution, and the
//! concurrency gates + in-flight dedup — that the thumbnail/waveform pipelines
//! (E4-S3/S4/S5) build on. It deliberately pulls no media-decode dependency.
//!
//! The [`clip`] and [`metadata`] modules (story E4-S1) provide the import-time
//! classification + probing layer:
//! * [`clip`] — the case-insensitive ClipType **extension gate** and the Lottie
//!   content **second-gate** sniff (`classify_path`).
//! * [`metadata`] — the asset metadata loader (`load_metadata`) returning
//!   duration / width / height / fps / has_audio. The pure-Rust `mp4` parser is
//!   the fast path; the E4-S1 `// TODO(ffmpeg)` fps fields are now **backfilled**
//!   via `ffmpeg-next` (avg/r_frame_rate) for VFR/edit-list containers
//!   ([`metadata::ffmpeg_frame_rate`]).
//!
//! The [`thumbnail`] (E4-S3 + E4-S5) and [`waveform`] (E4-S4) modules provide the
//! visual-cache decode pipelines that build on the [`cache`] infrastructure:
//! * [`thumbnail::video`] — `ffmpeg-next` video **sprite-sheet** strips
//!   (120×68, ≤ 50-col JPEG sprite + `.thumbs.json` sidecar; ungated, #16) and
//!   the single-frame `extract_frame` for Epic 11's moment thumbnails.
//! * [`thumbnail::image_thumb`] — EXIF-aware **image** thumbnails (gated 4, #16).
//! * [`waveform`] — `symphonia` **waveform** decode → 150 samples/s cap 20000
//!   `Vec<f32>` (gated 2, #16).
//!
//! The [`decode`] module (story E5-S2) is the **preview decode pipeline** — the
//! root of the preview/playback frame source. It owns the FFmpeg
//! [`decode::Decoder`] (one `AVFormatContext` per source URL, HW decode when
//! available), the [`decode::FrameCache`] (LRU keyed by `(media_ref,
//! source_frame)`, evicting by distance from the playhead under the 512 MB RAM
//! ceiling), the [`decode::SeekMode`] tolerance/throttle math, and the
//! engine-facing [`decode::FrameSource`] handle (`request_frame` /
//! `prefetch` / `cache_stats`). `palmier-engine` consumes decoded **CPU-side**
//! frames through that handle and never opens FFmpeg itself.

pub mod audio_decode;
pub mod cache;
pub mod clip;
pub mod decode;
pub mod import;
pub mod metadata;
pub mod thumbnail;
pub mod waveform;

pub use audio_decode::{
    decode_audio_pcm, AudioDecodeError, AudioPcmCache, DecodedAudio, TARGET_CHANNELS,
    TARGET_SAMPLE_RATE_HZ,
};
pub use clip::{
    classify_path, clip_type_for_extension, clip_type_for_path, is_lottie, is_lottie_bytes,
    ClipType,
};
pub use decode::{
    CacheStats, DecodeError, DecodedFrame, Decoder, DecoderPool, FrameCache, FrameKey, FrameResult,
    FrameSource, HwDecodeStatus, HwKind, PixelLayout, Plane, ScrubThrottle, SeekMode, UrlResolver,
};
pub use import::{
    add_media_asset, asset_drag_string, asset_drag_string_with_segment, asset_id_from_drag_string,
    asset_segment_from_drag_string, drag_payload, folder_drag_string, folder_id_from_drag_string,
    import_finder_items, import_folder, ImportSummary, SourceResolver, ASSET_DRAG_SCHEME,
    FOLDER_DRAG_SCHEME,
};
pub use metadata::{load_metadata, load_metadata_as, AssetMetadata, MetadataError};
pub use thumbnail::{
    extract_frame, extract_frame_timed, make_image_thumbnail, make_image_thumbnail_sized,
    video_thumbnail_times, ImageThumbnailCache, ThumbnailFrame, ThumbnailSidecar, TimedFrame,
    VideoThumbnailCache,
};
pub use waveform::{
    downsample_rms, generate_waveform, waveform_sample_count, WaveformCache, WaveformError,
};

/// Placeholder for the media subsystem.
pub fn placeholder() -> &'static str {
    "palmier-media"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-media");
    }
}
