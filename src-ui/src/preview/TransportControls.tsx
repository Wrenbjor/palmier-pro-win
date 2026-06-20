// Transport controls + scrub bar (E5-S10).
//
// Port of the macOS reference `PreviewContainerView` `transportBar` + `scrubBar`:
// the timecode readout, the 5 transport buttons (start / step-back / play-pause /
// step-forward / end), and the draggable scrub bar (interactive-scrub during drag,
// exact on release). Keyboard transport (J/K/L/Space/Home/End) is bound at the panel
// level (`PreviewPanel`); this component owns the buttons + the scrubber.
//
// All actions are callbacks the panel routes through the engine transport via Tauri
// commands (FOUNDATION §4). `mode` follows the reference: scrub drag = interactiveScrub,
// release = exact (so the playhead lands precisely).

import { useRef, useState } from "react";

import { formatTimecode, scrubFrame } from "./geometry";
import type { SeekMode } from "./api";

export interface TransportControlsProps {
  fps: number;
  /** The active tab's playhead frame. */
  frame: number;
  /** The active tab's duration in frames. */
  durationFrames: number;
  playing: boolean;
  onTogglePlay: () => void;
  onSeek: (frame: number, mode: SeekMode) => void;
  onScrubBegin: () => void;
  onScrubEnd: (frame: number) => void;
}

export function TransportControls({
  fps,
  frame,
  durationFrames,
  playing,
  onTogglePlay,
  onSeek,
  onScrubBegin,
  onScrubEnd,
}: TransportControlsProps) {
  return (
    <div className="flex flex-col gap-1 px-4 py-2">
      <ScrubBar
        frame={frame}
        durationFrames={durationFrames}
        onSeek={onSeek}
        onScrubBegin={onScrubBegin}
        onScrubEnd={onScrubEnd}
      />
      <div className="flex items-center justify-between">
        <span className="font-mono text-xs tabular-nums text-cyan-300">
          {formatTimecode(frame, fps)}
          <span className="text-white/40"> / </span>
          <span className="text-white/70">{formatTimecode(durationFrames, fps)}</span>
        </span>

        <div className="flex items-center gap-2">
          <TransportButton label="Go to start" onClick={() => onSeek(0, "exact")} title="Home">
            ⏮
          </TransportButton>
          <TransportButton label="Step back" onClick={() => onSeek(frame - 1, "exact")} title="Step back (←)">
            ◀|
          </TransportButton>
          <TransportButton
            label={playing ? "Pause" : "Play"}
            onClick={onTogglePlay}
            title="Play/Pause (Space / K)"
          >
            {playing ? "❚❚" : "▶"}
          </TransportButton>
          <TransportButton label="Step forward" onClick={() => onSeek(frame + 1, "exact")} title="Step forward (→)">
            |▶
          </TransportButton>
          <TransportButton label="Go to end" onClick={() => onSeek(durationFrames, "exact")} title="End">
            ⏭
          </TransportButton>
        </div>

        <span className="w-24" aria-hidden />
      </div>
    </div>
  );
}

function TransportButton({
  children,
  label,
  title,
  onClick,
}: {
  children: React.ReactNode;
  label: string;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title}
      onClick={onClick}
      className="flex h-7 w-8 items-center justify-center rounded text-sm text-white/70 hover:bg-white/10 hover:text-white"
    >
      {children}
    </button>
  );
}

interface ScrubBarProps {
  frame: number;
  durationFrames: number;
  onSeek: (frame: number, mode: SeekMode) => void;
  onScrubBegin: () => void;
  onScrubEnd: (frame: number) => void;
}

function ScrubBar({ frame, durationFrames, onSeek, onScrubBegin, onScrubEnd }: ScrubBarProps) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const [scrubbing, setScrubbing] = useState(false);
  const progress = durationFrames > 0 ? Math.max(0, Math.min(1, frame / durationFrames)) : 0;

  const frameForEvent = (clientX: number): number => {
    const el = trackRef.current;
    if (!el) return 0;
    const r = el.getBoundingClientRect();
    return scrubFrame(clientX - r.left, r.width, durationFrames);
  };

  const onDown = (e: React.PointerEvent) => {
    (e.target as Element).setPointerCapture(e.pointerId);
    setScrubbing(true);
    onScrubBegin();
    onSeek(frameForEvent(e.clientX), "interactiveScrub");
  };
  const onMove = (e: React.PointerEvent) => {
    if (!scrubbing) return;
    onSeek(frameForEvent(e.clientX), "interactiveScrub");
  };
  const onUp = (e: React.PointerEvent) => {
    if (!scrubbing) return;
    setScrubbing(false);
    onScrubEnd(frameForEvent(e.clientX));
  };

  return (
    <div
      ref={trackRef}
      role="slider"
      aria-label="Playhead"
      aria-valuemin={0}
      aria-valuemax={durationFrames}
      aria-valuenow={frame}
      tabIndex={0}
      className="relative flex h-3 cursor-pointer items-center"
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      <div className={`w-full rounded-full bg-white/15 ${scrubbing ? "h-1" : "h-[3px]"}`} />
      <div
        className="absolute left-0 rounded-full bg-cyan-400"
        style={{ width: `${progress * 100}%`, height: scrubbing ? 4 : 3 }}
      />
      <div
        className="absolute rounded-full bg-white shadow"
        style={{
          left: `calc(${progress * 100}% - ${scrubbing ? 5 : 3}px)`,
          width: scrubbing ? 10 : 6,
          height: scrubbing ? 10 : 6,
        }}
      />
    </div>
  );
}
