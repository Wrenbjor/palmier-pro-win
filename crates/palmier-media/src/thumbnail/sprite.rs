//! Sprite-sheet **layout math** + the `.thumbs.json` sidecar shape (story E4-S3).
//!
//! Pure, decoder-free geometry so it unit-tests without a real video. Port of the
//! grid math in `MediaVisualCache.saveThumbnails` / `loadThumbnails`
//! (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`):
//!
//! * a strip of `n` tiles is laid out into a grid of `columns = min(50, n)` and
//!   `rows = ceil(n / columns)` — the **≤ 50 columns** rule (ruling #16);
//! * tile `i` sits at grid cell `(col = i % columns, row = i / columns)`, pixel
//!   origin `(col * tileWidth, row * tileHeight)`. The reference uses a
//!   bottom-left CoreGraphics origin and flips the row; we keep a **top-left**
//!   origin (the `image` crate's coordinate space), so row 0 is the top — the
//!   stored bytes differ from macOS but the sidecar `times[]`↔cell mapping is
//!   identical, which is the parity contract the loader round-trips on.
//!
//! The sidecar (`<key>.thumbs.json`) is `tileWidth, tileHeight, columns, times[]`
//! and is **written last** = the completion marker (an entry with a sprite but no
//! sidecar is treated as incomplete). See `docs/reference/media-panel.md`
//! §"Video thumbnail strip".

use serde::{Deserialize, Serialize};

/// The reference's hard cap on sprite-sheet columns (`min(50, thumbs.count)`),
/// ruling #16.
pub const MAX_SPRITE_COLUMNS: usize = 50;

/// JSON sidecar written next to the sprite JPEG (`<key>.thumbs.json`). Field
/// names match the Swift `ThumbnailCacheMeta` (`tileWidth`, `tileHeight`,
/// `columns`, `times`) so the on-disk shape is identical across platforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThumbnailSidecar {
    /// Width of one tile in pixels.
    #[serde(rename = "tileWidth")]
    pub tile_width: u32,
    /// Height of one tile in pixels.
    #[serde(rename = "tileHeight")]
    pub tile_height: u32,
    /// Number of grid columns (`min(50, times.len())`).
    pub columns: u32,
    /// Source-seconds timestamp of each tile, in tile order (row-major).
    pub times: Vec<f64>,
}

impl ThumbnailSidecar {
    /// Rows implied by the tile count and column count: `ceil(times / columns)`.
    /// Mirrors `(thumbs.count + columns - 1) / columns`.
    pub fn rows(&self) -> u32 {
        sprite_rows(self.times.len(), self.columns as usize) as u32
    }

    /// Validate the sidecar describes a non-empty, self-consistent grid (the
    /// `meta.tileWidth > 0 && … && !meta.times.isEmpty` guard in `loadThumbnails`).
    pub fn is_valid(&self) -> bool {
        self.tile_width > 0
            && self.tile_height > 0
            && self.columns > 0
            && !self.times.is_empty()
    }
}

/// Grid geometry derived from a tile count: `(columns, rows)`.
///
/// `columns = min(50, n)`, `rows = ceil(n / columns)` — the exact reference math.
/// `n == 0` yields `(0, 0)` (no grid; caller writes nothing).
pub fn sprite_grid(n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let columns = n.min(MAX_SPRITE_COLUMNS);
    let rows = sprite_rows(n, columns);
    (columns, rows)
}

/// `ceil(n / columns)` with a zero-column guard (returns 0).
pub fn sprite_rows(n: usize, columns: usize) -> usize {
    if columns == 0 {
        return 0;
    }
    n.div_ceil(columns)
}

/// Pixel origin `(x, y)` of tile `i` in a top-left grid of `columns` columns and
/// `tile_width × tile_height` cells. `(col, row) = (i % columns, i / columns)`.
pub fn tile_origin(i: usize, columns: usize, tile_width: u32, tile_height: u32) -> (u32, u32) {
    debug_assert!(columns > 0, "tile_origin needs at least one column");
    let col = (i % columns) as u32;
    let row = (i / columns) as u32;
    (col * tile_width, row * tile_height)
}

/// Total sprite-sheet pixel dimensions for `n` tiles of `tile_width × tile_height`.
/// `(columns * tile_width, rows * tile_height)`; `(0, 0)` for an empty strip.
pub fn sprite_dimensions(n: usize, tile_width: u32, tile_height: u32) -> (u32, u32) {
    let (columns, rows) = sprite_grid(n);
    (columns as u32 * tile_width, rows as u32 * tile_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_caps_columns_at_50() {
        // Under the cap: columns == n, one row.
        assert_eq!(sprite_grid(1), (1, 1));
        assert_eq!(sprite_grid(10), (10, 1));
        assert_eq!(sprite_grid(50), (50, 1));
        // Over the cap: columns pinned at 50, rows wrap.
        assert_eq!(sprite_grid(51), (50, 2));
        assert_eq!(sprite_grid(100), (50, 2));
        assert_eq!(sprite_grid(101), (50, 3));
        // Empty strip → no grid.
        assert_eq!(sprite_grid(0), (0, 0));
    }

    #[test]
    fn rows_is_ceil_div() {
        assert_eq!(sprite_rows(0, 50), 0);
        assert_eq!(sprite_rows(1, 50), 1);
        assert_eq!(sprite_rows(50, 50), 1);
        assert_eq!(sprite_rows(51, 50), 2);
        assert_eq!(sprite_rows(100, 50), 2);
        assert_eq!(sprite_rows(101, 50), 3);
        // Zero-column guard.
        assert_eq!(sprite_rows(5, 0), 0);
    }

    #[test]
    fn tile_origin_is_row_major() {
        // 50-column grid, 120×68 tiles.
        assert_eq!(tile_origin(0, 50, 120, 68), (0, 0));
        assert_eq!(tile_origin(1, 50, 120, 68), (120, 0));
        assert_eq!(tile_origin(49, 50, 120, 68), (49 * 120, 0));
        // Tile 50 wraps to row 1, col 0.
        assert_eq!(tile_origin(50, 50, 120, 68), (0, 68));
        assert_eq!(tile_origin(51, 50, 120, 68), (120, 68));
    }

    #[test]
    fn sprite_dimensions_match_grid() {
        // 51 tiles → 50 cols × 2 rows of 120×68.
        assert_eq!(sprite_dimensions(51, 120, 68), (50 * 120, 2 * 68));
        // 10 tiles → 10 cols × 1 row.
        assert_eq!(sprite_dimensions(10, 120, 68), (10 * 120, 68));
        // Empty.
        assert_eq!(sprite_dimensions(0, 120, 68), (0, 0));
    }

    #[test]
    fn sidecar_round_trips_json_with_reference_field_names() {
        let meta = ThumbnailSidecar {
            tile_width: 120,
            tile_height: 68,
            columns: 50,
            times: vec![0.0, 1.0, 2.0],
        };
        let json = serde_json::to_string(&meta).unwrap();
        // Reference field names (camelCase) must be on the wire.
        assert!(json.contains("\"tileWidth\":120"), "got {json}");
        assert!(json.contains("\"tileHeight\":68"), "got {json}");
        assert!(json.contains("\"columns\":50"), "got {json}");
        assert!(json.contains("\"times\":[0.0,1.0,2.0]"), "got {json}");
        let back: ThumbnailSidecar = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
        assert_eq!(back.rows(), 1);
        assert!(back.is_valid());
    }

    #[test]
    fn sidecar_validity_guard() {
        let base = ThumbnailSidecar {
            tile_width: 120,
            tile_height: 68,
            columns: 50,
            times: vec![0.0],
        };
        assert!(base.is_valid());
        assert!(!ThumbnailSidecar { tile_width: 0, ..base.clone() }.is_valid());
        assert!(!ThumbnailSidecar { tile_height: 0, ..base.clone() }.is_valid());
        assert!(!ThumbnailSidecar { columns: 0, ..base.clone() }.is_valid());
        assert!(!ThumbnailSidecar { times: vec![], ..base.clone() }.is_valid());
    }
}
