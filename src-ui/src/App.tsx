// App-shell window router (E1-S4). Every Palmier Pro window loads this same bundle; the
// surface to mount is resolved from the window label / hash (`app/route.ts`):
//   home → Home (project browser)         project/<id> → Project (editor shell)
//   settings → Settings (5 tabs)          help → Help (Shortcuts + MCP)
//   feedback → Feedback dialog
//
// The main-menu `menu://<id>` event listeners (E1-S3) are registered here for windows
// that consume editor-action events (Home/Project); the window-opening Help/Settings/
// Feedback items are handled natively in Rust, so those surfaces don't need listeners.
import { useEffect, useMemo } from "react";
import { registerMenuHandlers } from "./app/menu-events";
import { resolveSurface } from "./app/route";
import Home from "./home/Home";
import Project from "./app/Project";
import Settings from "./settings/Settings";
import Help from "./settings/Help";
import Feedback from "./settings/Feedback";

export default function App() {
  const surface = useMemo(() => resolveSurface(), []);

  // Subscribe to the main-menu events on the Project (editor) surface. The Home
  // surface registers its own menu overrides (File → New / Open) in `Home.tsx`, so
  // it is excluded here to avoid a double subscription firing the New/Open dialog
  // twice.
  useEffect(() => {
    if (surface.kind !== "project") return;
    let unlisten: (() => void) | undefined;
    registerMenuHandlers()
      .then((un) => {
        unlisten = un;
      })
      .catch((err) => {
        // Outside a Tauri webview (plain `vite dev`) the event API is unavailable.
        // eslint-disable-next-line no-console
        console.debug("[menu] handler registration skipped:", err);
      });
    return () => unlisten?.();
  }, [surface.kind]);

  switch (surface.kind) {
    case "settings":
      return <Settings />;
    case "help":
      return <Help />;
    case "feedback":
      return <Feedback />;
    case "project":
      return <Project projectId={surface.projectId} />;
    case "home":
    default:
      return <Home />;
  }
}
