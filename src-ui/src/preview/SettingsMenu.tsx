// Preview settings menu (E5-S10) — aspect / fps / quality / zoom.
//
// Port of the macOS reference `PreviewContainerView.projectSettingsGroup`: four badge
// dropdowns showing the current aspect ratio, frame rate, resolution, and canvas zoom,
// each opening a checkmarked preset list. Aspect/fps/quality apply new timeline
// settings (width/height/fps) via `onApplyTimelineSettings`; zoom sets the canvas zoom
// via `onSetZoom` and resets the pan offset (reference behavior).

import { useEffect, useRef, useState } from "react";

import {
  ASPECT_PRESETS,
  FPS_PRESETS,
  QUALITY_PRESETS,
  ZOOM_PRESETS,
  aspectBadgeLabel,
  isZoomPresetActive,
  qualityBadgeLabel,
  qualityMatches,
  qualityResolution,
  zoomBadgeLabel,
} from "./presets";

export interface SettingsMenuProps {
  width: number;
  height: number;
  fps: number;
  zoom: number;
  onApplyTimelineSettings: (settings: { fps: number; width: number; height: number }) => void;
  onSetZoom: (zoom: number) => void;
}

export function SettingsMenu({
  width,
  height,
  fps,
  zoom,
  onApplyTimelineSettings,
  onSetZoom,
}: SettingsMenuProps) {
  return (
    <div className="flex items-center gap-2">
      <BadgeMenu label={aspectBadgeLabel(width, height)} title="Aspect Ratio">
        {ASPECT_PRESETS.map((p) => (
          <MenuItem
            key={p.label}
            label={p.label}
            checked={width === p.width && height === p.height}
            onClick={() => onApplyTimelineSettings({ fps, width: p.width, height: p.height })}
          />
        ))}
      </BadgeMenu>

      <BadgeMenu label={`${fps}`} title="Frame Rate">
        {FPS_PRESETS.map((f) => (
          <MenuItem
            key={f}
            label={`${f} fps`}
            checked={fps === f}
            onClick={() => onApplyTimelineSettings({ fps: f, width, height })}
          />
        ))}
      </BadgeMenu>

      <BadgeMenu label={qualityBadgeLabel(width, height)} title="Resolution">
        {QUALITY_PRESETS.map((p) => (
          <MenuItem
            key={p.label}
            label={p.label}
            checked={qualityMatches(p, width, height)}
            onClick={() => {
              const res = qualityResolution(p, width, height);
              onApplyTimelineSettings({ fps, width: res.width, height: res.height });
            }}
          />
        ))}
      </BadgeMenu>

      <BadgeMenu label={zoomBadgeLabel(zoom)} title="Canvas Zoom">
        {ZOOM_PRESETS.map((p) => (
          <MenuItem
            key={p.label}
            label={p.label}
            checked={isZoomPresetActive(p, zoom)}
            onClick={() => onSetZoom(p.value)}
          />
        ))}
      </BadgeMenu>
    </div>
  );
}

function BadgeMenu({
  label,
  title,
  children,
}: {
  label: string;
  title: string;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        title={title}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="rounded px-2 py-1 text-[11px] font-bold text-white/70 hover:bg-white/10 hover:text-white"
      >
        {label}
      </button>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 z-10 mb-1 min-w-32 rounded border border-white/10 bg-[#1e1e1e] py-1 shadow-lg"
          onClick={() => setOpen(false)}
        >
          {children}
        </div>
      )}
    </div>
  );
}

function MenuItem({
  label,
  checked,
  onClick,
}: {
  label: string;
  checked: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={checked}
      onClick={onClick}
      className="flex w-full items-center justify-between px-3 py-1 text-left text-xs text-white/80 hover:bg-white/10"
    >
      <span>{label}</span>
      {checked && <span className="text-cyan-300">✓</span>}
    </button>
  );
}
