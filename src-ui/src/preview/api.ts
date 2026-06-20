// Tauri command/event bridge for the preview viewport (E5-S10).
//
// Thin typed wrappers over the `preview_*` commands in
// `crates/palmier-tauri/src/preview.rs` and the events that module emits. The viewport
// drives the engine transport ONLY through these commands (FOUNDATION §4 strict
// layering — the webview never touches `palmier-engine` directly); the engine streams
// the reactive playhead back as the `preview://current-frame` event.
//
// Every call degrades gracefully outside a Tauri webview (plain `vite dev`) so the
// panel + overlays render in a browser for design work — the transport just becomes a
// no-op and the playhead is driven locally.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { Crop, Transform } from "./types";

/** True when running inside a Tauri webview (vs plain `vite dev`). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Seek semantics (mirrors Rust `SeekMode`). */
export type SeekMode = "exact" | "interactiveScrub";

/** The `preview://current-frame` payload (mirrors Rust `CurrentFramePayload`). */
export interface CurrentFramePayload {
  frame: number;
  isTimeline: boolean;
}

/** Event names emitted by `preview.rs`. */
export const CURRENT_FRAME_EVENT = "preview://current-frame";
export const PLAYBACK_STATE_EVENT = "preview://playback-state";

async function tryInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!inTauri()) return undefined;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`[preview] invoke '${cmd}' failed:`, err);
    return undefined;
  }
}

// ── surface lifecycle (built on E5-S8's preview.rs) ───────────────────────────

/** Initialize the wgpu present surface for `windowLabel` (returns the adapter summary). */
export const previewInit = (windowLabel: string, forceDx12?: boolean) =>
  tryInvoke<string>("preview_init", { windowLabel, forceDx12: forceDx12 ?? null });

/** Resize the present surface (window/viewport resize). */
export const previewResize = (width: number, height: number) =>
  tryInvoke<void>("preview_resize", { width, height });

/** Tear down the present surface (window closing). */
export const previewTeardown = () => tryInvoke<void>("preview_teardown");

// ── transport (E5-S10 wiring — drives the engine Transport) ───────────────────

/**
 * Push the project timeline the transport composes from. Until the `get_timeline`
 * command lands (Epic 7) the frontend owns the timeline view-model and hands the
 * engine a serialized `palmier-model::Timeline`. Sending it (re)builds the transport's
 * source resolver from the embedded source sizes.
 */
export const previewSetTimeline = (timeline: unknown) =>
  tryInvoke<void>("preview_set_timeline", { timeline });

/** Start playback (engine `Transport::play`). */
export const previewPlay = () => tryInvoke<number>("preview_play");

/** Pause playback (engine `Transport::pause`). */
export const previewPause = () => tryInvoke<number>("preview_pause");

/** Toggle play/pause. Returns the resulting playing flag. */
export const previewTogglePlayback = () => tryInvoke<boolean>("preview_toggle_playback");

/** Seek to `frame` under `mode` (engine `Transport::seek`). Returns the landed frame. */
export const previewSeek = (frame: number, mode: SeekMode) =>
  tryInvoke<number>("preview_seek", { frame, mode });

/** Step the playhead by `delta` frames (exact). Returns the new frame. */
export const previewStep = (delta: number) => tryInvoke<number>("preview_step", { delta });

/** Activate a preview tab by id (engine `Transport::activate_tab`). Returns the restored frame. */
export const previewSetTab = (tabId: string) => tryInvoke<number>("preview_set_tab", { tabId });

/** Commit a transform edit for a clip (flows into the edit engine). */
export const previewApplyTransform = (clipId: string, transform: Transform) =>
  tryInvoke<void>("preview_apply_transform", { clipId, transform });

/** Commit a crop edit for a clip. */
export const previewApplyCrop = (clipId: string, crop: Crop) =>
  tryInvoke<void>("preview_apply_crop", { clipId, crop });

// ── events ────────────────────────────────────────────────────────────────────

/** Subscribe to the reactive playhead; returns an unlisten fn (no-op outside Tauri). */
export async function onCurrentFrame(handler: (p: CurrentFramePayload) => void): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<CurrentFramePayload>(CURRENT_FRAME_EVENT, (e) => handler(e.payload));
}

/** Subscribe to play/pause state changes; returns an unlisten fn. */
export async function onPlaybackState(handler: (playing: boolean) => void): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<boolean>(PLAYBACK_STATE_EVENT, (e) => handler(e.payload));
}
