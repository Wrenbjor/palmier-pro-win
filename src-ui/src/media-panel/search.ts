// Search logic for the media panel (E4-S10 scaffold; E11-S11 Moments/Spoken wiring).
//
// Name search (Files section) always works with no backend. The Moments (visual)
// and Spoken (transcript) sections are fed by Epic 11's search backend
// (E11-S6 `coordinator.search` for visual, E11-S8 `TranscriptSearch.search` for
// spoken), surfaced via the `search_media` Tauri command (E11-S10). Ported from
// `MediaTab/MediaTab+Search.swift` (`scheduleMomentSearch`).
//
// Parity with the reference `scheduleMomentSearch`:
//   - 250ms debounce, cancel the prior in-flight task.
//   - spoken runs SYNC (keyword match, no model — always available),
//     visual runs ASYNC (`await coordinator.search`), into separate hit arrays.
//   - empty trimmed query ⇒ clear both hit arrays.
//
// LIVE-WIRING SEAM: `runVisualSearch` / `runSpokenSearch` call the `search_media`
// command (scope=visual / scope=spoken) when it exists; outside Tauri or before
// E11-S10 lands they return [] so the panel renders "No matches". The adapter that
// turns the serde payload into VisualHit[]/SpokenHit[] is the only thing that
// changes when the backend lands.

import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./media-actions";
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
 * Assemble the search-results panel data. `moments`/`spoken` come from the Epic 11
 * search backend; `files` is computed locally and always works.
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

/** Assets eligible to feed the moment search (video/audio only). */
export function momentSearchCandidates(
  assets: readonly MediaAssetView[],
): MediaAssetView[] {
  return assets.filter((a) => a.type === "video" || a.type === "audio");
}

// --- Live-wiring seam to the E11-S10 `search_media` command -------------------
//
// `search_media(query, scope?, media_ref?, limit?)` returns
//   { hits: [{ score, media_ref, range?, image? }], visual_status }
// per FOUNDATION §6.14. For the UI we want the typed Hit shapes:
//   visual → VisualHit{ assetID, time, shotStart, shotEnd, score }
//   spoken → SpokenHit{ assetID, start, end, text }
// E11-S10 should expose those fields directly (the reference returns
// `VisualSearch.Hit` / `TranscriptSearch.Hit` to the UI), so the adapters below
// map the command payload 1:1. Until the command exists they return [].

interface VisualHitPayload {
  assetID?: string;
  asset_id?: string;
  media_ref?: string;
  time: number;
  shotStart?: number;
  shot_start?: number;
  shotEnd?: number;
  shot_end?: number;
  score: number;
}

interface SpokenHitPayload {
  assetID?: string;
  asset_id?: string;
  media_ref?: string;
  start: number;
  end: number;
  text: string;
}

function adaptVisual(p: VisualHitPayload): VisualHit {
  const shotStart = p.shotStart ?? p.shot_start ?? p.time;
  const shotEnd = p.shotEnd ?? p.shot_end ?? shotStart;
  return {
    assetID: p.assetID ?? p.asset_id ?? p.media_ref ?? "",
    time: p.time,
    shotStart,
    shotEnd,
    score: p.score,
  };
}

function adaptSpoken(p: SpokenHitPayload): SpokenHit {
  return {
    assetID: p.assetID ?? p.asset_id ?? p.media_ref ?? "",
    start: p.start,
    end: p.end,
    text: p.text,
  };
}

/**
 * Visual ("Moments") search — async. Calls `search_media` (scope=visual) and adapts
 * the payload to VisualHit[]. Returns [] outside Tauri or if the command is absent
 * (so the panel renders "No matches" until E11-S10 lands).
 */
export async function runVisualSearch(query: string): Promise<VisualHit[]> {
  if (!inTauri()) return [];
  try {
    const res = await invoke<{ hits?: VisualHitPayload[] }>("search_media", {
      query,
      scope: "visual",
    });
    return (res.hits ?? []).map(adaptVisual);
  } catch {
    return [];
  }
}

/**
 * Spoken (transcript) search — sync in the reference (keyword match, no model). The
 * keyword index is local; here we still go through `search_media` (scope=spoken) for
 * strict layering (no transcript cache in the frontend). Returns [] outside Tauri /
 * before the command lands.
 */
export async function runSpokenSearch(query: string): Promise<SpokenHit[]> {
  if (!inTauri()) return [];
  try {
    const res = await invoke<{ hits?: SpokenHitPayload[] }>("search_media", {
      query,
      scope: "spoken",
    });
    return (res.hits ?? []).map(adaptSpoken);
  } catch {
    return [];
  }
}

/**
 * `scheduleMomentSearch` (MediaTab+Search.swift): debounce 250ms + cancel prior
 * in-flight task, then run spoken + visual search into separate hit arrays and hand
 * them to `onResults`. Returns a cancel fn.
 *
 * Parity: the reference computes `spoken` synchronously then `await`s the visual
 * coordinator, assigning `visualHits = visual; spokenHits = spoken` only if the task
 * was not cancelled. Here both run after the debounce; results are delivered together.
 */
export function scheduleMomentSearch(
  query: string,
  _candidates: readonly MediaAssetView[],
  onResults: (r: { moments: VisualHit[]; spoken: SpokenHit[] }) => void,
  debounceMs = MOMENT_SEARCH_DEBOUNCE_MS,
): () => void {
  const trimmed = query.trim();
  if (trimmed.length === 0) {
    onResults({ moments: [], spoken: [] });
    return () => {};
  }
  let cancelled = false;
  const timer = setTimeout(() => {
    void Promise.all([
      runSpokenSearch(trimmed),
      runVisualSearch(trimmed),
    ]).then(([spoken, moments]) => {
      if (!cancelled) onResults({ moments, spoken });
    });
  }, debounceMs);
  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}

/**
 * Source seconds → integer frame, parity with the reference `secondsToFrame`
 * (`Utilities/TimeFormatting.swift`): `Int(seconds * fps)` — truncates toward zero,
 * NOT rounded. Used by the moment/spoken tap → `selectMediaAsset(atSourceFrame:)`.
 */
export function secondsToFrame(seconds: number, fps: number): number {
  return Math.trunc(seconds * fps);
}

/**
 * Format source seconds as a timecode for hit labels. Parity with the reference
 * `timecode(_:)`: rounds to the nearest second, shows `h:mm:ss` at ≥1 hour else
 * `m:ss`.
 */
export function formatTimecode(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s >= 3600) {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const rem = s % 60;
    return `${h}:${m.toString().padStart(2, "0")}:${rem.toString().padStart(2, "0")}`;
  }
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return `${m}:${rem.toString().padStart(2, "0")}`;
}
