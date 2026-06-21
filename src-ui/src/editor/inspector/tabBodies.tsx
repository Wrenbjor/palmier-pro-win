// The concrete TabBodyRenderer + AssetBodyRenderer (E12-S5..S9 wiring).
//
// These replace the shell's default labelled placeholders. `InspectorPanel` resolves
// which tab/mode is active and calls the renderer for THAT one; here we select the
// matching tab body (Video/Audio/Text/Details/AI-Edit) and feed it the selected
// clips + an `editorEdit` dispatch. The shell never sees the body internals.
//
// Usage (in the app shell — wiring is out of THIS story's file scope):
//   <InspectorPanel input={input} controller={ctrl}
//       tabBodies={makeTabBodies({ activeFrame, onSeek })}
//       assetBody={makeAssetBody()} />

import type { JSX, ReactNode } from "react";
import { editorEdit } from "../bridge";
import type { TabBodyRenderer, AssetBodyRenderer } from "./InspectorPanel";
import {
  nonTextVisualClips,
  selectedAudioClips,
  selectedVisualClips,
} from "./logic";
import type { ClipTab, InspectorInput } from "./types";
import { AudioTab } from "./AudioTab";
import { VideoTab, type EditDispatch } from "./VideoTab";
import { TextTab } from "./TextTab";
import { DetailsTab, AIEditTab, type AssetDetail } from "./DetailsTab";

export interface TabBodiesOptions {
  /** Playhead frame for keyframe-aware seeding. */
  activeFrame?: number;
  /** Crop editing state (shell-owned). */
  cropEditingActive?: boolean;
  onToggleCropEditing?: (active: boolean) => void;
  /** Seek for the keyframes panel chevrons. */
  onSeek?: (absoluteFrame: number) => void;
  /** Override the mutating dispatch (defaults to `editorEdit`). */
  edit?: EditDispatch;
  /** Richer asset details for the Details tab (until the media view-model carries them). */
  assetDetail?: (assetId: string) => AssetDetail | undefined;
}

const defaultEdit: EditDispatch = (name, args) => editorEdit(name, args);

/** Build the `TabBodyRenderer` to pass to `InspectorPanel.tabBodies`. */
export function makeTabBodies(opts: TabBodiesOptions = {}): TabBodyRenderer {
  const edit = opts.edit ?? defaultEdit;
  return ({ tab, input, state }) => renderTabBody(tab, input, state.activeTab, opts, edit);
}

function renderTabBody(
  tab: ClipTab,
  input: InspectorInput,
  _activeTab: ClipTab | null,
  opts: TabBodiesOptions,
  edit: EditDispatch,
): ReactNode {
  const canvasWidth = input.timeline?.width ?? 1920;
  const canvasHeight = input.timeline?.height ?? 1080;
  const fps = input.timeline?.fps ?? 30;
  const activeFrame = opts.activeFrame ?? input.activeFrame;

  switch (tab) {
    case "video": {
      const clips = nonTextVisualClips(input);
      if (clips.length === 0) return null;
      return (
        <VideoTab
          clips={clips}
          canvasWidth={canvasWidth}
          canvasHeight={canvasHeight}
          activeFrame={activeFrame}
          cropEditingActive={opts.cropEditingActive}
          onToggleCropEditing={opts.onToggleCropEditing}
          edit={edit}
        />
      );
    }
    case "audio": {
      const clips = selectedAudioClips(input);
      if (clips.length === 0) return null;
      return (
        <AudioTab
          clips={clips}
          hasVisualSelected={selectedVisualClips(input).length > 0}
          fps={fps}
          activeFrame={activeFrame}
          edit={edit}
        />
      );
    }
    case "text": {
      const clip = selectedVisualClips(input).find((c) => c.mediaType === "text");
      if (!clip) return null;
      return (
        <TextTab
          clip={clip}
          canvasWidth={canvasWidth}
          canvasHeight={canvasHeight}
          edit={edit}
        />
      );
    }
    case "ai": {
      // AI-Edit tab for an AI-eligible visual clip.
      return (
        <AIEditTab
          isMisconfigured={input.account.isMisconfigured}
          hasClipContext={selectedVisualClips(input).length > 0}
          actions={[]}
        />
      );
    }
  }
}

/** Build the `AssetBodyRenderer` (the media-asset "Source" inspector). */
export function makeAssetBody(opts: TabBodiesOptions = {}): AssetBodyRenderer {
  return ({ input }): JSX.Element => {
    const id = [...input.selectedMediaAssetIds][0];
    const base = input.mediaAssets.find((a) => a.id === id);
    const detail: AssetDetail = opts.assetDetail?.(id ?? "") ?? {
      id: id ?? "",
      isVisual: base?.isVisual ?? false,
    };
    return <DetailsTab asset={detail} />;
  };
}
