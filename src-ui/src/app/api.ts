// Tauri command/event bridge for the app shell (E1-S4/E1-S9/E1-S10).
//
// Thin typed wrappers over `@tauri-apps/api` `invoke`/`listen` matching the Rust
// commands in `crates/palmier-tauri/src/commands.rs` + the events emitted by
// `menu.rs` / `update.rs`. Each call degrades gracefully outside a Tauri webview
// (plain `vite dev`) so the surfaces render in a browser for design work.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** True when running inside a Tauri webview (vs plain `vite dev` / a browser). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Booted prefs snapshot (mirrors Rust `SettingsSnapshot`). */
export interface SettingsSnapshot {
  notificationsEnabled: boolean;
  telemetryEnabled: boolean;
  mcpEnabled: boolean;
  hasSeenWelcome: boolean;
  /** Telemetry value snapshotted at launch — restart required when it differs. */
  telemetryEnabledForLaunch: boolean;
}

/** Account/credit snapshot (mirrors Rust `AccountSnapshot`). */
export interface AccountSnapshot {
  isMisconfigured: boolean;
  isLoading: boolean;
  isSignedIn: boolean;
  aiAllowed: boolean;
  tier: "none" | "pro" | "max";
  planLabel: string;
  remainingCredits: number;
  budgetCredits: number | null;
  email: string | null;
  name: string | null;
  lastError: string | null;
  topOffMin: number;
  topOffMax: number;
  topOffDefault: number;
}

/** MCP liveness (mirrors Rust `McpStatus`). */
export interface McpStatus {
  enabled: boolean;
  running: boolean;
  bind: string;
}

/** Update status pushed over the `update://status` event (mirrors Rust `UpdateEvent`). */
export interface UpdateStatus {
  available: boolean;
  version: string | null;
  enabled: boolean;
}

async function tryInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!inTauri()) return undefined;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`[api] invoke '${cmd}' failed:`, err);
    return undefined;
  }
}

// ── settings ────────────────────────────────────────────────────────────────
export const getSettings = () => tryInvoke<SettingsSnapshot>("get_settings");
export const setNotificationsEnabled = (enabled: boolean) =>
  tryInvoke<void>("set_notifications_enabled", { enabled });
export const setTelemetryEnabled = (enabled: boolean) =>
  tryInvoke<void>("set_telemetry_enabled", { enabled });
export const setMcpEnabled = (enabled: boolean) =>
  tryInvoke<void>("set_mcp_enabled", { enabled });
export const dismissWelcome = () => tryInvoke<void>("dismiss_welcome");

// ── account / agent ─────────────────────────────────────────────────────────
export const getAccount = () => tryInvoke<AccountSnapshot>("get_account");
export const hasAnthropicKey = () => tryInvoke<boolean>("has_anthropic_key");
export const saveAnthropicKey = (key: string) =>
  tryInvoke<void>("save_anthropic_key", { key });
export const deleteAnthropicKey = () => tryInvoke<void>("delete_anthropic_key");
export const getMcpStatus = () => tryInvoke<McpStatus>("get_mcp_status");

// ── windows ─────────────────────────────────────────────────────────────────
export const openSettings = () => tryInvoke<void>("open_settings");
export const openHelp = () => tryInvoke<void>("open_help");
export const openFeedback = () => tryInvoke<void>("open_feedback");
export const openProject = (projectId: string) =>
  tryInvoke<void>("open_project", { projectId });
export const showHome = () => tryInvoke<void>("show_home");

// ── project lifecycle (E1-S7) ─────────────────────────────────────────────────

/** A recent-project row (mirrors Rust `RecentProject`). */
export interface RecentProject {
  id: string;
  title: string;
  path: string;
  /** Last-opened time as Unix seconds (newest-first sort key). */
  lastOpened: number;
  accessible: boolean;
}

/** Recent projects, newest-first (registry `sorted_entries`). */
export const listRecent = () => tryInvoke<RecentProject[]>("list_recent");

/** New project: Rust opens the Save-As dialog. Returns the new id, or null on cancel. */
export const createProject = () =>
  tryInvoke<string | null>("create_project");

/** Open project: Rust opens the Open dialog. Returns the id, or null on cancel. */
export const openProjectDialog = () =>
  tryInvoke<string | null>("open_project_dialog");

/** Delete a project (trash bundle + drop the registry entry). */
export const deleteProject = (projectId: string) =>
  tryInvoke<void>("delete_project", { projectId });

// ── samples (E1-S8) ───────────────────────────────────────────────────────────

/** A sample-carousel card (mirrors Rust `SampleCard`). */
export interface SampleCard {
  slug: string;
  title: string;
  posterUrl: string | null;
}

/** Sample summaries; empty when offline / unconfigured (degrades, never errors). */
export const listSamples = () => tryInvoke<SampleCard[]>("list_samples");

/** Resolve + materialize + open a sample (download progress over `sample://progress`). */
export async function openSample(slug: string): Promise<{ ok: boolean; error?: string }> {
  if (!inTauri()) return { ok: false, error: "Not running in the app." };
  try {
    await invoke<void>("open_sample", { slug });
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

/** Sample download-progress event payload (mirrors Rust `SampleProgress`). */
export interface SampleProgress {
  slug: string;
  /** 0.0..=1.0. */
  progress: number;
}

/** Subscribe to sample download progress; returns an unlisten fn (no-op outside Tauri). */
export async function onSampleProgress(
  handler: (p: SampleProgress) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<SampleProgress>("sample://progress", (e) => handler(e.payload));
}

// ── feedback ────────────────────────────────────────────────────────────────
export interface FeedbackInput {
  message: string;
  mayContact: boolean;
  email?: string;
  screenshotPngBase64?: string;
}

/** Send feedback. Returns true on success; surfaces the error string on failure. */
export async function sendFeedback(input: FeedbackInput): Promise<{ ok: boolean; error?: string }> {
  if (!inTauri()) return { ok: false, error: "Not running in the app." };
  try {
    await invoke<void>("send_feedback", {
      message: input.message,
      mayContact: input.mayContact,
      email: input.email ?? null,
      screenshotPngBase64: input.screenshotPngBase64 ?? null,
    });
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

// ── updater ─────────────────────────────────────────────────────────────────
export const checkForUpdates = () => tryInvoke<void>("check_for_updates");

/** Subscribe to update-status events; returns an unlisten fn (no-op outside Tauri). */
export async function onUpdateStatus(
  handler: (status: UpdateStatus) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<UpdateStatus>("update://status", (e) => handler(e.payload));
}

/** Subscribe to the Help-tab-select event the menu emits; returns an unlisten fn. */
export async function onHelpSelectTab(
  handler: (tab: string) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<string>("help://select-tab", (e) => handler(e.payload));
}
