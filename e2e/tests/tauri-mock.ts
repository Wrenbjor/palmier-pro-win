// tauri-mock.ts — a seeded fake of the Tauri webview bridge for the UI smoke suite.
//
// WHY: the React UI calls Tauri commands via `@tauri-apps/api` `invoke`/`listen`,
// which dispatch to `window.__TAURI_INTERNALS__`. In plain `vite dev` that global is
// absent, so the UI degrades to empty/fixture states (inTauri() === false) — the
// surfaces render but with no data. To assert that each surface renders its REAL
// content (project browser, timeline canvas, agent panel, settings tabs), we install
// a complete `__TAURI_INTERNALS__` whose `invoke` returns realistic, hand-seeded
// payloads matching the Rust command shapes (commands.rs / agent.rs / preview).
//
// The payloads mirror these TS interfaces (src-ui/src/.../api.ts, controller.ts):
//   SettingsSnapshot, AccountSnapshot, RecentProject[], SampleCard[],
//   BackendStatus (agent_status), SessionSummaryWire[], get_timeline, get_media.
//
// Injected via page.addInitScript BEFORE the app bundle loads. `label` controls the
// surface (route.ts checks __TAURI_INTERNALS__.metadata.currentWindow.label first).

export type SurfaceLabel = "home" | "settings" | `project/${string}` | "help" | "feedback";

/** Build the init-script source string for a given window label. */
export function tauriMockScript(label: SurfaceLabel): string {
  // NOTE: this function body is serialized and run in the browser. Keep it
  // self-contained (no imports, no closures over Node values except `label`).
  const fn = (windowLabel: string) => {
    // ---- seeded backend payloads (camelCase wire shapes) --------------------
    const settings = {
      notificationsEnabled: true,
      telemetryEnabled: false,
      mcpEnabled: true,
      hasSeenWelcome: true, // skip the welcome overlay so the browser shows through
      telemetryEnabledForLaunch: false,
    };
    const account = {
      isMisconfigured: false,
      isLoading: false,
      isSignedIn: true,
      aiAllowed: true,
      tier: "pro",
      planLabel: "Pro",
      remainingCredits: 1200,
      budgetCredits: 2000,
      email: "tester@example.com",
      name: "Test User",
      lastError: null,
      topOffMin: 5,
      topOffMax: 100,
      topOffDefault: 20,
    };
    const recent = [
      {
        id: "proj-fixture-1",
        title: "Demo Reel",
        path: "C:/projects/demo-reel.palmier",
        lastOpened: 1_750_000_000,
        accessible: true,
      },
      {
        id: "proj-fixture-2",
        title: "Vacation Cut",
        path: "C:/projects/vacation.palmier",
        lastOpened: 1_749_000_000,
        accessible: true,
      },
    ];
    const samples = [
      { slug: "smpte-bars", title: "SMPTE Bars", posterUrl: null },
      { slug: "test-pattern", title: "Test Pattern", posterUrl: null },
    ];
    const agentStatus = {
      hasApiKey: true,
      isSignedIn: true,
      isPaid: true,
      hasCredits: true,
      paidCatalog: ["sonnet", "opus", "haiku"],
    };
    const sessions = [
      {
        id: "sess-1",
        title: "First session",
        updatedAt: 1_750_000_000,
        isOpen: true,
        isCurrent: true,
        messageCount: 1,
      },
    ];
    const sessionMessages = [
      {
        id: "msg-1",
        role: "assistant",
        blocks: [{ kind: "text", text: "Hi! I can edit your timeline." }],
      },
    ];
    const timeline = {
      fps: 30,
      width: 1280,
      height: 720,
      totalFrames: 150,
      canGenerate: true,
      tracks: [
        {
          label: "V1",
          clips: [
            {
              id: "clip-1",
              mediaRef: "asset-1",
              startFrame: 0,
              durationFrames: 120,
              mediaType: "video",
            },
          ],
        },
      ],
    };
    const media = {
      assets: [
        {
          id: "asset-1",
          name: "oracle-clip-5s.mp4",
          type: "video",
          duration: 5,
          generationStatus: "none",
        },
      ],
    };

    // ---- command router -----------------------------------------------------
    const handlers: Record<string, (args: any) => any> = {
      // app shell / settings
      get_settings: () => settings,
      get_account: () => account,
      has_anthropic_key: () => true,
      get_mcp_status: () => ({ enabled: true, running: true, bind: "127.0.0.1:19789" }),
      set_notifications_enabled: () => null,
      set_telemetry_enabled: () => null,
      set_mcp_enabled: () => null,
      dismiss_welcome: () => null,
      // project lifecycle
      list_recent: () => recent,
      list_samples: () => samples,
      create_project: () => null,
      open_project_dialog: () => null,
      delete_project: () => null,
      open_sample: () => null,
      // windows / nav (no-ops in the browser)
      open_settings: () => null,
      open_help: () => null,
      open_feedback: () => null,
      open_project: () => null,
      show_home: () => null,
      check_for_updates: () => null,
      send_feedback: () => null,
      // agent
      agent_status: () => agentStatus,
      agent_list_sessions: () => sessions,
      agent_get_session: () => sessionMessages,
      agent_new_session: () => "sess-2",
      agent_open_session: () => null,
      agent_close_session: () => null,
      agent_delete_session: () => null,
      agent_set_pref: () => null,
      agent_send: () => null,
      agent_cancel: () => null,
      // editor reads
      get_timeline: () => timeline,
      get_media: () => media,
      // preview (timeline viewport) — return benign values so it doesn't throw
      preview_init: () => "mock-adapter",
      preview_resize: () => null,
      preview_teardown: () => null,
      preview_set_timeline: () => null,
      preview_set_tab: () => 0,
      preview_seek: () => 0,
      preview_play: () => 0,
      preview_pause: () => 0,
      preview_toggle_playback: () => false,
      preview_step: () => 0,
      preview_apply_transform: () => null,
      preview_apply_crop: () => null,
      // media panel
      reveal_in_explorer: () => null,
      copy_paths_to_clipboard: () => null,
      pick_relink_path: () => null,
      read_clipboard_importable_paths: () => [],
      search_media: () => ({ visual: { status: "ready", moments: [] }, spoken: [] }),
    };

    // ---- callback registry (transformCallback / Channel support) ------------
    const callbacks = new Map<number, (payload: any) => void>();
    let nextCallbackId = 1;

    const invoke = (cmd: string, args: any) => {
      // Event plugin: return a fake listener id; never deliver events (the UI wraps
      // listen() in .catch and tolerates no events).
      if (cmd === "plugin:event|listen") return Promise.resolve(nextCallbackId++);
      if (cmd === "plugin:event|unlisten") return Promise.resolve();
      if (cmd === "plugin:event|emit") return Promise.resolve();
      if (cmd === "plugin:event|emit_to") return Promise.resolve();

      const h = handlers[cmd];
      if (h) {
        try {
          return Promise.resolve(h(args));
        } catch (e) {
          return Promise.reject(e);
        }
      }
      // Unknown command: resolve undefined (the typed wrappers tolerate this).
      // eslint-disable-next-line no-console
      console.debug("[tauri-mock] unhandled invoke:", cmd, args);
      return Promise.resolve(undefined);
    };

    (window as any).__TAURI_INTERNALS__ = {
      invoke,
      transformCallback(cb: (p: any) => void, _once?: boolean) {
        const id = nextCallbackId++;
        callbacks.set(id, cb);
        return id;
      },
      unregisterCallback(id: number) {
        callbacks.delete(id);
      },
      convertFileSrc(path: string) {
        return path;
      },
      metadata: {
        currentWindow: { label: windowLabel },
        currentWebview: { label: windowLabel, windowLabel },
      },
    };
    (window as any).isTauri = true;
  };

  return `(${fn.toString()})(${JSON.stringify(label)});`;
}
