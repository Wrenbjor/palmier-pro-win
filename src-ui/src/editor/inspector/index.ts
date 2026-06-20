// Public API of the Inspector module (E12-S2).
//
// The editor mounts `<InspectorPanel input={…} controller={…} />` in the right
// rail. It is a PURE VIEW over reactive state: header (Timeline/Inspector/Source),
// gated clip tab-bar, marquee summary, and no-selection Project/Format metadata.
//
// SEAM for the sibling stories (E12-S3..S9): pass `tabBodies` (a `TabBodyRenderer`)
// to fill the Video/Audio/Text tab content (E12-S5/S6/S7), and `assetBody` (an
// `AssetBodyRenderer`) to fill the "Source" media-asset inspector (E12-S9). Each
// receives the live `InspectorInput` + the resolved `InspectorState` + (for
// tab bodies) the active `ClipTab`. The shell already resolves which tab/mode is
// active and only calls the renderer for that one — siblings render their disjoint
// body and never touch the shell's gating.

export { InspectorPanel } from "./InspectorPanel";
export type {
  InspectorPanelProps,
  TabBodyRenderer,
  AssetBodyRenderer,
} from "./InspectorPanel";

export { InspectorController, MOCK_ACCOUNT, accountStateFromSnapshot } from "./controller";

// Pure resolution logic — exported so sibling stories + tests can reuse it.
export {
  resolveInspector,
  resolveHeader,
  availableTabs,
  aiEditEligible,
  activeTab,
  resolvePreferredTab,
  shouldClearCropEditing,
  selectedVisualClips,
  selectedAudioClips,
  nonTextVisualClips,
  selectedMediaAsset,
  resolvedClipAsset,
  linkedPartnerIds,
  isVisual,
  resolveProject,
  resolveFormat,
  totalFrames,
  formatDuration,
  formatAspectRatio,
  fileStem,
  middleTruncate,
  gcd,
} from "./logic";

export { CLIP_TAB_LABEL } from "./types";
export type {
  ClipTab,
  InspectorTitle,
  InspectorIcon,
  InspectorInput,
  InspectorState,
  ResolvedHeader,
  NoSelectionPanel,
  ProjectMetadata,
  FormatMetadata,
  MediaAssetView,
  AccountState,
} from "./types";

export { Theme as InspectorTheme, Spacing as InspectorSpacing } from "./theme";

export { runInspectorParityChecks } from "./parity.checks";
