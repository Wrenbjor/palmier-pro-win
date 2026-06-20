// Media grid (E4-S9 layout + E4-S11 marquee/keyboard nav).
//
// Lays out folder/flat/grouped views with the exact `gridDimensions` math, renders
// `FolderTile`/`AssetTile`, and overlays a marquee selection rectangle. Tracks each
// cell's frame (`assetFrames`) for marquee hit-testing and arrow-key scroll-reveal.
//
// View-mode bodies:
//   folder  — breadcrumb + subfolders-then-assets of currentFolderId
//   flat    — every asset, no folders
//   grouped — collapsible sections (root "Library" + one per folder)

import { useEffect, useMemo, useRef, useState } from "react";
import * as React from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme } from "./theme";
import { AssetTile, FolderTile } from "./MediaTile";
import {
  buildView,
  breadcrumb,
  gridDimensions,
  marqueeRect,
  marqueeSelect,
  moveSelection,
  type FilterState,
  type Rect,
} from "./logic";
import { Interaction } from "./theme";
import { buildAssetDragPayload, folderUri } from "./drag";
import {
  folderIdFromItemKey,
  isFolderItemKey,
  type MediaAssetView,
  type MediaFolderView,
  type MediaPanelItemKey,
  type MediaSnapshot,
  type SortMode,
  type ViewMode,
} from "./types";

export interface MediaGridProps {
  snapshot: MediaSnapshot;
  currentFolderId: string | null;
  viewMode: ViewMode;
  sort: SortMode;
  thumbnailSize: number;
  filter: FilterState;
  selection: Set<MediaPanelItemKey>;
  collapsedSections: Set<string | null>;
  /** Width override for tests / SSR; otherwise measured from the container. */
  measuredWidth?: number;
  onOpenFolder: (id: string | null) => void;
  onSelect: (key: MediaPanelItemKey, additive: boolean) => void;
  onSetSelection: (keys: Iterable<MediaPanelItemKey>) => void;
  onRenameFolder: (id: string, name: string) => void;
  onRenameAsset: (id: string, name: string) => void;
  onToggleSection: (folderId: string | null) => void;
  /**
   * A drag payload (`text/plain`) was dropped on a folder/breadcrumb/section/root
   * target (E4-S12). `targetFolderId` is the destination (null = Library root).
   * `files` are absolute paths from a native OS-file drop (import); `text` is the
   * in-panel move payload.
   */
  onDropOnFolder?: (
    targetFolderId: string | null,
    drop: { files?: string[]; text?: string },
  ) => void;
  // --- per-asset OS actions (E4-S12) ---
  onRevealAsset?: (id: string) => void;
  onCopyAssetPath?: (id: string) => void;
  onRelinkAsset?: (id: string) => void;
  onDeleteAsset?: (id: string) => void;
}

export function MediaGrid(props: MediaGridProps) {
  const {
    snapshot,
    currentFolderId,
    viewMode,
    sort,
    thumbnailSize,
    filter,
    selection,
    collapsedSections,
  } = props;

  const containerRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(props.measuredWidth ?? 360);
  const frames = useRef<Map<MediaPanelItemKey, Rect>>(new Map());
  const [marquee, setMarquee] = useState<Rect | null>(null);
  const dragStart = useRef<{ x: number; y: number; additive: boolean } | null>(
    null,
  );

  useEffect(() => {
    if (props.measuredWidth != null) return;
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) setWidth(w);
    });
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, [props.measuredWidth]);

  const { columns, tileWidth } = useMemo(
    () => gridDimensions(width, thumbnailSize),
    [width, thumbnailSize],
  );

  const view = useMemo(
    () => buildView(viewMode, snapshot, currentFolderId, filter, sort),
    [viewMode, snapshot, currentFolderId, filter, sort],
  );

  const foldersById = useMemo(
    () => new Map(snapshot.folders.map((f) => [f.id, f])),
    [snapshot.folders],
  );
  const assetsById = useMemo(
    () => new Map(snapshot.assets.map((a) => [a.id, a])),
    [snapshot.assets],
  );

  // Record a cell frame for marquee hit-testing (relative to the scroll container).
  const reportFrame = (key: MediaPanelItemKey, el: HTMLDivElement | null) => {
    const container = containerRef.current;
    if (!el || !container) {
      frames.current.delete(key);
      return;
    }
    const cr = container.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    frames.current.set(key, {
      x: r.left - cr.left + container.scrollLeft,
      y: r.top - cr.top + container.scrollTop,
      w: r.width,
      h: r.height,
    });
  };

  // --- marquee selection (E4-S11) -------------------------------------------
  const onPointerDown = (e: React.PointerEvent) => {
    // Ignore drags that start on a cell (reference parity).
    const target = e.target as HTMLElement;
    if (target.closest("[data-asset-id], [data-folder-id], button, input")) {
      return;
    }
    const container = containerRef.current;
    if (!container) return;
    const cr = container.getBoundingClientRect();
    dragStart.current = {
      x: e.clientX - cr.left + container.scrollLeft,
      y: e.clientY - cr.top + container.scrollTop,
      additive: e.shiftKey,
    };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const start = dragStart.current;
    const container = containerRef.current;
    if (!start || !container) return;
    const cr = container.getBoundingClientRect();
    const x = e.clientX - cr.left + container.scrollLeft;
    const y = e.clientY - cr.top + container.scrollTop;
    if (
      Math.abs(x - start.x) < Interaction.marqueeMinDistance &&
      Math.abs(y - start.y) < Interaction.marqueeMinDistance
    ) {
      return;
    }
    const rect = marqueeRect(start.x, start.y, x, y);
    setMarquee(rect);
    const next = marqueeSelect(
      rect,
      frames.current,
      start.additive ? selection : new Set(),
      start.additive,
    );
    props.onSetSelection(next);
  };

  const onPointerUp = () => {
    dragStart.current = null;
    setMarquee(null);
  };

  // --- drop routing (E4-S12) -------------------------------------------------
  // A drop on a folder/breadcrumb/section/root target moves in-panel items (text
  // payload) or imports OS files (file payload). Native OS-file drops arrive via
  // the Tauri `tauri://drag-drop` event (panel root), so the in-DOM handler here
  // covers the in-panel move payload via `dataTransfer.getData("text/plain")`.
  const handleDrop = (
    e: React.DragEvent,
    targetFolderId: string | null,
  ) => {
    if (!props.onDropOnFolder) return;
    e.preventDefault();
    e.stopPropagation();
    const text = e.dataTransfer.getData("text/plain");
    const files = Array.from(e.dataTransfer.files ?? [])
      .map((f) => (f as File & { path?: string }).path)
      .filter((p): p is string => !!p);
    props.onDropOnFolder(targetFolderId, {
      text: text || undefined,
      files: files.length > 0 ? files : undefined,
    });
  };
  const allowDrop = (e: React.DragEvent) => {
    if (props.onDropOnFolder) e.preventDefault();
  };

  // --- keyboard arrow nav (E4-S11) ------------------------------------------
  const onKeyDown = (e: React.KeyboardEvent) => {
    const dir =
      e.key === "ArrowLeft"
        ? "left"
        : e.key === "ArrowRight"
          ? "right"
          : e.key === "ArrowUp"
            ? "up"
            : e.key === "ArrowDown"
              ? "down"
              : null;
    if (!dir) return;
    e.preventDefault();
    const current =
      selection.size > 0 ? Array.from(selection)[selection.size - 1] : null;
    const next = moveSelection(view.orderedKeys, current, dir, columns);
    if (next != null) {
      props.onSelect(next, e.shiftKey);
      // scroll the focused cell into view
      const fr = frames.current.get(next);
      const c = containerRef.current;
      if (fr && c) {
        if (fr.y < c.scrollTop) c.scrollTop = fr.y;
        else if (fr.y + fr.h > c.scrollTop + c.clientHeight)
          c.scrollTop = fr.y + fr.h - c.clientHeight;
      }
    }
  };

  const gridTemplate: CSSProperties = {
    display: "grid",
    gridTemplateColumns: `repeat(${columns}, ${tileWidth}px)`,
    gap: Spacing.xl,
    justifyContent: "start",
  };

  const renderAsset = (a: MediaAssetView) => (
    <div key={a.id} ref={(el) => reportFrame(a.id, el)}>
      <AssetTile
        asset={a}
        width={tileWidth}
        selected={selection.has(a.id)}
        onSelect={(additive) => props.onSelect(a.id, additive)}
        onRename={(name) => props.onRenameAsset(a.id, name)}
        onReveal={props.onRevealAsset ? () => props.onRevealAsset!(a.id) : undefined}
        onCopyPath={
          props.onCopyAssetPath ? () => props.onCopyAssetPath!(a.id) : undefined
        }
        onRelink={props.onRelinkAsset ? () => props.onRelinkAsset!(a.id) : undefined}
        onDelete={props.onDeleteAsset ? () => props.onDeleteAsset!(a.id) : undefined}
        onDragStart={(e) =>
          e.dataTransfer.setData(
            "text/plain",
            buildAssetDragPayload(a.id, selection),
          )
        }
      />
    </div>
  );

  const renderFolder = (f: MediaFolderView) => {
    const key = `folder-${f.id}`;
    return (
      <div key={key} ref={(el) => reportFrame(key, el)}>
        <FolderTile
          folder={f}
          width={tileWidth}
          selected={selection.has(key)}
          onSelect={(additive) => props.onSelect(key, additive)}
          onOpen={() => props.onOpenFolder(f.id)}
          onRename={(name) => props.onRenameFolder(f.id, name)}
          onDragStart={(e) =>
            e.dataTransfer.setData("text/plain", folderUri(f.id))
          }
          onDropTarget={
            props.onDropOnFolder ? (e) => handleDrop(e, f.id) : undefined
          }
        />
      </div>
    );
  };

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onDragOver={allowDrop}
      onDrop={
        props.onDropOnFolder ? (e) => handleDrop(e, currentFolderId) : undefined
      }
      style={{
        position: "relative",
        flex: 1,
        minHeight: 0,
        overflowY: "auto",
        overflowX: "hidden",
        padding: Spacing.md,
        outline: "none",
      }}
    >
      {viewMode === "folder" && (
        <Breadcrumb
          crumbs={breadcrumb(snapshot.folders, currentFolderId)}
          onNavigate={props.onOpenFolder}
          onDropCrumb={
            props.onDropOnFolder
              ? (id, e) => handleDrop(e, id)
              : undefined
          }
        />
      )}

      {viewMode === "grouped" ? (
        <div style={{ display: "flex", flexDirection: "column", gap: Spacing.lg }}>
          {view.sections.map((sec) => {
            const collapsed = collapsedSections.has(sec.folderId);
            return (
              <div key={sec.folderId ?? "__root"}>
                <button
                  onClick={() => props.onToggleSection(sec.folderId)}
                  onDragOver={allowDrop}
                  onDrop={
                    props.onDropOnFolder
                      ? (e) => handleDrop(e, sec.folderId)
                      : undefined
                  }
                  style={sectionHeaderStyle}
                >
                  <span>{collapsed ? "▸" : "▾"}</span>
                  <span>{sec.title}</span>
                  <span style={{ color: Theme.text.muted }}>
                    ({sec.assets.length})
                  </span>
                </button>
                {!collapsed && (
                  <div style={gridTemplate}>{sec.assets.map(renderAsset)}</div>
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <div style={gridTemplate}>
          {view.orderedKeys.map((key) => {
            if (isFolderItemKey(key)) {
              const id = folderIdFromItemKey(key);
              const f = id ? foldersById.get(id) : undefined;
              return f ? renderFolder(f) : null;
            }
            const a = assetsById.get(key);
            return a ? renderAsset(a) : null;
          })}
        </div>
      )}

      {view.orderedKeys.length === 0 && view.sections.length === 0 && (
        <div style={{ color: Theme.text.muted, padding: Spacing.xl, fontSize: 12 }}>
          No media here.
        </div>
      )}

      {marquee && (
        <div
          style={{
            position: "absolute",
            left: marquee.x,
            top: marquee.y,
            width: marquee.w,
            height: marquee.h,
            background: Theme.marqueeFill,
            border: `1px solid ${Theme.marqueeStroke}`,
            pointerEvents: "none",
          }}
        />
      )}
    </div>
  );
}

function Breadcrumb({
  crumbs,
  onNavigate,
  onDropCrumb,
}: {
  crumbs: { id: string | null; name: string }[];
  onNavigate: (id: string | null) => void;
  /** Each chip is a drop target (E4-S12): drop reparents into that folder. */
  onDropCrumb?: (id: string | null, e: React.DragEvent) => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 4,
        marginBottom: Spacing.md,
        flexWrap: "wrap",
      }}
    >
      {crumbs.map((c, i) => {
        const last = i === crumbs.length - 1;
        return (
          <span
            key={c.id ?? "__lib"}
            style={{ display: "flex", alignItems: "center", gap: 4 }}
          >
            <button
              onClick={() => !last && onNavigate(c.id)}
              onDragOver={(e) => {
                if (onDropCrumb) e.preventDefault();
              }}
              onDrop={onDropCrumb ? (e) => onDropCrumb(c.id, e) : undefined}
              disabled={last}
              style={{
                fontSize: 12,
                background: "transparent",
                border: "none",
                cursor: last ? "default" : "pointer",
                color: last ? Theme.text.primary : Theme.accentTimecode,
                padding: 0,
                fontWeight: last ? 600 : 400,
              }}
            >
              {c.name}
            </button>
            {!last && <span style={{ color: Theme.text.muted }}>/</span>}
          </span>
        );
      })}
    </div>
  );
}

const sectionHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  width: "100%",
  textAlign: "left",
  fontSize: 12,
  fontWeight: 600,
  color: Theme.text.secondary,
  background: "transparent",
  border: "none",
  cursor: "pointer",
  padding: `${Spacing.xs}px 0`,
  marginBottom: Spacing.sm,
};
