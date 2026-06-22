// Lazy media-tile thumbnails (E4-S3/E4-S5 wiring).
//
// The Rust `thumbnail` command (crates/palmier-tauri/src/media.rs) is REAL: it
// resolves a file path, decodes a frame (video: seek to `sourceSeconds`; image:
// load + EXIF-correct), scales to `maxSize`, and returns a `data:image/jpeg;base64`
// URL (or `null` for audio/text/lottie/decode-failure). The `momentThumbnail`
// wrapper in `media-actions.ts` is the typed seam over it.
//
// This module owns the GRID-tile side of that pipeline:
//   - a process-wide cache (asset id → data-URL | null) so a frame is decoded once
//     and never re-fetched on re-render / scroll, mirroring the Rust-side cache;
//   - `useAssetThumbnail(asset)` — a hook that lazily fetches the thumbnail the
//     first time a tile mounts (video/image only), updates on resolution, and
//     returns the data-URL (or null while loading / for non-raster kinds).
//
// The decode key is the asset's on-disk PATH: the `thumbnail` command does
// `PathBuf::from(media_ref)`, so we pass `asset.path`. The `get_media` catalog
// projection currently omits `path` (it lives on the manifest entry — see
// adapt.ts), so real imported assets resolve no thumbnail until that path is
// threaded through; the tile falls back to its type glyph (an honest empty state,
// not a silent no-op). Fixture / external assets carry a real path and DO render a
// frame.

import { useEffect, useState } from "react";
import { momentThumbnail } from "./media-actions";
import type { MediaAssetView, MediaType } from "./types";

/** Box size requested from the decoder (matches the reference 240px moment box). */
export const TILE_THUMBNAIL_MAX = 240;

/** Source-time the still frame is sampled at — 1s in (skips black lead-in frames). */
export const TILE_THUMBNAIL_SECONDS = 1;

/** Kinds with a decodable raster source (audio/text/lottie keep the type glyph). */
function hasRasterThumbnail(type: MediaType): boolean {
  return type === "video" || type === "image";
}

/**
 * Process-wide thumbnail cache, keyed by `assetId@maxSize`. The value is the
 * data-URL, `null` (decoded but no raster — negative cache), or a pending promise
 * (dedupe concurrent loads of the same tile). Bounded in practice by the number of
 * distinct assets browsed in a session; cleared only on reload.
 */
type CacheEntry = string | null;
const cache = new Map<string, CacheEntry>();
const inflight = new Map<string, Promise<CacheEntry>>();

function cacheKey(assetId: string, maxSize: number): string {
  return `${assetId}@${maxSize}`;
}

/**
 * Resolve (and memoize) one asset's thumbnail data-URL. Returns the cached value
 * synchronously when present; otherwise fetches via `momentThumbnail(asset.path…)`,
 * caches the outcome (including `null`), and resolves. Concurrent calls for the
 * same key share one in-flight request.
 *
 * Returns `null` for non-raster kinds, an asset with no resolvable path, outside
 * Tauri (the invoke wrapper returns undefined → cached null), or on decode failure.
 */
export async function loadAssetThumbnail(
  asset: MediaAssetView,
  maxSize = TILE_THUMBNAIL_MAX,
  sourceSeconds = TILE_THUMBNAIL_SECONDS,
): Promise<CacheEntry> {
  const key = cacheKey(asset.id, maxSize);
  if (cache.has(key)) return cache.get(key) ?? null;
  const existing = inflight.get(key);
  if (existing) return existing;

  if (!hasRasterThumbnail(asset.type) || !asset.path) {
    cache.set(key, null);
    return null;
  }

  const promise = momentThumbnail(asset.path, sourceSeconds, maxSize)
    .then((url) => {
      const value: CacheEntry = url ?? null;
      cache.set(key, value);
      inflight.delete(key);
      return value;
    })
    .catch(() => {
      cache.set(key, null);
      inflight.delete(key);
      return null;
    });
  inflight.set(key, promise);
  return promise;
}

/** Test/diagnostic helper: drop the cache (e.g. after a relink repoints a path). */
export function clearThumbnailCache(): void {
  cache.clear();
  inflight.clear();
}

/**
 * Hook: the data-URL for `asset`'s thumbnail, lazily loaded on first mount and
 * memoized across the session. Returns the prebuilt one on `asset.thumbnailUrl`
 * (a pipeline that already filled it) without a fetch; else `null` until the
 * decode resolves (the tile renders its type glyph in the meantime). Re-fetches
 * when the asset id or its path changes (e.g. after Relink).
 */
export function useAssetThumbnail(
  asset: MediaAssetView,
  maxSize = TILE_THUMBNAIL_MAX,
): string | null {
  const [url, setUrl] = useState<string | null>(
    asset.thumbnailUrl ?? cache.get(cacheKey(asset.id, maxSize)) ?? null,
  );

  useEffect(() => {
    // A pre-filled thumbnail wins (no decode needed).
    if (asset.thumbnailUrl) {
      setUrl(asset.thumbnailUrl);
      return;
    }
    let active = true;
    void loadAssetThumbnail(asset, maxSize).then((value) => {
      if (active) setUrl(value);
    });
    return () => {
      active = false;
    };
    // Re-run when the identity or the source path changes (Relink repoints path).
  }, [asset, asset.id, asset.path, asset.thumbnailUrl, maxSize]);

  return url;
}
