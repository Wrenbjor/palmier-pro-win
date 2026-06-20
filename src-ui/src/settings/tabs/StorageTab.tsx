// Storage tab (E1-S9): cache / index / model byte sizes + clear buttons.
//
// The reference `StoragePane` shows a Cache row (sum of image/video + visual caches,
// path shown `~`-relativized, "Clear cache") and a Media-search section (toggle, index
// bytes, model bytes, clear/remove). The DiskCache / EmbeddingStore / ModelDownloader
// sizes come from later epics (search/gen); this renders the reference layout with
// zeroed sizes until those commands land.
function fmtBytes(n: number): string {
  if (n === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(n) / Math.log(1024));
  return `${(n / 1024 ** i).toFixed(1)} ${units[i]}`;
}

export default function StorageTab() {
  // Zeroed until the DiskCache/EmbeddingStore/ModelDownloader size commands land.
  const cacheBytes = 0;
  const indexBytes = 0;
  const modelBytes = 0;

  return (
    <div>
      <h1 className="mb-6 text-lg font-semibold">Storage</h1>

      <section className="mb-8">
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          Cache
        </h2>
        <div className="flex items-center justify-between border-b border-white/10 py-4">
          <span className="text-sm">Media &amp; preview cache</span>
          <div className="flex items-center gap-3">
            <span className="text-sm text-white/50">{fmtBytes(cacheBytes)}</span>
            <button
              type="button"
              className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/70 hover:bg-white/10"
            >
              Clear cache
            </button>
          </div>
        </div>
      </section>

      <section>
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          Media search
        </h2>
        <div className="flex items-center justify-between border-b border-white/10 py-4">
          <span className="text-sm">Search index</span>
          <div className="flex items-center gap-3">
            <span className="text-sm text-white/50">{fmtBytes(indexBytes)}</span>
            <button
              type="button"
              className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/70 hover:bg-white/10"
            >
              Clear index
            </button>
          </div>
        </div>
        <div className="flex items-center justify-between border-b border-white/10 py-4">
          <span className="text-sm">Search model</span>
          <div className="flex items-center gap-3">
            <span className="text-sm text-white/50">{fmtBytes(modelBytes)}</span>
            <button
              type="button"
              className="rounded-md border border-white/15 px-3 py-1.5 text-sm text-white/70 hover:bg-white/10"
            >
              Remove model
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
