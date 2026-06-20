// Inspector resolution logic (E12-S2) — PURE functions, no React, no Tauri.
//
// This is the 1:1 port of the reference `InspectorView` resolution rules:
//   - headerTitle / headerIcon              (inspector.md §Header/title)
//   - availableTabs / aiEditEligible        (inspector.md §Clip-tab-set; FOUNDATION §6.7)
//   - resolvePreferredTab / activeTab        (selection-change tab gating)
//   - project + format metadata math         (inspector.md §Project-metadata)
//
// Everything here is a pure function of `InspectorInput` so it is exercised by
// `tsc --noEmit` + the framework-free `parity.checks.ts` (same convention as the
// editor's `geometry.ts`). The interactive React shell (`InspectorPanel.tsx`)
// only renders the resolved `InspectorState`; all behavior lives here.

import type { ClipType, ClipView } from "../types";
import type {
  AccountState,
  ClipTab,
  FormatMetadata,
  InspectorInput,
  InspectorState,
  MediaAssetView,
  NoSelectionPanel,
  ProjectMetadata,
  ResolvedHeader,
} from "./types";

/** `mediaType.isVisual` (reference `ClipType.isVisual`): video | image | text | lottie. */
const VISUAL: ReadonlySet<ClipType> = new Set<ClipType>([
  "video",
  "image",
  "text",
  "lottie",
]);

export function isVisual(type: ClipType): boolean {
  return VISUAL.has(type);
}

// ── Selection accessors (reference selectedVisualClips/selectedAudioClips) ──────

/** All selected VISUAL clips, in track→clip order. */
export function selectedVisualClips(input: InspectorInput): ClipView[] {
  const out: ClipView[] = [];
  if (!input.timeline || input.selectedClipIds.size === 0) return out;
  for (const track of input.timeline.tracks) {
    for (const clip of track.clips) {
      if (input.selectedClipIds.has(clip.id) && isVisual(clip.mediaType)) {
        out.push(clip);
      }
    }
  }
  return out;
}

/** All selected AUDIO clips, in track→clip order. */
export function selectedAudioClips(input: InspectorInput): ClipView[] {
  const out: ClipView[] = [];
  if (!input.timeline || input.selectedClipIds.size === 0) return out;
  for (const track of input.timeline.tracks) {
    for (const clip of track.clips) {
      if (input.selectedClipIds.has(clip.id) && clip.mediaType === "audio") {
        out.push(clip);
      }
    }
  }
  return out;
}

/** Selected visual clips that are not text (drives the Video tab). */
export function nonTextVisualClips(input: InspectorInput): ClipView[] {
  return selectedVisualClips(input).filter((c) => c.mediaType !== "text");
}

/** The single selected media asset, or null (reference `selectedMediaAsset`). */
export function selectedMediaAsset(
  input: InspectorInput,
): MediaAssetView | null {
  if (input.selectedMediaAssetIds.size !== 1) return null;
  const id = [...input.selectedMediaAssetIds][0];
  return input.mediaAssets.find((a) => a.id === id) ?? null;
}

/**
 * The visual MediaAsset backing the selected visual clip (reference
 * `resolvedClipAsset`): the clip must be visual and its `mediaRef` must resolve
 * to an asset whose media is visual.
 */
export function resolvedClipAsset(
  input: InspectorInput,
): MediaAssetView | null {
  const clips = selectedVisualClips(input);
  const clip = clips[0];
  if (!clip || !isVisual(clip.mediaType)) return null;
  const asset = input.mediaAssets.find((a) => a.id === clip.mediaRef) ?? null;
  return asset && asset.isVisual ? asset : null;
}

/**
 * Link-partner IDs of a clip: every OTHER clip sharing its `linkGroupId`
 * (reference `editor.linkedPartnerIds(of:)`). Returns an empty set if the clip
 * is not linked.
 */
export function linkedPartnerIds(
  input: InspectorInput,
  clipId: string,
): ReadonlySet<string> {
  const partners = new Set<string>();
  if (!input.timeline) return partners;
  let groupId: string | null | undefined;
  for (const track of input.timeline.tracks) {
    for (const clip of track.clips) {
      if (clip.id === clipId) groupId = clip.linkGroupId;
    }
  }
  if (!groupId) return partners;
  for (const track of input.timeline.tracks) {
    for (const clip of track.clips) {
      if (clip.linkGroupId === groupId && clip.id !== clipId) {
        partners.add(clip.id);
      }
    }
  }
  return partners;
}

// ── Header (reference headerTitle / headerIcon, marquee override) ───────────────

export function resolveHeader(input: InspectorInput): ResolvedHeader {
  // While marquee-selecting the header is always "Inspector" + slider icon.
  if (input.isMarqueeSelecting) {
    return { title: "Inspector", icon: "slider.horizontal.3" };
  }
  const hasVisual = selectedVisualClips(input).length > 0;
  const hasAudio = selectedAudioClips(input).length > 0;
  if (hasVisual || hasAudio) {
    return { title: "Inspector", icon: "slider.horizontal.3" };
  }
  if (selectedMediaAsset(input)) {
    return { title: "Source", icon: "info.circle" };
  }
  return { title: "Timeline", icon: "info.circle" };
}

// ── aiEditEligible (reference) ──────────────────────────────────────────────────

/**
 * True when the selection resolves to a single AI-editable visual clip: exactly
 * one visual clip that resolves to a visual MediaAsset, and any selected audio
 * clips are ALL link-partners of that visual (a linked A/V pair counts as one).
 */
export function aiEditEligible(input: InspectorInput): boolean {
  const visuals = selectedVisualClips(input);
  const audios = selectedAudioClips(input);
  if (visuals.length !== 1 || resolvedClipAsset(input) === null) return false;
  if (audios.length === 0) return true;
  const partners = linkedPartnerIds(input, visuals[0].id);
  return audios.every((a) => partners.has(a.id));
}

// ── availableTabs (reference — ORDER MATTERS) ───────────────────────────────────

/**
 * Tabs in EXACT reference order:
 *   1. Text  — iff the selection is a single text clip.
 *   2. Video — iff there is ≥1 non-text visual clip.
 *   3. Audio — iff there is ≥1 audio clip.
 *   4. AI Edit — iff aiEditEligible && !account.isMisconfigured.
 */
export function availableTabs(input: InspectorInput): ClipTab[] {
  const visuals = selectedVisualClips(input);
  const audios = selectedAudioClips(input);
  const nonText = nonTextVisualClips(input);
  const isSingle = visuals.length + audios.length === 1;
  const isSingleText = isSingle && visuals[0]?.mediaType === "text";

  const tabs: ClipTab[] = [];
  if (isSingleText) tabs.push("text");
  if (nonText.length > 0) tabs.push("video");
  if (audios.length > 0) tabs.push("audio");
  if (aiEditEligible(input) && !input.account.isMisconfigured) tabs.push("ai");
  return tabs;
}

/**
 * The active tab = `preferredTab` if still available, else the first available
 * (reference `activeTab`). `preferredTab` defaults to `"video"`.
 */
export function activeTab(
  tabs: ClipTab[],
  preferredTab: ClipTab,
): ClipTab | null {
  return tabs.includes(preferredTab) ? preferredTab : (tabs[0] ?? null);
}

/**
 * `resolvePreferredTab` — fires on selection change. Returns the NEXT preferred
 * tab given the current one and the new selection:
 *   - single text  → force "text"
 *   - leaving text (was "text", no longer single-text) → drop to "video"
 *   - otherwise unchanged.
 * The caller (the shell) is responsible for also clearing `cropEditingActive` on
 * every selection change — see `cropEditingShouldClearOnSelectionChange`.
 */
export function resolvePreferredTab(
  input: InspectorInput,
  current: ClipTab,
): ClipTab {
  const visuals = selectedVisualClips(input);
  const audios = selectedAudioClips(input);
  const isSingleText =
    visuals.length + audios.length === 1 && visuals[0]?.mediaType === "text";
  if (isSingleText) return "text";
  if (current === "text") return "video";
  return current;
}

/**
 * Crop editing is ALWAYS cleared on selection change, and additionally whenever
 * the preferred tab moves off "video" (reference `onChange(preferredTab)`).
 * Exposed as an explicit predicate so the shell wires `cropEditingActive=false`
 * at the same points the reference does.
 */
export function shouldClearCropEditing(nextPreferredTab: ClipTab): boolean {
  return nextPreferredTab !== "video";
}

// ── Project / Format metadata (reference §Project-metadata) ──────────────────────

/** Middle-truncate a string to `max` chars, eliding the middle with "…". */
export function middleTruncate(s: string, max: number): string {
  if (s.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  const tail = Math.floor(keep / 2);
  return `${s.slice(0, head)}…${s.slice(s.length - tail)}`;
}

/** Greatest common divisor (Euclid). Guards against 0. */
export function gcd(a: number, b: number): number {
  a = Math.abs(a);
  b = Math.abs(b);
  while (b !== 0) {
    [a, b] = [b, a % b];
  }
  return a === 0 ? 1 : a;
}

/** File stem: last path component with its extension removed. */
export function fileStem(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? path;
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

/** Aspect ratio `W:H` reduced by gcd. */
export function formatAspectRatio(width: number, height: number): string {
  const g = gcd(width, height);
  return `${Math.round(width / g)}:${Math.round(height / g)}`;
}

/**
 * `formatDuration` (reference): round to whole seconds, then `H:MM:SS` if there
 * are hours, else `M:SS` (minutes are NOT zero-padded; seconds always are).
 */
export function formatDuration(seconds: number): string {
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const mins = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (hours > 0) return `${hours}:${pad(mins)}:${pad(secs)}`;
  return `${mins}:${pad(secs)}`;
}

/** The "Project" section, or null when there is no project path. */
export function resolveProject(input: InspectorInput): ProjectMetadata | null {
  if (!input.projectPath) return null;
  return { name: fileStem(input.projectPath), path: input.projectPath };
}

/** The "Format" section, or null when there is no timeline. */
export function resolveFormat(input: InspectorInput): FormatMetadata | null {
  const t = input.timeline;
  if (!t) return null;
  const durationSeconds = t.fps > 0 ? totalFrames(t.tracks) / t.fps : 0;
  return {
    resolution: `${t.width} × ${t.height}`,
    frameRate: `${t.fps} fps`,
    aspectRatio: formatAspectRatio(t.width, t.height),
    duration: formatDuration(durationSeconds),
  };
}

/**
 * Total timeline frames = the max clip end across all tracks (reference
 * `timeline.totalFrames`). The `TimelineView` does not carry an explicit
 * `totalFrames`, so it is derived from clip extents here.
 */
export function totalFrames(
  tracks: { clips: { startFrame: number; durationFrames: number }[] }[],
): number {
  let max = 0;
  for (const track of tracks) {
    for (const clip of track.clips) {
      const end = clip.startFrame + clip.durationFrames;
      if (end > max) max = end;
    }
  }
  return max;
}

function resolveNoSelection(input: InspectorInput): NoSelectionPanel {
  return { project: resolveProject(input), format: resolveFormat(input) };
}

// ── Top-level resolver ──────────────────────────────────────────────────────────

/**
 * Resolve the full Inspector state from its input + the persisted `preferredTab`.
 * This is the single function the shell calls each render. `preferredTab` is
 * UI-persistent state owned by the shell (seeded "video"); the shell updates it
 * via `resolvePreferredTab` on selection change before resolving.
 */
export function resolveInspector(
  input: InspectorInput,
  preferredTab: ClipTab,
): InspectorState {
  const header = resolveHeader(input);

  if (input.isMarqueeSelecting) {
    return {
      header,
      mode: "marquee",
      marqueeCount: input.selectedClipIds.size,
      tabs: [],
      activeTab: null,
      showTabBar: false,
      noSelection: null,
    };
  }

  const hasVisual = selectedVisualClips(input).length > 0;
  const hasAudio = selectedAudioClips(input).length > 0;
  if (hasVisual || hasAudio) {
    const tabs = availableTabs(input);
    return {
      header,
      mode: "clip",
      marqueeCount: 0,
      tabs,
      activeTab: activeTab(tabs, preferredTab),
      showTabBar: tabs.length > 1,
      noSelection: null,
    };
  }

  if (selectedMediaAsset(input)) {
    return {
      header,
      mode: "asset",
      marqueeCount: 0,
      tabs: [],
      activeTab: null,
      showTabBar: false,
      noSelection: null,
    };
  }

  return {
    header,
    mode: "project",
    marqueeCount: 0,
    tabs: [],
    activeTab: null,
    showTabBar: false,
    noSelection: resolveNoSelection(input),
  };
}

/** Re-export so the controller can type its account-state seam without indirection. */
export type { AccountState };
