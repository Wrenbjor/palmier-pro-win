// App-shell window router (E1-S4). Every Palmier Pro window loads this same bundle; the
// surface to mount is resolved from the window label / hash (`app/route.ts`):
//   home → Home (project browser)         project/<id> → Project (editor shell)
//   settings → Settings (5 tabs)          help → Help (Shortcuts + MCP)
//   feedback → Feedback dialog
//
// The main-menu `menu://<id>` event listeners (E1-S3) are registered by each surface
// that consumes editor-action events: Home registers File → New/Open in `Home.tsx`,
// and Project registers the Edit menu + File → Save in `Project.tsx` (where the live
// `EditController` + selection/playhead state are available). The window-opening Help/
// Settings/Feedback items are handled natively in Rust, so those surfaces need no
// listeners — and App registers none, to avoid double-subscribing a `menu://` family.
import { useMemo } from "react";
import { resolveSurface } from "./app/route";
import Home from "./home/Home";
import Project from "./app/Project";
import Settings from "./settings/Settings";
import Help from "./settings/Help";
import Feedback from "./settings/Feedback";

export default function App() {
  const surface = useMemo(() => resolveSurface(), []);

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
