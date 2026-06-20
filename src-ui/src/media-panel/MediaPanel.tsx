// MediaPanel — the left-dock panel shell (E4-S8): a 3-icon vertical rail switching
// Media / Captions / Music, backed by the reactive store + controller, wired to the
// backend→UI reveal events. The app shell mounts `<MediaPanel />`.
//
// Self-contained: if no `store`/`controller` prop is passed it creates its own from
// the fixture, so the panel is mountable in isolation (and in `vite dev` outside a
// Tauri webview). The reveal events degrade gracefully when the Tauri event API is
// unavailable (plain `vite dev`), mirroring App.tsx's `registerMenuHandlers`.

import { useEffect, useMemo, useRef } from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme } from "./theme";
import { createMediaPanelStore, useMediaStore, type MediaPanelStore } from "./store";
import { MediaPanelController } from "./controller";
import { MediaTab } from "./MediaTab";
import { CaptionsTab } from "./CaptionsTab";
import { MusicTab } from "./MusicTab";
import { makeFixtureJobs, makeFixtureSnapshot } from "./fixture";
import { registerRevealHandlers } from "./reveal-events";
import type { PanelTab } from "./types";

export interface MediaPanelProps {
  /** Inject a store (e.g. shared with the editor); otherwise one is created. */
  store?: MediaPanelStore;
  /** Inject a controller; otherwise one is created bound to the store. */
  controller?: MediaPanelController;
  /**
   * Seed from the fixture when self-creating a store (default true). Set false to
   * start empty (the real `get_media` command will populate it at Epic 7).
   */
  seedFixture?: boolean;
}

const TABS: { tab: PanelTab; label: string; icon: string }[] = [
  { tab: "media", label: "Media", icon: "▦" },
  { tab: "captions", label: "Captions", icon: "💬" },
  { tab: "music", label: "Music", icon: "♪" },
];

export function MediaPanel({ store, controller, seedFixture = true }: MediaPanelProps) {
  // Create a self-contained store/controller once if not injected.
  const owned = useRef<{ store: MediaPanelStore; controller: MediaPanelController } | null>(
    null,
  );
  const { store: theStore, controller: theController } = useMemo(() => {
    if (store && controller) return { store, controller };
    if (store && !controller)
      return { store, controller: new MediaPanelController(store) };
    if (!owned.current) {
      const s = createMediaPanelStore(
        seedFixture
          ? { snapshot: makeFixtureSnapshot(), jobs: makeFixtureJobs() }
          : undefined,
      );
      owned.current = { store: s, controller: new MediaPanelController(s) };
    }
    return owned.current;
  }, [store, controller, seedFixture]);

  const tab = useMediaStore(theStore, (s) => s.tab);

  // Wire backend→UI reveal events (Tauri). Degrades to no-op outside a webview.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/event")
      .then((api) =>
        registerRevealHandlers(api, theStore, {
          onPasteRequest: () => {
            // TODO(E7): read clipboard + theController.importPaths(paths).
          },
        }),
      )
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      })
      .catch((err) => {
        // eslint-disable-next-line no-console
        console.debug("[media-panel] reveal handlers skipped:", err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [theStore, theController]);

  // Panel-level keyboard shortcuts (Ctrl+Shift+N new folder, Ctrl+Up navigate up).
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (theStore.getState().tab !== "media") return;
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "n") {
      e.preventDefault();
      theController.createFolder();
    } else if ((e.ctrlKey || e.metaKey) && e.key === "ArrowUp") {
      e.preventDefault();
      theController.navigateUp();
    }
  };

  return (
    <div style={panelStyle} onKeyDown={onKeyDown}>
      {/* rail */}
      <div style={railStyle}>
        {TABS.map((t) => (
          <button
            key={t.tab}
            title={t.label}
            aria-pressed={tab === t.tab}
            onClick={() => theStore.setTab(t.tab)}
            style={{
              ...railButtonStyle,
              color: tab === t.tab ? "#000" : Theme.text.secondary,
              background: tab === t.tab ? Theme.accent : "transparent",
            }}
          >
            <span style={{ fontSize: 16 }}>{t.icon}</span>
            <span style={{ fontSize: 9 }}>{t.label}</span>
          </button>
        ))}
      </div>

      {/* tab body */}
      <div style={{ flex: 1, minWidth: 0, minHeight: 0 }}>
        {tab === "media" && <MediaTab store={theStore} controller={theController} />}
        {tab === "captions" && <CaptionsTab />}
        {tab === "music" && <MusicTab />}
      </div>
    </div>
  );
}

const panelStyle: CSSProperties = {
  display: "flex",
  height: "100%",
  minHeight: 0,
  background: Theme.background.base,
  color: Theme.text.primary,
  fontFamily:
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
  borderRight: `1px solid ${Theme.border.subtle}`,
};

const railStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.xs,
  padding: Spacing.xs,
  background: Theme.background.surface,
  borderRight: `1px solid ${Theme.border.subtle}`,
};

const railButtonStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  gap: 2,
  width: 52,
  padding: "8px 4px",
  borderRadius: 8,
  border: "none",
  cursor: "pointer",
};
