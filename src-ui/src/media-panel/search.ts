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
  IndexStatus,
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
// `search_media` returns the reference `searchMedia` shape (parity authority —
// `ToolExecutor+Search.swift`), NOT a flat `{ hits }`:
//   {
//     query, scope,
//     visual: { status, moments: [VisualMoment | ImageMoment] },
//     spoken: [SpokenSegment]
//   }
// where
//   VisualMoment = { mediaRef, name, score, startSeconds, endSeconds }   // video shot
//   ImageMoment  = { mediaRef, name, score, type: "image" }              // still: no seconds
//   SpokenSegment = { mediaRef, name, startSeconds, endSeconds, text }
//   status ∈ "ready"|"indexing"|"model_not_installed"|"downloading_model"
//            |"preparing"|"disabled"|"failed"  (snake_case wire string)
//
// The UI's internal Hit shapes carry a `time` (thumbnail seek) the wire shape does
// not: the reference seeks the moment thumbnail to the shot start, so `time` maps
// from `startSeconds` (0 for stills, which render their own thumbnail, not a seek).
// The renderer (`SearchResultsPanel`) owns the `[start, max(end, start+0.1)]` range
// math and the still→plain-asset drag rule, so these adapters keep `shotStart`/
// `shotEnd` raw (= the wire `startSeconds`/`endSeconds`) and let the renderer floor.

/** A video moment (carries a shot range). */
interface VisualMomentPayload {
  mediaRef: string;
  name?: string;
  score: number;
  startSeconds: number;
  endSeconds: number;
}
/** A still-image moment (`type:"image"`, no range). */
interface VisualImagePayload {
  mediaRef: string;
  name?: string;
  score: number;
  type: "image";
}
type VisualMoment = VisualMomentPayload | VisualImagePayload;

interface VisualGroup {
  status: string;
  moments?: VisualMoment[];
}

interface SpokenSegmentPayload {
  mediaRef: string;
  name?: string;
  startSeconds: number;
  endSeconds: number;
  text: string;
}

interface SearchMediaResponse {
  visual?: VisualGroup;
  spoken?: SpokenSegmentPayload[];
}

/** The wire status strings emitted by `visual_status_wire` (reference parity). */
type VisualStatusWire =
  | "ready"
  | "indexing"
  | "model_not_installed"
  | "downloading_model"
  | "preparing"
  | "disabled"
  | "failed";

/** Visual search outcome: the adapted hits plus the live model-loader status. */
export interface VisualSearchOutcome {
  moments: VisualHit[];
  status: VisualStatusWire;
}

function isImageMoment(m: VisualMoment): m is VisualImagePayload {
  return (m as VisualImagePayload).type === "image";
}

function adaptVisual(m: VisualMoment): VisualHit {
  if (isImageMoment(m)) {
    // Still: no shot range. The renderer drags it as a plain asset (it keys off the
    // asset's own `type === "image"`), so its segment fields are inert — set to 0.
    return { assetID: m.mediaRef, time: 0, shotStart: 0, shotEnd: 0, score: m.score };
  }
  // Video shot: thumbnail seeks to the shot start; renderer floors the drag range.
  return {
    assetID: m.mediaRef,
    time: m.startSeconds,
    shotStart: m.startSeconds,
    shotEnd: m.endSeconds,
    score: m.score,
  };
}

function adaptSpoken(s: SpokenSegmentPayload): SpokenHit {
  return {
    assetID: s.mediaRef,
    start: s.startSeconds,
    end: s.endSeconds,
    text: s.text,
  };
}

/**
 * Map the wire `visual.status` to the panel's `IndexStatus` pill state. The wire
 * status is discrete (no fraction/counts), so `downloading_model` shows an
 * indeterminate 0% and `indexing` shows 0/0 — the live progress arrives separately.
 * `ready` (the steady state) is folded by the caller into "don't surface".
 */
export function indexStatusFromWire(status: VisualStatusWire): IndexStatus {
  switch (status) {
    case "ready":
      return { kind: "ready" };
    case "indexing":
      return { kind: "indexing", completed: 0, total: 0 };
    case "model_not_installed":
      return { kind: "notInstalled" };
    case "downloading_model":
      return { kind: "downloading", fraction: 0 };
    case "preparing":
      return { kind: "preparing" };
    case "disabled":
      // No model enabled — surface as "not installed" (offers the download CTA).
      return { kind: "notInstalled" };
    case "failed":
      return { kind: "failed", message: "Visual search failed" };
  }
}

/**
 * Visual ("Moments") search — async. Calls `search_media` (scope=visual), reads
 * `response.visual.moments[]`, and adapts each to a `VisualHit`. Also surfaces the
 * `response.visual.status` (reference parity). Returns empty + `disabled` outside
 * Tauri or if the command throws (so the panel renders "No matches" / the status
 * affordance until the visual backend is wired).
 */
export async function runVisualSearch(query: string): Promise<VisualSearchOutcome> {
  if (!inTauri()) return { moments: [], status: "disabled" };
  try {
    const res = await invoke<SearchMediaResponse>("search_media", {
      query,
      scope: "visual",
    });
    const status = (res.visual?.status as VisualStatusWire) ?? "disabled";
    const moments = (res.visual?.moments ?? []).map(adaptVisual);
    return { moments, status };
  } catch {
    return { moments: [], status: "disabled" };
  }
}

/**
 * Spoken (transcript) search — sync in the reference (keyword match, no model). The
 * keyword index is local; here we still go through `search_media` (scope=spoken) for
 * strict layering (no transcript cache in the frontend). Reads `response.spoken[]`.
 * Returns [] outside Tauri / before the command lands.
 */
export async function runSpokenSearch(query: string): Promise<SpokenHit[]> {
  if (!inTauri()) return [];
  try {
    const res = await invoke<SearchMediaResponse>("search_media", {
      query,
      scope: "spoken",
    });
    return (res.spoken ?? []).map(adaptSpoken);
  } catch {
    return [];
  }
}

/** The debounced search outcome handed to `scheduleMomentSearch`'s callback. */
export interface MomentSearchResult {
  moments: VisualHit[];
  spoken: SpokenHit[];
  /** The live `visual.status` wire string (`disabled` outside Tauri). */
  visualStatus: VisualStatusWire;
}

/**
 * `scheduleMomentSearch` (MediaTab+Search.swift): debounce 250ms + cancel prior
 * in-flight task, then run spoken + visual search into separate hit arrays and hand
 * them to `onResults`. Returns a cancel fn.
 *
 * Parity: the reference computes `spoken` synchronously then `await`s the visual
 * coordinator, assigning `visualHits = visual; spokenHits = spoken` only if the task
 * was not cancelled. Here both run after the debounce; results are delivered together.
 * The visual coordinator's `status` rides along so the panel can surface a
 * non-`ready` model-loader state.
 */
export function scheduleMomentSearch(
  query: string,
  _candidates: readonly MediaAssetView[],
  onResults: (r: MomentSearchResult) => void,
  debounceMs = MOMENT_SEARCH_DEBOUNCE_MS,
): () => void {
  const trimmed = query.trim();
  if (trimmed.length === 0) {
    onResults({ moments: [], spoken: [], visualStatus: "disabled" });
    return () => {};
  }
  let cancelled = false;
  const timer = setTimeout(() => {
    void Promise.all([
      runSpokenSearch(trimmed),
      runVisualSearch(trimmed),
    ]).then(([spoken, visual]) => {
      if (!cancelled)
        onResults({
          moments: visual.moments,
          spoken,
          visualStatus: visual.status,
        });
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
