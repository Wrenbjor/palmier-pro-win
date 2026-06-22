// adaptMedia — map the `get_media` wire JSON → the panel `MediaSnapshot`.
//
// The `editor_get_media` Tauri command (crates/palmier-tauri/src/commands.rs) returns
// an ENRICHED asset payload (richer than the compact MCP `get_media` tool the LLM uses):
//   { assets: [{ id, name, type, duration, hasAudio, generationStatus, folderId?,
//                path?, width?, height?, sizeBytes?, isGenerated,
//                generatedModel?, generatedAspect?, generatedResolution?, prompt? }] }
// (`duration` is seconds, null for stills; `path` is the ABSOLUTE on-disk path that
//  drives tile thumbnails via `useAssetThumbnail`; `width`/`height` are source pixels;
//  `sizeBytes` is the real file size; `isGenerated` is the persistent AI-generated flag).
// Folders come from `list_folders` (a separate tool); `editor_get_media` returns assets
// only, so folders default to [] here until a folder read is threaded in.
//
// This is the single seam that turns the serde payload into the `MediaSnapshot` the
// panel stores — the boundary `media-panel/types.ts` always anticipated ("the adapter
// that turns the serde payload into a MediaSnapshot is the only thing that changes").

import type {
  MediaAssetView,
  MediaFolderView,
  MediaSnapshot,
  MediaType,
} from "./types";

type Json = Record<string, unknown>;

function asString(v: unknown, fallback: string): string {
  return typeof v === "string" ? v : fallback;
}

/** A finite number, or undefined (drops nulls / non-numbers / NaN). */
function asFiniteNumber(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** An optional string field (dropped when absent / non-string). */
function asOptString(v: unknown): string | undefined {
  return typeof v === "string" && v.length > 0 ? v : undefined;
}

const MEDIA_TYPES: readonly MediaType[] = [
  "video",
  "image",
  "text",
  "lottie",
  "audio",
];
function asMediaType(v: unknown): MediaType {
  return typeof v === "string" && (MEDIA_TYPES as readonly string[]).includes(v)
    ? (v as MediaType)
    : "video";
}

/** Map one wire asset object → a MediaAssetView (defaults filled). */
function adaptAsset(raw: Json): MediaAssetView {
  const generationStatus = asString(raw.generationStatus, "none");
  // The enriched payload carries a persistent `isGenerated` boolean (true when the
  // asset has a generationInput). Fall back to the status heuristic only if the wire
  // omitted it (older payloads / partial reads).
  const isGenerated =
    typeof raw.isGenerated === "boolean"
      ? raw.isGenerated
      : generationStatus !== "none" && generationStatus !== "";
  return {
    id: asString(raw.id, ""),
    name: asString(raw.name, ""),
    // Absolute on-disk path (enriched `editor_get_media`). Drives the tile thumbnail
    // (`useAssetThumbnail` decodes a frame from this path) + Reveal/Copy-Path. Empty
    // when the asset has no resolvable source (the tile falls back to its type glyph).
    path: asString(raw.path, ""),
    type: asMediaType(raw.type),
    folderId: typeof raw.folderId === "string" ? raw.folderId : null,
    durationSeconds: asFiniteNumber(raw.duration) ?? null,
    width: asFiniteNumber(raw.width),
    height: asFiniteNumber(raw.height),
    sizeBytes: asFiniteNumber(raw.sizeBytes),
    hasAudio: typeof raw.hasAudio === "boolean" ? raw.hasAudio : undefined,
    isGenerated,
    generatedModel: asOptString(raw.generatedModel),
    generatedAspect: asOptString(raw.generatedAspect),
    generatedResolution: asOptString(raw.generatedResolution),
    prompt: asOptString(raw.prompt),
    missing: generationStatus === "failed" || undefined,
  };
}

/** Map one wire folder object (from list_folders) → a MediaFolderView. */
function adaptFolder(raw: Json): MediaFolderView {
  return {
    id: asString(raw.id, ""),
    name: asString(raw.name, ""),
    parentFolderId:
      typeof raw.parentFolderId === "string" ? raw.parentFolderId : null,
  };
}

/**
 * Map the `editor_get_media` wire JSON → a `MediaSnapshot`. Optional `folders` are
 * accepted (a future folder-bearing read), else default to []. Tolerant of a missing
 * `assets` array so it never throws on a partial / early-boot payload.
 */
export function adaptMedia(wire: unknown): MediaSnapshot {
  const raw = (typeof wire === "object" && wire !== null ? wire : {}) as Json;
  const assets = Array.isArray(raw.assets)
    ? raw.assets
        .filter((a): a is Json => typeof a === "object" && a !== null)
        .map(adaptAsset)
    : [];
  const folders = Array.isArray(raw.folders)
    ? raw.folders
        .filter((f): f is Json => typeof f === "object" && f !== null)
        .map(adaptFolder)
    : [];
  return { assets, folders };
}
