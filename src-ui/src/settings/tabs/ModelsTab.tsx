// Models tab (E1-S9): per-model enable toggles via the model catalog + model prefs.
//
// The reference `ModelsPane` has a search field and Image/Video/Audio sections from
// `ModelCatalog`, each row a toggle via `ModelPreferences`. The catalog load
// (`/v1/models`) is async + 24h-cached and is a later integration (the boot spawns it
// non-blocking); the per-model prefs store is outside this subtree. This renders the
// reference layout with an empty catalog until that data lands — offline-safe (an empty
// catalog is the degraded state, never an error).
import { useState } from "react";

interface ModelEntry {
  id: string;
  name: string;
  enabled: boolean;
}

export default function ModelsTab() {
  const [query, setQuery] = useState("");
  // Catalog is empty until the async `/v1/models` load + prefs store land.
  const sections: { title: string; models: ModelEntry[] }[] = [
    { title: "Image", models: [] },
    { title: "Video", models: [] },
    { title: "Audio", models: [] },
  ];

  return (
    <div>
      <h1 className="mb-6 text-lg font-semibold">Models</h1>
      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search models"
        className="mb-6 w-full rounded-md border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none focus:border-[#F29933]"
      />
      {sections.map((s) => (
        <section key={s.title} className="mb-6">
          <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
            {s.title}
          </h2>
          {s.models.length === 0 ? (
            <p className="text-sm text-white/40">
              Model catalog loads when online.
            </p>
          ) : (
            <ul>
              {s.models
                .filter((m) => m.name.toLowerCase().includes(query.toLowerCase()))
                .map((m) => (
                  <li key={m.id} className="flex items-center justify-between border-b border-white/10 py-3">
                    <span className="text-sm">{m.name}</span>
                  </li>
                ))}
            </ul>
          )}
        </section>
      ))}
    </div>
  );
}
