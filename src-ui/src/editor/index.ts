// Public API of the timeline editor module (E3-S9).
//
// The app shell mounts `<TimelineCanvas />`. It can drive it with an explicit
// `timeline` prop or with a `TimelineStore` (created via `createTimelineStore`).
// Until the `get_timeline` Tauri command lands (Epic 7), seed it from
// `makeFixtureTimeline()`.

export { TimelineCanvas } from "./TimelineCanvas";
export type { TimelineCanvasProps } from "./TimelineCanvas";

// E3-S10 — the wired interactive editor (the app shell mounts this).
export { TimelineEditor, default as default } from "./TimelineEditor";
export type { TimelineEditorProps, ToolMode } from "./TimelineEditor";

export { createTimelineStore, useTimelineStore } from "./store";
export type { TimelineStore, TimelineState } from "./store";

// E3-S10 — command seam + local optimistic edit layer + undo (replaced by Tauri at E7).
export { EditController } from "./controller";
export { TimelineHistory } from "./history";
export {
  applyEdit,
  mergeRanges,
  computeRippleShiftsForRanges,
  computeOverwrite,
  splitClip,
  trimClamp,
  isCompatible,
  hasNoSourceMedia,
  localUuid,
} from "./apply";
export type { OverwriteAction, TrimClamp } from "./apply";
export type { EditIntent, EditOrigin, FrameRange, ClipShift } from "./edit-types";
export { rangeLength, rangeContains } from "./edit-types";
export {
  collectTargets,
  findSnap,
  makeSnapState,
} from "./snap";
export type { SnapTarget, SnapResult, SnapState } from "./snap";
export {
  subModeForLocalX,
  hitTestClip,
  moveProbeOffsets,
  clampFrameDelta,
  clampedTrackDelta,
  pinnedCompanions,
  marqueeRect,
  marqueeSelect,
  expandToLinkGroup,
} from "./drag";
export type { DragState, Modifiers, SubMode } from "./drag";

export { makeFixtureTimeline } from "./fixture";

export type {
  TimelineView,
  TrackView,
  ClipView,
  ClipType,
  Interpolation,
  KeyframeView,
  KeyframeTrackView,
  TimelineViewport,
  TimelineRangeSelectionView,
} from "./types";

// Pure geometry/sampling — exported so E3-S7/S10 (and tests) can reuse them.
export {
  makeLayout,
  clipRect,
  frameAt,
  xForFrame,
  trackAt,
  trackY,
  dropTargetAt,
  tickInterval,
  minorSubdivisions,
  formatTimecode,
  sampleTrack,
  volumeAt,
  opacityAt,
  fadeMultiplier,
  sourceFramesConsumed,
  sourceDurationFrames,
  endFrame,
  roundTiesAway,
} from "./geometry";
export type { Rect, TimelineLayout, TrackDropTarget } from "./geometry";

export { renderTimeline } from "./renderer";
export type { RenderArgs } from "./renderer";
