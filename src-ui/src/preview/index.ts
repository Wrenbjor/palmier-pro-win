// Public API of the preview viewport module (E5-S10).
//
// The project window mounts `<PreviewPanel />`. It composes the preview tabs, the
// transparent viewport (the native wgpu surface from E5-S1 shows through), the
// transform/crop overlays, the transport controls, and the aspect/quality/zoom menu.
// Transport + edits flow through the `preview_*` Tauri commands (see `api.ts`) into
// `palmier-engine`; the reactive `current_frame` streams back over Tauri events.

export { PreviewPanel } from "./PreviewPanel";
export type { PreviewPanelProps } from "./PreviewPanel";

export { PreviewTabs } from "./PreviewTabs";
export type { PreviewTabsProps } from "./PreviewTabs";
export { TransportControls } from "./TransportControls";
export type { TransportControlsProps } from "./TransportControls";
export { SettingsMenu } from "./SettingsMenu";
export type { SettingsMenuProps } from "./SettingsMenu";
export { TransformOverlay } from "./TransformOverlay";
export type { TransformOverlayProps } from "./TransformOverlay";
export { CropOverlay } from "./CropOverlay";
export type { CropOverlayProps } from "./CropOverlay";

export { createPreviewStore, usePreviewStore, TIMELINE_ID } from "./store";
export type { PreviewStore, PreviewState } from "./store";

export * from "./types";
export * from "./presets";
export {
  videoContentRect,
  fitSize,
  clipFrame,
  cropFrame,
  movedTransform,
  resizedTransform,
  rotatedHitFrame,
  clipLocalDelta,
  pannedCrop,
  resizedCrop,
  zoomAboutPoint,
  scrubFrame,
  formatTimecode,
  SNAP_THRESHOLD_PX,
  MIN_NORMALIZED,
} from "./geometry";
export type { Rect } from "./geometry";

export {
  inTauri,
  previewInit,
  previewResize,
  previewTeardown,
  previewSetTimeline,
  previewPlay,
  previewPause,
  previewTogglePlayback,
  previewSeek,
  previewStep,
  previewSetTab,
  previewRenderFrame,
  previewApplyTransform,
  previewApplyCrop,
  onCurrentFrame,
  onPlaybackState,
  CURRENT_FRAME_EVENT,
  PLAYBACK_STATE_EVENT,
} from "./api";
export type { SeekMode, CurrentFramePayload, PreviewFrameData } from "./api";
