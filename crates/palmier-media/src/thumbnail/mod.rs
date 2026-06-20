//! Thumbnail pipelines (stories E4-S3 + E4-S5).
//!
//! * [`video`] — the video **sprite-sheet** strip via `ffmpeg-next`: seek + scale
//!   to 120×68, assemble ONE JPEG sprite-sheet (≤ 50 cols, q=0.75) + a
//!   `.thumbs.json` sidecar written last (E4-S3). Cached through the E4-S2 key
//!   (#16) under the **ungated** video-thumb gate. The single-frame
//!   [`video::extract_frame`] backs the Tauri `thumbnail(media_ref,
//!   source_seconds, max_size)` "moment" command (Epic 11 search panel).
//! * [`image_thumb`] — the EXIF-aware **image** thumbnail (`image` crate, max
//!   pixel 120), cached under the E4-S2 key behind the **4-wide** image-thumb gate
//!   (E4-S5).
//! * [`sprite`] / [`times`] — the pure, decoder-free math (sprite-sheet layout,
//!   sampling-time formula) split out for direct unit testing.
//!
//! See `docs/reference/media-panel.md` §"Video thumbnail strip" / §"Image
//! thumbnail" and `_bmad-output/implementation-artifacts/epic-04-media-panel.md`
//! (E4-S3, E4-S5).

pub mod image_thumb;
pub mod sprite;
pub mod times;
pub mod video;

pub use image_thumb::{
    apply_exif_orientation, make_image_thumbnail, ImageThumbnailCache, ImageThumbnailError,
    IMAGE_THUMB_MAX_PIXEL,
};
pub use sprite::{
    sprite_dimensions, sprite_grid, sprite_rows, tile_origin, ThumbnailSidecar, MAX_SPRITE_COLUMNS,
};
pub use times::{video_thumbnail_times, THUMB_MAX_HEIGHT, THUMB_MAX_WIDTH};
pub use video::{
    extract_frame, extract_strip, ThumbnailFrame, VideoThumbnailCache, VideoThumbnailError,
    PROGRESSIVE_PUBLISH_EVERY, SPRITE_JPEG_QUALITY,
};
