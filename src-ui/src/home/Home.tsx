// Home window surface (E1-S4 shell, wired by E1-S7 + E1-S8).
//
// The project browser binds to the real `ProjectRegistry` via E1-S7's commands
// (`list_recent` / `create_project` / `open_project` / `delete_project`) and the
// sample carousel to E1-S8's `list_samples` / `open_sample` (with download
// progress). Both degrade gracefully outside a Tauri webview (plain `vite dev`)
// and when offline (empty lists, no error).
import { useCallback, useEffect, useState } from "react";
import {
  createProject,
  deleteProject,
  dismissWelcome,
  getSettings,
  listRecent,
  listSamples,
  onSampleProgress,
  openProject,
  openProjectDialog,
  openSample,
  openSettings,
  type RecentProject,
  type SampleCard,
} from "../app/api";
import { registerMenuHandlers } from "../app/menu-events";

/** Format a Unix-seconds timestamp as a short date label. */
function lastOpenedLabel(unixSeconds: number): string {
  if (!unixSeconds) return "";
  const d = new Date(unixSeconds * 1000);
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export default function Home() {
  const [showWelcome, setShowWelcome] = useState(false);
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [samples, setSamples] = useState<SampleCard[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // slug → download progress (0..1) while a sample is materializing.
  const [sampleProgress, setSampleProgress] = useState<Record<string, number>>({});

  const refreshRecents = useCallback(async () => {
    const r = await listRecent();
    if (r) setRecents(r);
  }, []);

  useEffect(() => {
    // Welcome overlay shows until `has_seen_welcome` is set (FR-1).
    getSettings().then((s) => {
      if (s && !s.hasSeenWelcome) setShowWelcome(true);
    });
    void refreshRecents();
    // Sample carousel: empty when offline / unconfigured (degrades, no error).
    listSamples().then((s) => {
      if (s) setSamples(s);
    });
  }, [refreshRecents]);

  // Subscribe to sample download progress.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onSampleProgress((p) => {
      setSampleProgress((prev) => ({ ...prev, [p.slug]: p.progress }));
    })
      .then((un) => {
        unlisten = un;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  // Wire File → New / File → Open menu items (E1-S3 emits `menu://new` /
  // `menu://open`) to the same create/open flow as the header buttons.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    registerMenuHandlers({
      new: () => {
        void (async () => {
          const id = await createProject();
          if (id) await refreshRecents();
        })();
      },
      open: () => {
        void (async () => {
          const id = await openProjectDialog();
          if (id) await refreshRecents();
        })();
      },
    })
      .then((un) => {
        unlisten = un;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, [refreshRecents]);

  function handleDismissWelcome() {
    setShowWelcome(false);
    void dismissWelcome();
  }

  async function handleNew() {
    setError(null);
    setBusy(true);
    try {
      // Rust opens the Save-As dialog; null = the user cancelled.
      const id = await createProject();
      if (id) await refreshRecents();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleOpenDialog() {
    setError(null);
    setBusy(true);
    try {
      const id = await openProjectDialog();
      if (id) await refreshRecents();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleOpenRecent(id: string) {
    setError(null);
    await openProject(id);
    await refreshRecents();
  }

  async function handleDelete(id: string, title: string) {
    setError(null);
    if (!confirm(`Move "${title}" to the Recycle Bin?`)) return;
    await deleteProject(id);
    await refreshRecents();
  }

  async function handleOpenSample(slug: string) {
    setError(null);
    setSampleProgress((prev) => ({ ...prev, [slug]: 0 }));
    const res = await openSample(slug);
    if (!res.ok) setError(res.error ?? "Failed to open sample.");
    setSampleProgress((prev) => {
      const next = { ...prev };
      delete next[slug];
      return next;
    });
  }

  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-white">
      <header className="flex items-center justify-between border-b border-white/10 px-8 py-5">
        <h1 className="text-xl font-semibold tracking-tight">Palmier Pro</h1>
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => void handleNew()}
            className="rounded-md bg-[#F29933] px-3 py-1.5 text-sm font-medium text-black hover:brightness-110 disabled:opacity-50"
          >
            New Project
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void handleOpenDialog()}
            className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/80 hover:bg-white/10 disabled:opacity-50"
          >
            Open…
          </button>
          <button
            type="button"
            onClick={() => void openSettings()}
            className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/80 hover:bg-white/10"
          >
            Settings
          </button>
        </div>
      </header>

      {error && (
        <div className="border-b border-red-500/30 bg-red-500/10 px-8 py-2 text-sm text-red-300">
          {error}
        </div>
      )}

      <main className="flex-1 overflow-auto px-8 py-6">
        <section className="mb-10">
          <h2 className="mb-3 text-sm font-medium uppercase tracking-wide text-white/50">
            Recent Projects
          </h2>
          {recents.length === 0 ? (
            <p className="text-sm text-white/40">
              No recent projects yet. Create one with New Project.
            </p>
          ) : (
            <ul className="grid grid-cols-2 gap-3 md:grid-cols-3">
              {recents.map((p) => (
                <li key={p.id} className="group relative">
                  <button
                    type="button"
                    disabled={!p.accessible}
                    onClick={() => void handleOpenRecent(p.id)}
                    title={p.path}
                    className="w-full rounded-lg border border-white/10 bg-white/5 p-4 text-left hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    <div className="font-medium">{p.title}</div>
                    <div className="text-xs text-white/40">
                      {p.accessible
                        ? lastOpenedLabel(p.lastOpened)
                        : "Missing on disk"}
                    </div>
                  </button>
                  <button
                    type="button"
                    aria-label={`Delete ${p.title}`}
                    onClick={() => void handleDelete(p.id, p.title)}
                    className="absolute right-2 top-2 hidden rounded px-1.5 py-0.5 text-xs text-white/50 hover:bg-white/10 hover:text-white group-hover:block"
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section>
          <h2 className="mb-3 text-sm font-medium uppercase tracking-wide text-white/50">
            Sample Projects
          </h2>
          {samples.length === 0 ? (
            <p className="text-sm text-white/40">
              Samples load from the catalog when online.
            </p>
          ) : (
            <div className="flex gap-4 overflow-x-auto pb-2">
              {samples.map((s) => {
                const progress = sampleProgress[s.slug];
                const loading = progress !== undefined;
                return (
                  <button
                    type="button"
                    key={s.slug}
                    disabled={loading}
                    onClick={() => void handleOpenSample(s.slug)}
                    className="w-56 shrink-0 rounded-lg border border-white/10 bg-white/5 p-3 text-left hover:bg-white/10 disabled:opacity-70"
                  >
                    <div
                      className="mb-2 h-28 rounded bg-white/10 bg-cover bg-center"
                      style={
                        s.posterUrl
                          ? { backgroundImage: `url(${s.posterUrl})` }
                          : undefined
                      }
                    />
                    <div className="text-sm font-medium">{s.title}</div>
                    {loading && (
                      <div className="mt-2 h-1.5 w-full overflow-hidden rounded bg-white/10">
                        <div
                          className="h-full bg-[#F29933] transition-all"
                          style={{ width: `${Math.round((progress ?? 0) * 100)}%` }}
                        />
                      </div>
                    )}
                  </button>
                );
              })}
            </div>
          )}
        </section>
      </main>

      {showWelcome && (
        <div className="absolute inset-0 flex items-center justify-center bg-black/70">
          <div className="w-[28rem] rounded-xl border border-white/10 bg-[#161616] p-8 text-center">
            <h2 className="mb-2 text-2xl font-semibold">Welcome to Palmier Pro</h2>
            <p className="mb-6 text-sm text-white/60">
              The AI-driven non-linear video editor. Create a project or open a sample
              to get started.
            </p>
            <button
              type="button"
              onClick={handleDismissWelcome}
              className="rounded-md bg-[#F29933] px-5 py-2 font-medium text-black hover:brightness-110"
            >
              Get Started
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
