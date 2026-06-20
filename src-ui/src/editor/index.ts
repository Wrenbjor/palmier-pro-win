// Public API of the timeline editor module (E3-S9).
//
// The app shell mounts `<TimelineCanvas />`. It can drive it with an explicit
// `timeline` prop or with a `TimelineStore` (created via `createTimelineStore`).
// Until the `get_timeline` Tauri command lands (Epic 7), seed it from
// `makeFixtureTimeline()`.

export { TimelineCanvas, default as default } from "./TimelineCanvas";
export type { TimelineCanvasProps } from "./TimelineCanvas";

export { createTimelineStore, useTimelineStore } from "./store";
export type { TimelineStore, TimelineState } from "./store";

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
