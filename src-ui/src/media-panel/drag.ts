// Drag-payload URI scheme — the byte-for-byte contract between the media panel,
// the timeline drop target, and agent "moment" drags (media-panel.md §"Drag-drop"
// + §"Port risks": this format is load-bearing; keep it exact).
//
// Schemes:
//   palmier-folder://<id>
//   palmier-asset://<id>
//   palmier-asset://<id>#<start>-<end>   (search moment; source seconds, "%.3f")
//
// Drag payload for a selected asset = ALL selected ids newline-joined; else just
// that id. The full in-panel/timeline DnD wiring is E4-S12; this module supplies
// the payload-building/parsing primitives E4-S11 (selection) + the search panel use.

export const ASSET_SCHEME = "palmier-asset://";
export const FOLDER_SCHEME = "palmier-folder://";

/** Source-seconds formatter — reference uses `%.3f`. */
export function formatSourceSeconds(seconds: number): string {
  return seconds.toFixed(3);
}

/** `palmier-asset://<id>`. */
export function assetUri(id: string): string {
  return `${ASSET_SCHEME}${id}`;
}

/** `palmier-folder://<id>`. */
export function folderUri(id: string): string {
  return `${FOLDER_SCHEME}${id}`;
}

/** `palmier-asset://<id>#<start>-<end>` with start/end as `%.3f` source seconds. */
export function momentUri(id: string, start: number, end: number): string {
  return `${ASSET_SCHEME}${id}#${formatSourceSeconds(start)}-${formatSourceSeconds(end)}`;
}

/**
 * Build the drag payload for a primary asset: if the asset is part of the current
 * selection, emit all selected ids newline-joined (as `palmier-asset://` URIs);
 * otherwise emit just the primary id. Selection order is preserved.
 */
export function buildAssetDragPayload(
  primaryId: string,
  selection: ReadonlySet<string>,
): string {
  if (selection.has(primaryId) && selection.size > 1) {
    return Array.from(selection).map(assetUri).join("\n");
  }
  return assetUri(primaryId);
}

export type ParsedUri =
  | { kind: "folder"; id: string }
  | { kind: "asset"; id: string }
  | { kind: "moment"; id: string; start: number; end: number }
  | null;

/** Parse one URI line back into a typed payload (null if unrecognized). */
export function parseUri(line: string): ParsedUri {
  const s = line.trim();
  if (s.startsWith(FOLDER_SCHEME)) {
    return { kind: "folder", id: s.slice(FOLDER_SCHEME.length) };
  }
  if (s.startsWith(ASSET_SCHEME)) {
    const rest = s.slice(ASSET_SCHEME.length);
    const hash = rest.indexOf("#");
    if (hash < 0) return { kind: "asset", id: rest };
    const id = rest.slice(0, hash);
    const range = rest.slice(hash + 1);
    const dash = range.indexOf("-");
    if (dash < 0) return { kind: "asset", id };
    const start = Number(range.slice(0, dash));
    const end = Number(range.slice(dash + 1));
    if (Number.isNaN(start) || Number.isNaN(end)) return { kind: "asset", id };
    return { kind: "moment", id, start, end };
  }
  return null;
}

/** Parse a newline-joined payload into its typed parts (drops unrecognized lines). */
export function parsePayload(payload: string): NonNullable<ParsedUri>[] {
  return payload
    .split("\n")
    .map(parseUri)
    .filter((p): p is NonNullable<ParsedUri> => p !== null);
}
