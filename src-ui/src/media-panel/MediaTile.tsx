// Asset + folder tiles for the media grid (E4-S9 hosts these; tile rendering here).
//
// `AssetTile` renders the thumbnail (or a type-colored placeholder until the
// E4-S3/E4-S5 pipeline fills `thumbnailUrl`), the type/AI badges, the duration, an
// inline rename field, and a lightweight context menu (Reveal / Copy Path / Relink
// / Rename / Delete). `FolderTile` renders a folder with inline rename and
// single/double-click open. Selection visuals come from `theme.ts`.
//
// Tap = single-select; shift-tap toggles the set (E4-S11 wires the keyboard/marquee
// side). The context-menu actions (Reveal in Finder / Copy Path / Relink) are Tauri
// `opener`/`clipboard`/`dialog` calls that land with E4-S12 — here they call the
// provided callbacks (no-op by default) so the menu is present and wired-ready.

import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme, typeColor } from "./theme";
import { tileHeight } from "./logic";
import type { MediaAssetView, MediaFolderView } from "./types";

function durationLabel(seconds: number | null): string | null {
  if (seconds == null) return null;
  const s = Math.max(0, Math.round(seconds));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${r.toString().padStart(2, "0")}`;
}

export interface AssetTileProps {
  asset: MediaAssetView;
  width: number;
  selected: boolean;
  onSelect: (additive: boolean) => void;
  onRename: (name: string) => void;
  onReveal?: () => void;
  onCopyPath?: () => void;
  onRelink?: () => void;
  onDelete?: () => void;
  onDragStart?: (e: React.DragEvent) => void;
}

export function AssetTile({
  asset,
  width,
  selected,
  onSelect,
  onRename,
  onReveal,
  onCopyPath,
  onRelink,
  onDelete,
  onDragStart,
}: AssetTileProps) {
  const [editing, setEditing] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [draft, setDraft] = useState(asset.name);
  const inputRef = useRef<HTMLInputElement>(null);
  const h = tileHeight(width);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    setEditing(false);
    const name = draft.trim();
    if (name.length > 0 && name !== asset.name) onRename(name);
    else setDraft(asset.name);
  };

  const dur = durationLabel(asset.durationSeconds);
  const tileStyle: CSSProperties = {
    position: "relative",
    width,
    boxSizing: "border-box",
    borderRadius: 6,
    border: `1px solid ${selected ? Theme.selectionStroke : Theme.border.subtle}`,
    background: selected ? Theme.selectionFill : Theme.background.raised,
    overflow: "hidden",
    cursor: "pointer",
    userSelect: "none",
  };

  return (
    <div
      role="gridcell"
      aria-selected={selected}
      data-asset-id={asset.id}
      style={tileStyle}
      draggable
      onDragStart={onDragStart}
      onClick={(e) => onSelect(e.shiftKey || e.metaKey || e.ctrlKey)}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuOpen(true);
      }}
    >
      {/* Thumbnail / placeholder (16:9) */}
      <div
        style={{
          width: "100%",
          height: h,
          background: asset.thumbnailUrl
            ? `center / cover no-repeat url(${asset.thumbnailUrl})`
            : typeColor(asset.type, 0.28),
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        {!asset.thumbnailUrl && (
          <span style={{ color: Theme.text.tertiary, fontSize: 11 }}>
            {asset.type}
          </span>
        )}
        {/* badges */}
        <div
          style={{
            position: "absolute",
            top: Spacing.xs,
            left: Spacing.xs,
            display: "flex",
            gap: Spacing.xs,
          }}
        >
          <span style={badgeStyle(typeColor(asset.type, 0.9))}>
            {asset.type}
          </span>
          {asset.isGenerated && <span style={badgeStyle(Theme.accentTimecode)}>AI</span>}
          {asset.missing && (
            <span style={badgeStyle(Theme.status.error)}>missing</span>
          )}
        </div>
        {dur && (
          <span
            style={{
              position: "absolute",
              bottom: Spacing.xs,
              right: Spacing.xs,
              ...badgeStyle("rgba(0,0,0,0.6)"),
            }}
          >
            {dur}
          </span>
        )}
      </div>

      {/* name / rename */}
      <div style={{ padding: `${Spacing.xs}px ${Spacing.sm}px` }}>
        {editing ? (
          <input
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setDraft(asset.name);
                setEditing(false);
              }
            }}
            style={nameInputStyle}
          />
        ) : (
          <div
            title={asset.name}
            onDoubleClick={() => setEditing(true)}
            style={nameLabelStyle}
          >
            {asset.name}
          </div>
        )}
      </div>

      {menuOpen && (
        <ContextMenu
          onClose={() => setMenuOpen(false)}
          items={[
            { label: "Reveal in Explorer", onClick: onReveal },
            { label: "Copy Path", onClick: onCopyPath },
            { label: "Rename", onClick: () => setEditing(true) },
            ...(asset.missing
              ? [{ label: "Relink…", onClick: onRelink }]
              : []),
            { label: "Delete", onClick: onDelete, danger: true },
          ]}
        />
      )}
    </div>
  );
}

export interface FolderTileProps {
  folder: MediaFolderView;
  width: number;
  selected: boolean;
  onSelect: (additive: boolean) => void;
  onOpen: () => void;
  onRename: (name: string) => void;
  onDragStart?: (e: React.DragEvent) => void;
  onDropTarget?: (e: React.DragEvent) => void;
}

export function FolderTile({
  folder,
  width,
  selected,
  onSelect,
  onOpen,
  onRename,
  onDragStart,
  onDropTarget,
}: FolderTileProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(folder.name);
  const inputRef = useRef<HTMLInputElement>(null);
  const h = tileHeight(width);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    setEditing(false);
    const name = draft.trim();
    if (name.length > 0 && name !== folder.name) onRename(name);
    else setDraft(folder.name);
  };

  return (
    <div
      role="gridcell"
      aria-selected={selected}
      data-folder-id={folder.id}
      draggable
      onDragStart={onDragStart}
      onDragOver={(e) => {
        if (onDropTarget) e.preventDefault();
      }}
      onDrop={onDropTarget}
      onClick={(e) => onSelect(e.shiftKey || e.metaKey || e.ctrlKey)}
      onDoubleClick={onOpen}
      style={{
        width,
        height: h + 26,
        boxSizing: "border-box",
        borderRadius: 6,
        border: `1px solid ${selected ? Theme.selectionStroke : Theme.border.subtle}`,
        background: selected ? Theme.selectionFill : Theme.background.surface,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: Spacing.xs,
        cursor: "pointer",
        userSelect: "none",
      }}
    >
      <span style={{ fontSize: 26, lineHeight: 1 }}>📁</span>
      {editing ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              setDraft(folder.name);
              setEditing(false);
            }
          }}
          style={nameInputStyle}
        />
      ) : (
        <div
          title={folder.name}
          onDoubleClick={(e) => {
            e.stopPropagation();
            setEditing(true);
          }}
          style={{ ...nameLabelStyle, textAlign: "center", maxWidth: width - 12 }}
        >
          {folder.name}
        </div>
      )}
    </div>
  );
}

// --- shared bits --------------------------------------------------------------

function badgeStyle(bg: string): CSSProperties {
  return {
    fontSize: 9,
    lineHeight: "12px",
    padding: "1px 4px",
    borderRadius: 3,
    color: "#fff",
    background: bg,
    textTransform: "uppercase",
    letterSpacing: 0.3,
    fontWeight: 600,
  };
}

const nameLabelStyle: CSSProperties = {
  fontSize: 11,
  color: Theme.text.secondary,
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
};

const nameInputStyle: CSSProperties = {
  width: "100%",
  fontSize: 11,
  color: Theme.text.primary,
  background: Theme.background.base,
  border: `1px solid ${Theme.accentTimecode}`,
  borderRadius: 3,
  padding: "1px 3px",
  boxSizing: "border-box",
};

interface MenuItem {
  label: string;
  onClick?: () => void;
  danger?: boolean;
}

function ContextMenu({
  items,
  onClose,
}: {
  items: MenuItem[];
  onClose: () => void;
}) {
  return (
    <>
      <div
        onClick={onClose}
        onContextMenu={(e) => {
          e.preventDefault();
          onClose();
        }}
        style={{ position: "fixed", inset: 0, zIndex: 50 }}
      />
      <div
        style={{
          position: "absolute",
          top: 24,
          left: 8,
          zIndex: 51,
          minWidth: 160,
          background: Theme.background.prominent,
          border: `1px solid ${Theme.border.primary}`,
          borderRadius: 6,
          padding: 4,
          boxShadow: "0 6px 20px rgba(0,0,0,0.5)",
        }}
      >
        {items.map((it) => (
          <button
            key={it.label}
            onClick={(e) => {
              e.stopPropagation();
              it.onClick?.();
              onClose();
            }}
            style={{
              display: "block",
              width: "100%",
              textAlign: "left",
              padding: "5px 8px",
              fontSize: 12,
              color: it.danger ? "#ff8a8a" : Theme.text.primary,
              background: "transparent",
              border: "none",
              borderRadius: 4,
              cursor: "pointer",
            }}
          >
            {it.label}
          </button>
        ))}
      </div>
    </>
  );
}
