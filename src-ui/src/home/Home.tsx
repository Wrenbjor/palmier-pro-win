// Home window surface (E1-S4): project browser + Recent + Sample carousel + Welcome
// overlay. The browser binds to E1-S7's registry and the carousel to E1-S8's sample
// list via a defined props/event contract; until those land, the data sources are
// stubbed (empty), so this surface renders and behaves independently.
import { useEffect, useState } from "react";
import {
  dismissWelcome,
  getSettings,
  openProject,
  openSettings,
} from "../app/api";

/** A recent-project entry (mirrors E1-S7's `ProjectEntry`; stubbed empty for now). */
export interface RecentProject {
  id: string;
  title: string;
  lastOpened: string;
}

/** A sample-carousel entry (mirrors E1-S8's `Summary`; stubbed empty for now). */
export interface SampleSummary {
  slug: string;
  title: string;
  posterUrl?: string;
}

export default function Home() {
  const [showWelcome, setShowWelcome] = useState(false);
  // E1-S7 / E1-S8 feed these; empty until their commands land.
  const [recents] = useState<RecentProject[]>([]);
  const [samples] = useState<SampleSummary[]>([]);

  useEffect(() => {
    // Welcome overlay shows until `has_seen_welcome` is set (FR-1).
    getSettings().then((s) => {
      if (s && !s.hasSeenWelcome) setShowWelcome(true);
    });
  }, []);

  function handleDismissWelcome() {
    setShowWelcome(false);
    void dismissWelcome();
  }

  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-white">
      <header className="flex items-center justify-between border-b border-white/10 px-8 py-5">
        <h1 className="text-xl font-semibold tracking-tight">Palmier Pro</h1>
        <button
          type="button"
          onClick={() => void openSettings()}
          className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/80 hover:bg-white/10"
        >
          Settings
        </button>
      </header>

      <main className="flex-1 overflow-auto px-8 py-6">
        <section className="mb-10">
          <h2 className="mb-3 text-sm font-medium uppercase tracking-wide text-white/50">
            Recent Projects
          </h2>
          {recents.length === 0 ? (
            <p className="text-sm text-white/40">
              No recent projects yet. Create one from File &rarr; New.
            </p>
          ) : (
            <ul className="grid grid-cols-2 gap-3 md:grid-cols-3">
              {recents.map((p) => (
                <li key={p.id}>
                  <button
                    type="button"
                    onClick={() => void openProject(p.id)}
                    className="w-full rounded-lg border border-white/10 bg-white/5 p-4 text-left hover:bg-white/10"
                  >
                    <div className="font-medium">{p.title}</div>
                    <div className="text-xs text-white/40">{p.lastOpened}</div>
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
              {samples.map((s) => (
                <div
                  key={s.slug}
                  className="w-56 shrink-0 rounded-lg border border-white/10 bg-white/5 p-3"
                >
                  <div className="mb-2 h-28 rounded bg-white/10" />
                  <div className="text-sm font-medium">{s.title}</div>
                </div>
              ))}
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
