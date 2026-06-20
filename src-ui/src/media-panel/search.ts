// Search logic for the media panel (E4-S10).
//
// Name search (Files section) always works with no backend. The Moments (visual)
// and Spoken (transcript) sections are fed by Epic 11's `search_media` command —
// here their data sources are STUBS returning []. The debounce + cancel scaffold
// (`scheduleMomentSearch`) and the result-assembly shape are built now so the
// Epic 11 wiring is a drop-in. Ported from `MediaTab/MediaTab+Search.swift`.

import type {
  MediaAssetView,
  SearchResults,
  SpokenHit,
  VisualHit,
} from "./types";

/** Debounce window before firing a moment search (reference = 250ms). */
export const MOMENT_SEARCH_DEBOUNCE_MS = 250;

/** Name-match Files section: case-insensitive substring over every asset. */
export function fileMatches(
  assets: readonly MediaAssetView[],
  query: string,
): MediaAssetView[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [];
  return assets.filter((a) => a.name.toLowerCase().includes(q));
}

/**
 * Assemble the search-results panel data. `moments`/`spoken` come from the
 * (stubbed) Epic 11 search; `files` is computed locally and always works.
 */
export function assembleResults(
  assets: readonly MediaAssetView[],
  query: string,
  moments: VisualHit[],
  spoken: SpokenHit[],
): SearchResults {
  return {
    moments,
    spoken,
    files: fileMatches(assets, query),
  };
}

/**
 * The async moment-search seam. Real impl lands in Epic 11; this stub returns
 * empty hit lists so the panel renders "No matches" for Moments/Spoken until the
 * backend exists. Only video/audio assets feed it (matches reference gating).
 *
 * TODO(E11): replace body with
 *   const res = await invoke<{moments: VisualHit[]; spoken: SpokenHit[]}>(
 *     'search_media', { query, scope: 'both' });
 *   return res;
 */
export async function runMomentSearchStub(
  _query: string,
  _candidates: readonly MediaAssetView[],
): Promise<{ moments: VisualHit[]; spoken: SpokenHit[] }> {
  return { moments: [], spoken: [] };
}

/** Assets eligible to feed the moment search (video/audio only). */
export function momentSearchCandidates(
  assets: readonly MediaAssetView[],
): MediaAssetView[] {
  return assets.filter((a) => a.type === "video" || a.type === "audio");
}

/**
 * `scheduleMomentSearch` scaffold: debounce + cancel prior in-flight task, then
 * run the (stubbed) search and hand results to `onResults`. Returns a cancel fn.
 * Keeps the 250ms debounce + cancellation semantics; the search call is a stub.
 */
export function scheduleMomentSearch(
  query: string,
  candidates: readonly MediaAssetView[],
  onResults: (r: { moments: VisualHit[]; spoken: SpokenHit[] }) => void,
  debounceMs = MOMENT_SEARCH_DEBOUNCE_MS,
): () => void {
  if (query.trim().length === 0) {
    onResults({ moments: [], spoken: [] });
    return () => {};
  }
  let cancelled = false;
  const timer = setTimeout(() => {
    void runMomentSearchStub(query, candidates).then((r) => {
      if (!cancelled) onResults(r);
    });
  }, debounceMs);
  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}

/** Format source seconds as a `m:ss` timecode for hit labels. */
export function formatTimecode(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return `${m}:${rem.toString().padStart(2, "0")}`;
}
