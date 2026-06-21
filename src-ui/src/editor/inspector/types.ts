// Inspector view-model types (E12-S2).
//
// The right-rail Inspector is a PURE VIEW over reactive editor state. Its inputs
// are the timeline + the current selection + the media-asset selection + the
// account state — exactly the slices the reference `InspectorView` reads off
// `EditorViewModel` / `AccountService`. These types mirror that input contract so
// the resolution logic (`logic.ts`) ports 1:1 from the Swift reference.
//
// Naming follows `docs/reference/inspector.md` §Header/title, §Clip-tab-set,
// §Project-metadata and `Inspector/InspectorView.swift`.

import type { ClipView, TimelineView } from "../types";

/** The clip-inspector tabs, in the reference's `ClipTab` raw-value order. */
export type ClipTab = "text" | "video" | "audio" | "ai";

/** Display labels for each tab (reference `ClipTab.rawValue`). */
export const CLIP_TAB_LABEL: Record<ClipTab, string> = {
  text: "Text",
  video: "Video",
  audio: "Audio",
  ai: "AI Edit",
};

/** Header title resolution result. */
export type InspectorTitle = "Inspector" | "Source" | "Timeline";

/** Reference SF-Symbol icon names (mapped to a glyph by the header component). */
export type InspectorIcon = "slider.horizontal.3" | "info.circle";

/**
 * A selected media asset (the media-panel "Source" selection). The Inspector
 * needs only enough to (a) know one is selected and (b) resolve `aiEditEligible`
 * for a clip whose `mediaRef` points at a visual asset. Mirrors the
 * render-relevant subset of `palmier-model::MediaAsset`.
 */
export interface MediaAssetView {
  id: string;
  /** Whether the asset's media is visual (video/image) vs audio-only. */
  isVisual: boolean;
}

/**
 * The account/AI state the tab-gating reads (reference `AccountService`).
 * Sourced from the app shell's `AccountSnapshot` (`app/api.ts`) via the seam in
 * `controller.ts`. `isMisconfigured` hides the AI Edit tab entirely.
 */
export interface AccountState {
  isMisconfigured: boolean;
}

/**
 * Everything the Inspector resolves its header + tabs + bodies from. This is the
 * reactive input the controller assembles from the timeline store, the
 * media-panel selection, and the account snapshot — the Inspector view itself is
 * a pure function of this.
 */
export interface InspectorInput {
  timeline: TimelineView | null;
  /** Selected clip IDs (the timeline selection — `editor.selectedClipIds`). */
  selectedClipIds: ReadonlySet<string>;
  /** Selected media-asset IDs (the media-panel "Source" selection). */
  selectedMediaAssetIds: ReadonlySet<string>;
  /** The media library, for resolving a clip's backing asset (`mediaRef`). */
  mediaAssets: readonly MediaAssetView[];
  /** True while a marquee drag is live (`editor.isMarqueeSelecting`). */
  isMarqueeSelecting: boolean;
  /** Account / AI state (`AccountService.isMisconfigured`). */
  account: AccountState;
  /**
   * The project file path (`editor.projectURL.path`), or null when unsaved /
   * no project. Drives the no-selection "Project" section (name = file stem).
   */
  projectPath: string | null;
  /**
   * The playhead frame (`editor.playheadFrame`) — OPTIONAL so the existing shell
   * mount (which does not yet supply it) type-checks. The tab BODIES (E12-S5/S8)
   * sample keyframe-driven fields and the keyframes panel at this frame; absent →
   * bodies fall back to the clip's static scalar values.
   */
  activeFrame?: number;
}

/** Resolved header (title + icon), accounting for the marquee-active override. */
export interface ResolvedHeader {
  title: InspectorTitle;
  icon: InspectorIcon;
}

/** The "Project" metadata section (only present when a project path exists). */
export interface ProjectMetadata {
  /** File stem (path without directory or extension). */
  name: string;
  /** Full path, rendered middle-truncated by the view. */
  path: string;
}

/** The "Format" metadata section (always present at no-selection). */
export interface FormatMetadata {
  /** `W × H`. */
  resolution: string;
  /** `fps fps`. */
  frameRate: string;
  /** `W:H` reduced by gcd. */
  aspectRatio: string;
  /** `H:MM:SS` or `M:SS`. */
  duration: string;
}

/** The resolved no-selection panel content. */
export interface NoSelectionPanel {
  project: ProjectMetadata | null;
  format: FormatMetadata | null;
}

/**
 * The fully-resolved Inspector state — the single value the view renders. A pure
 * function of `InspectorInput` (see `resolveInspector` in `logic.ts`).
 */
export interface InspectorState {
  header: ResolvedHeader;
  /** Which content region renders. */
  mode: "marquee" | "clip" | "asset" | "project";
  /** Marquee mode: centered "N selected" count. */
  marqueeCount: number;
  /** Clip mode: tabs in reference order (empty in other modes). */
  tabs: ClipTab[];
  /** Clip mode: the active tab (preferred if available, else first). */
  activeTab: ClipTab | null;
  /** Clip mode: whether the tab bar is shown (hidden when ≤1 tab). */
  showTabBar: boolean;
  /** Project mode: resolved metadata sections. */
  noSelection: NoSelectionPanel | null;
}

/** Re-export the clip/timeline types the inspector consumes, for convenience. */
export type { ClipView, TimelineView };
