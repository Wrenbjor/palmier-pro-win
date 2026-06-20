// Skeleton app shell. The real Home / Editor / Settings surfaces land per
// Epic 1 (epic-01-app-shell.md) under src/app, src/home, src/editor, etc.
import { useEffect } from "react";
import { registerMenuHandlers } from "./app/menu-events";

export default function App() {
  // E1-S3 — subscribe to the main-menu `menu://<id>` events. Until each owning
  // story lands its real handler, these are logged no-ops, which still proves
  // every menu binding is invokable (the event fires and is consumed here).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    registerMenuHandlers()
      .then((un) => {
        unlisten = un;
      })
      .catch((err) => {
        // Outside a Tauri webview (e.g. plain `vite dev`) the event API is
        // unavailable; that is fine for the skeleton shell.
        // eslint-disable-next-line no-console
        console.debug("[menu] handler registration skipped:", err);
      });
    return () => unlisten?.();
  }, []);

  return (
    <main className="flex min-h-screen items-center justify-center">
      <h1 className="text-2xl font-semibold">Palmier Pro — workspace skeleton</h1>
    </main>
  );
}
