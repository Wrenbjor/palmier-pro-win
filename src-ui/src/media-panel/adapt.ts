// adaptMedia — map the `get_media` wire JSON → the panel `MediaSnapshot`.
//
// The `editor_get_media` Tauri command (crates/palmier-tauri/src/commands.rs) returns
// the reference asset catalog from read.rs `get_media`:
//   { assets: [{ id, name, type, duration, generationStatus, folderId? }] }
// (`duration` is seconds, null for stills; `generationStatus` ∈
//  none | generating | downloading | failed). Folders come from `list_folders`
// (a separate tool); `editor_get_media` returns assets only, so folders default to []
// here until a folder read is threaded in (the panel renders a flat Library then).
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
  return {
    id: asString(raw.id, ""),
    name: asString(raw.name, ""),
    // The catalog projection carries no on-disk path (it lives on the manifest entry);
    // Reveal/Copy-Path degrade gracefully when empty. Threaded in when a path-bearing
    // read lands.
    path: asString(raw.path, ""),
    type: asMediaType(raw.type),
    folderId: typeof raw.folderId === "string" ? raw.folderId : null,
    durationSeconds:
      typeof raw.duration === "number" && Number.isFinite(raw.duration)
        ? raw.duration
        : null,
    // A generated asset is one whose status is not the steady "none" — the AI badge /
    // filter reads this. (A fully-rendered generated asset reverts to "none"; the wire
    // has no persistent generated flag, so this reflects in-flight generation only.)
    isGenerated: generationStatus !== "none" && generationStatus !== "",
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
