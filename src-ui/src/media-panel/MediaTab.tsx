// Media tab body (E4-S9/S10/S11): toolbar + (grid | search results) + index pill +
// generation panel. When the search query is non-empty the grid is replaced by the
// search-results panel (Moments/Spoken/Files); otherwise the browse grid shows.

import { useMemo } from "react";
import { Theme } from "./theme";
import { MediaToolbar } from "./MediaToolbar";
import { MediaGrid } from "./MediaGrid";
import { SearchResultsPanel } from "./SearchResultsPanel";
import { IndexStatusPill } from "./IndexStatusPill";
import { GenerationPanel } from "./GenerationPanel";
import { secondsToFrame } from "./search";
import type { MediaPanelController } from "./controller";
import { useMediaStore, type MediaPanelStore } from "./store";

export interface MediaTabProps {
  store: MediaPanelStore;
  controller: MediaPanelController;
  /** Timeline fps for moment/spoken tap → `selectMediaAsset(atSourceFrame:)`. */
  fps?: number;
}

export function MediaTab({ store, controller, fps = 30 }: MediaTabProps) {
  const snapshot = useMediaStore(store, (s) => s.snapshot);
  const currentFolderId = useMediaStore(store, (s) => s.currentFolderId);
  const sort = useMediaStore(store, (s) => s.sort);
  const viewMode = useMediaStore(store, (s) => s.viewMode);
  const thumbnailSize = useMediaStore(store, (s) => s.thumbnailSize);
  const filter = useMediaStore(store, (s) => s.filter);
  const selection = useMediaStore(store, (s) => s.selection);
  const collapsedSections = useMediaStore(store, (s) => s.collapsedSections);
  const searchResults = useMediaStore(store, (s) => s.searchResults);
  const indexStatus = useMediaStore(store, (s) => s.indexStatus);
  const jobs = useMediaStore(store, (s) => s.jobs);

  const assetsById = useMemo(
    () => new Map(snapshot.assets.map((a) => [a.id, a])),
    [snapshot.assets],
  );

  const searching = filter.query.trim().length > 0;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        background: Theme.background.base,
      }}
    >
      <MediaToolbar
        sort={sort}
        viewMode={viewMode}
        thumbnailSize={thumbnailSize}
        filter={filter}
        onSort={(s) => store.setSort(s)}
        onViewMode={(v) => store.setViewMode(v)}
        onThumbnailSize={(n) => store.setThumbnailSize(n)}
        onToggleType={(t) => store.toggleTypeFilter(t)}
        onFilterAI={(on) => store.setFilterAI(on)}
        onQuery={(q) => controller.search(q)}
        onNewFolder={() => controller.createFolder()}
      />

      {searching && searchResults ? (
        <SearchResultsPanel
          results={searchResults}
          assetsById={assetsById}
          onSelectFile={(a) => {
            store.setSelection([a.id]);
            store.setFocused(a.id);
          }}
          onSelectMoment={(hit) =>
            // previewMoment(atSeconds: range.lowerBound = shotStart) →
            // selectMediaAsset(atSourceFrame: secondsToFrame(shotStart, fps)).
            controller.selectMediaAtSource(
              hit.assetID,
              secondsToFrame(hit.shotStart, fps),
            )
          }
          onSelectSpoken={(hit) =>
            // previewMoment(atSeconds: range.lowerBound = start) →
            // selectMediaAsset(atSourceFrame: secondsToFrame(start, fps)).
            controller.selectMediaAtSource(
              hit.assetID,
              secondsToFrame(hit.start, fps),
            )
          }
        />
      ) : (
        <MediaGrid
          snapshot={snapshot}
          currentFolderId={currentFolderId}
          viewMode={viewMode}
          sort={sort}
          thumbnailSize={thumbnailSize}
          filter={filter}
          selection={selection}
          collapsedSections={collapsedSections}
          onOpenFolder={(id) => store.openFolder(id)}
          onSelect={(key, additive) => store.toggleSelection(key, additive)}
          onSetSelection={(keys) => store.setSelection(keys)}
          onRenameFolder={(id, name) => controller.renameFolder(id, name)}
          onRenameAsset={(id, name) => controller.renameAsset(id, name)}
          onToggleSection={(fid) => store.toggleSectionCollapsed(fid)}
          onDropOnFolder={(target, drop) =>
            void controller.handleProviderDrop(drop, target)
          }
          onRevealAsset={(id) => void controller.revealAsset(id)}
          onCopyAssetPath={(id) => void controller.copyAssetPath(id)}
          onRelinkAsset={(id) => void controller.relinkAsset(id)}
        />
      )}

      <IndexStatusPill status={indexStatus} />
      <GenerationPanel
        jobs={jobs}
        onCancel={(id) => controller.cancelJob(id)}
        onDismiss={(id) => controller.dismissJob(id)}
      />
    </div>
  );
}
