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
import type { AccountState, ClipTab, InspectorInput, MediaAssetView } from "./types";
import { AudioTab } from "./AudioTab";
import { VideoTab, type EditDispatch } from "./VideoTab";
import { TextTab } from "./TextTab";
import { DetailsTab, AIEditTab, type AIAction, type AssetDetail } from "./DetailsTab";

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
  /**
   * Override the per-asset Details with a richer source (rare — the inspector
   * `MediaAssetView` now carries the real fields, so the default derives the detail
   * straight from the selected asset). Kept as an escape hatch.
   */
  assetDetail?: (assetId: string) => AssetDetail | undefined;
}

const defaultEdit: EditDispatch = (name, args) => editorEdit(name, args);

/**
 * Whether AI editing is RUNNABLE for this account: it must be configured (Convex /
 * Clerk wired), signed in, and AI-allowed (has credits). When any is false the AI-Edit
 * tab renders its gated sign-in notice rather than dead buttons.
 */
function aiAvailable(account: AccountState): boolean {
  if (account.isMisconfigured) return false;
  // `aiAllowed`/`isSignedIn` are optional on the view-model; when present they gate.
  if (account.aiAllowed === false) return false;
  if (account.isSignedIn === false) return false;
  return true;
}

/** The gate notice for the AI-Edit tab when AI editing is not available. */
function aiGateNotice(account: AccountState): string {
  if (account.isMisconfigured) {
    return "AI editing is unavailable — sign in to Palmier and configure your account.";
  }
  return "Sign in to a plan to use AI editing.";
}

/** Build the inspector `AssetDetail` straight from the selected media-asset view. */
function assetDetailFromView(
  view: MediaAssetView | undefined,
  id: string,
): AssetDetail {
  if (!view) return { id, isVisual: false };
  return {
    id: view.id,
    isVisual: view.isVisual,
    name: view.name,
    type: view.type,
    width: view.width,
    height: view.height,
    durationSeconds:
      view.durationSeconds == null ? undefined : view.durationSeconds,
    sizeBytes: view.sizeBytes,
    path: view.path,
    isGenerated: view.isGenerated,
    generatedModel: view.generatedModel,
    generatedAspect: view.generatedAspect,
    generatedResolution: view.generatedResolution,
    prompt: view.prompt,
  };
}


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
      // AI-Edit tab for an AI-eligible visual clip. GATED: when AI editing is not
      // available (account misconfigured / not signed in / no credits) the tab shows
      // the sign-in notice instead of dead buttons. When available, the actions route
      // through `editorEdit('upscale_media' | 'generate_*', …)`.
      const available = aiAvailable(input.account);
      const clips = selectedVisualClips(input);
      const actions: AIAction[] = available
        ? clips.map((c) => ({
            id: `upscale-${c.id}`,
            label: "Upscale clip source",
            onRun: () => void edit("upscale_media", { mediaRef: c.mediaRef }),
          }))
        : [];
      return (
        <AIEditTab
          isMisconfigured={input.account.isMisconfigured}
          aiAvailable={available}
          gateNotice={aiGateNotice(input.account)}
          hasClipContext={clips.length > 0}
          actions={actions}
        />
      );
    }
  }
}

/** Build the `AssetBodyRenderer` (the media-asset "Source" inspector). */
export function makeAssetBody(opts: TabBodiesOptions = {}): AssetBodyRenderer {
  return ({ input }): JSX.Element => {
    const id = [...input.selectedMediaAssetIds][0] ?? "";
    const view = input.mediaAssets.find((a) => a.id === id);
    // Prefer an explicit override (escape hatch); else derive the full detail straight
    // from the enriched media-asset view (real type / dimensions / duration / size /
    // path + generated metadata flow through `Project.tsx` → `inspectorInput`).
    const detail = opts.assetDetail?.(id) ?? assetDetailFromView(view, id);
    return <DetailsTab asset={detail} />;
  };
}
