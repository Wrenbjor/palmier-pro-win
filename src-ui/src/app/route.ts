// Window → surface routing (E1-S4).
//
// Every Palmier Pro window loads the same `index.html`; the Rust side
// (`crates/palmier-tauri/src/window.rs`) gives each window a stable **label** and a
// matching `#/<label>` hash route. This module resolves which surface to mount from
// (in priority order) the Tauri window label, then the URL hash — so it works both in
// the real app and in plain `vite dev` (where you can hit `/#/settings`).

export type Surface =
  | { kind: "home" }
  | { kind: "project"; projectId: string }
  | { kind: "settings" }
  | { kind: "help" }
  | { kind: "feedback" };

/** The Tauri window label for the current window, if running inside Tauri. */
function tauriLabel(): string | undefined {
  // `withGlobalTauri` exposes the current window's label synchronously.
  const internals = (window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
  }).__TAURI_INTERNALS__;
  return internals?.metadata?.currentWindow?.label;
}

/** Parse a label/route string ("settings", "project/abc", "help", …) into a Surface. */
function parse(route: string): Surface | undefined {
  const r = route.replace(/^#?\/?/, "");
  if (r === "" || r === "home") return { kind: "home" };
  if (r === "settings") return { kind: "settings" };
  if (r === "help") return { kind: "help" };
  if (r === "feedback") return { kind: "feedback" };
  if (r.startsWith("project/")) {
    return { kind: "project", projectId: r.slice("project/".length) };
  }
  return undefined;
}

/** Resolve the surface to mount for the current window. Defaults to Home. */
export function resolveSurface(): Surface {
  const label = tauriLabel();
  if (label) {
    const fromLabel = parse(label);
    if (fromLabel) return fromLabel;
  }
  const fromHash = parse(window.location.hash);
  if (fromHash) return fromHash;
  return { kind: "home" };
}
