// Timeline view model types for the 2D timeline canvas (E3-S9).
//
// These are the FRONTEND view types — a TS mirror of the Rust `palmier-model`
// Timeline/Track/Clip shape, carrying only what the canvas needs to render.
// The real `get_timeline` Tauri command does not exist yet (Epic 7), so the
// canvas is fed a `TimelineView` via a prop / Zustand store, populated from a
// fixture (see `fixture.ts`). When the command lands, the adapter that turns the
// serde payload into a `TimelineView` is the only thing that changes.
//
// Naming and field semantics follow `docs/reference/timeline-model.md` §"Data
// model" and the macOS reference (`Models/Timeline.swift`). IDs are UUID
// strings, not typed Uuid (reference parity). Keyframe frames are CLIP-RELATIVE
// offsets in storage (timeline-model.md line 69) — the same convention is used
// here so the sampling math ports 1:1.

/** Clip / track media kinds. `isVisual = video | image | text | lottie`. */
export type ClipType = "video" | "image" | "text" | "lottie" | "audio";

/** Keyframe interpolation out of a segment. Reference default is `smooth`. */
export type Interpolation = "linear" | "hold" | "smooth";

/** A single keyframe. `frame` is a CLIP-RELATIVE offset. */
export interface KeyframeView<V = number> {
  frame: number;
  value: V;
  /** Interpolation applied on the segment LEAVING this keyframe. */
  interpolationOut: Interpolation;
}

/** A keyframe track. `isActive` mirrors the reference `!keyframes.isEmpty`. */
export interface KeyframeTrackView<V = number> {
  keyframes: KeyframeView<V>[];
}

/**
 * A clip as the canvas needs to draw it. Mirrors the render-relevant subset of
 * `palmier-model::Clip` (timeline-model.md lines 50-56). Frame fields are in
 * timeline frames unless noted; trim fields are in source frames.
 */
export interface ClipView {
  id: string;
  /** Display name (resolved media name / text content). */
  name: string;
  mediaRef: string;
  /** The type used for the visual content drawn in the body. */
  mediaType: ClipType;
  /** The type that drives the THEME COLOR + fill (reference: `sourceClipType`). */
  sourceClipType: ClipType;

  startFrame: number;
  durationFrames: number;
  /** Source frames trimmed off the head / tail (source-frame units). */
  trimStartFrame: number;
  trimEndFrame: number;
  speed: number;

  /** Static linear volume (audio). dB is derived via VolumeScale. */
  volume: number;
  opacity: number;

  fadeInFrames: number;
  fadeOutFrames: number;
  fadeInInterpolation: Interpolation;
  fadeOutInterpolation: Interpolation;

  /** Present → clip is part of a link group (name underlined when set). */
  linkGroupId?: string | null;

  /** Source media could not be resolved → red wash + red border. */
  isMissing?: boolean;
  /** Clip is mid-generation (suppresses the missing-media wash). */
  isGenerating?: boolean;

  /** Keyframe tracks. Volume kfs render on the rubber band; others as diamonds. */
  volumeTrack?: KeyframeTrackView | null;
  opacityTrack?: KeyframeTrackView | null;
  positionTrack?: KeyframeTrackView | null;
  scaleTrack?: KeyframeTrackView | null;
  cropTrack?: KeyframeTrackView | null;

  /**
   * Optional per-source audio peak samples in [0,1], dB-normalised
   * (0 = loud, 1 = silent) — used by the waveform. Omitted → placeholder bars.
   */
  waveform?: number[] | null;
}

/** A track lane. Visual tracks stack above audio tracks (caller-ordered). */
export interface TrackView {
  id: string;
  type: ClipType;
  muted: boolean;
  hidden: boolean;
  syncLocked: boolean;
  /** Lane height in px. Reference default 50 (not serialized). */
  displayHeight: number;
  clips: ClipView[];
}

/** The whole timeline the canvas renders. */
export interface TimelineView {
  fps: number;
  width: number;
  height: number;
  tracks: TrackView[];
}

/** Inclusive-exclusive `[start, end)` time-range selection (ruler shift-drag). */
export interface TimelineRangeSelectionView {
  startFrame: number;
  endFrame: number;
}

/** Viewport / zoom / playhead state — the parts of editor state the canvas reads. */
export interface TimelineViewport {
  /** Horizontal scroll offset in px (frame 0 sits at headerWidth - scrollX). */
  scrollX: number;
  /** Zoom: pixels per frame. Reference default 4.0. */
  pixelsPerFrame: number;
  /** Current playhead position in timeline frames. */
  playheadFrame: number;
  /** Selected clip IDs (selection persists across re-renders by ID — FR-9). */
  selectedClipIds: ReadonlySet<string>;
  /** Active time-range selection, if any. */
  rangeSelection?: TimelineRangeSelectionView | null;
}
