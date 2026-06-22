// Pure-math parity checks for the timeline canvas (E3-S9 acceptance).
//
// There is no test runner wired into `src-ui` yet (no vitest), and adding one would
// touch the shared package.json/lockfile owned by the concurrent app-shell worker.
// So these parity checks are written as a framework-free, type-checked module: it is
// covered by `tsc --noEmit` (so it can never drift from the types it exercises) and
// can be executed directly once a runner exists, or via a quick transpile.
//
// To run today (from `src-ui/`):
//   corepack pnpm exec vite-node src/editor/parity.checks.ts     # if vite-node added
// or fold these asserts into the first `*.test.ts` when vitest lands — `runParityChecks`
// returns the failures so a test can simply `expect(runParityChecks()).toEqual([])`.
//
// Golden values verified against the macOS reference math:
//   `TimelineGeometry.clipRect/frameAt`, `TimelineRuler.tickInterval/minorSubdivisions`,
//   `Utilities/TimeFormatting.formatTimecode`, `Models/Keyframe.sample`,
//   `Models/Timeline.volumeAt/opacityAt`, waveform trim->sample mapping.

import {
  clipRect,
  endFrame,
  formatTimecode,
  frameAt,
  makeLayout,
  minorSubdivisions,
  opacityAt,
  roundTiesAway,
  sampleTrack,
  sourceDurationFrames,
  sourceFramesConsumed,
  tickInterval,
  volumeAt,
} from "./geometry";
import { Defaults, VolumeScale, ClipRender } from "./theme";
import type { ClipView, KeyframeTrackView } from "./types";
import {
  dbForY,
  envelopeBodyRect,
  envelopeValueForY,
  hitTestEnvelope,
  mergeScalarKeyframe,
  opacityForY,
  yForDb,
  yForOpacity,
} from "./envelope";

function approx(a: number, b: number, eps = 1e-9): boolean {
  return Math.abs(a - b) <= eps;
}

const baseClip = (over: Partial<ClipView>): ClipView => ({
  id: "c",
  name: "c",
  mediaRef: "c",
  mediaType: "video",
  sourceClipType: "video",
  startFrame: 0,
  durationFrames: 100,
  trimStartFrame: 0,
  trimEndFrame: 0,
  speed: 1,
  volume: 1,
  opacity: 1,
  fadeInFrames: 0,
  fadeOutFrames: 0,
  fadeInInterpolation: "linear",
  fadeOutInterpolation: "linear",
  ...over,
});

/** Returns a list of failure messages; empty == all green. */
export function runParityChecks(): string[] {
  const fail: string[] = [];
  const check = (cond: boolean, msg: string) => {
    if (!cond) fail.push(msg);
  };

  // --- clipRect geometry (ppf=4 default) ---
  {
    const layout = makeLayout(Defaults.pixelsPerFrame, [50, 50]);
    const clip = baseClip({ startFrame: 30, durationFrames: 60 });
    const r = clipRect(layout, clip, 0);
    // track 0 top = rulerHeight(24) + dropZoneHeight(60) = 84; y = 84 + 2 = 86
    check(r.x === 0 + 30 * 4, `clipRect.x ${r.x}`);
    check(r.y === 86, `clipRect.y ${r.y}`);
    check(r.w === 60 * 4, `clipRect.w ${r.w}`);
    check(r.h === 50 - 4, `clipRect.h ${r.h}`);
    // track 1 top = 84 + 50 = 134; y = 136
    check(clipRect(layout, clip, 1).y === 136, "clipRect track1 y");
  }

  // --- frameAt: max(0, floor((x - header)/ppf)) ---
  {
    const layout = makeLayout(4, [50]);
    check(frameAt(layout, 0) === 0, "frameAt 0");
    check(frameAt(layout, 7) === 1, "frameAt 7 -> 1"); // floor(7/4)=1
    check(frameAt(layout, -5) === 0, "frameAt negative clamps to 0");
  }

  // --- ruler tick interval / minor subdivisions (fps 30) ---
  {
    // ppf=4: target 80px -> rawFrames=20; first candidate*30 >= 20 is 1*30=30
    check(tickInterval(4, 30) === 30, `tickInterval(4,30)=${tickInterval(4, 30)}`);
    // majorPixels = 30*4 = 120; /10 = 12 >= 12 -> 10 subdivisions
    check(
      minorSubdivisions(30, 4) === 10,
      `minorSubdivisions(30,4)=${minorSubdivisions(30, 4)}`,
    );
    // tiny ppf -> coarse interval still positive
    check(tickInterval(0.1, 30) > 0, "tickInterval small ppf positive");
  }

  // --- timecode HH:MM:SS:FF ---
  {
    check(formatTimecode(0, 30) === "00:00:00:00", "tc 0");
    check(formatTimecode(30, 30) === "00:00:01:00", "tc 1s");
    check(formatTimecode(90, 30) === "00:00:03:00", "tc 3s");
    check(formatTimecode(1, 30) === "00:00:00:01", "tc 1 frame");
    check(
      formatTimecode(3600 * 30 + 1, 30) === "01:00:00:01",
      "tc 1h+1frame",
    );
  }

  // --- source frame derivations (ties-away round) ---
  {
    // speed 1.7, duration 100 -> 170
    check(
      sourceFramesConsumed(baseClip({ speed: 1.7, durationFrames: 100 })) === 170,
      "sourceFramesConsumed 1.7",
    );
    // 0.5 ties: duration 3 -> 1.5 -> rounds away to 2
    check(
      roundTiesAway(1.5) === 2 && roundTiesAway(2.5) === 3,
      "roundTiesAway ties-away",
    );
    const c = baseClip({ speed: 0.5, durationFrames: 3, trimStartFrame: 4, trimEndFrame: 1 });
    // consumed = round(1.5)=2; sourceDuration = 2+4+1 = 7
    check(sourceFramesConsumed(c) === 2, "consumed 0.5*3");
    check(sourceDurationFrames(c) === 7, "sourceDuration");
    check(endFrame(baseClip({ startFrame: 10, durationFrames: 5 })) === 15, "endFrame");
  }

  // --- keyframe sample: half-open behaviour + interpolation switch on leaving kf ---
  {
    const track: KeyframeTrackView = {
      keyframes: [
        { frame: 0, value: 0, interpolationOut: "linear" },
        { frame: 10, value: 10, interpolationOut: "hold" },
        { frame: 20, value: 0, interpolationOut: "smooth" },
      ],
    };
    check(sampleTrack(track, -5, 99) === 0, "sample before first -> first");
    check(sampleTrack(track, 25, 99) === 0, "sample after last -> last");
    check(approx(sampleTrack(track, 5, 0), 5), "sample linear mid"); // lerp 0..10
    check(sampleTrack(track, 15, 0) === 10, "sample hold holds a.value");
    check(sampleTrack(null, 5, 7) === 7, "sample empty -> fallback");
  }

  // --- volumeAt: linear * kfGain(dB) * fade ; opacityAt: raw * fade (non-audio) ---
  {
    // audio with static volume 0.5, no kfs, no fade -> 0.5
    const a = baseClip({ mediaType: "audio", volume: 0.5 });
    check(approx(volumeAt(a, 10), 0.5), `volumeAt static ${volumeAt(a, 10)}`);
    // VolumeScale round-trip near unity
    check(approx(VolumeScale.linearFromDb(0), 1), "0 dB -> 1.0 linear");
    check(VolumeScale.linearFromDb(-60) === 0, "-60 dB hard mute");
    // opacity fade: 20-frame smooth fade-in, sample at clip start -> 0
    const v = baseClip({ fadeInFrames: 20, fadeInInterpolation: "smooth" });
    check(approx(opacityAt(v, 0), 0), `opacityAt fade-in start ${opacityAt(v, 0)}`);
    check(approx(opacityAt(v, 20), 1), `opacityAt fade-in end ${opacityAt(v, 20)}`);
    // audio never applies opacity fade reduction even with a fade present
    const af = baseClip({ mediaType: "audio", fadeInFrames: 20 });
    check(approx(opacityAt(af, 0), 1), "audio opacity ignores fade");
  }

  // --- envelope value↔y mapping (the SHARED draw + hit-test math) ---
  {
    const body = { x: 0, y: 100, w: 400, h: 50 };
    // Volume dB axis: top=6 → body.y, bottom=-60 → body.y+h; round-trips.
    check(approx(yForDb(ClipRender.volumeRubberBandTopDb, body), 100), "yForDb top→body.y");
    check(approx(yForDb(ClipRender.volumeRubberBandBottomDb, body), 150), "yForDb bottom→body bottom");
    check(approx(dbForY(yForDb(0, body), body), 0), "dbForY∘yForDb(0 dB) round-trip");
    check(approx(dbForY(yForDb(-20, body), body), -20), "dbForY∘yForDb(-20 dB) round-trip");
    // Opacity axis: 1 → top, 0 → bottom; round-trips.
    check(approx(yForOpacity(1, body), 100), "yForOpacity 1→body.y");
    check(approx(yForOpacity(0, body), 150), "yForOpacity 0→body bottom");
    check(approx(opacityForY(yForOpacity(0.5, body), body), 0.5), "opacityForY round-trip 0.5");
    check(approx(opacityForY(125, body), 0.5), "opacityForY midpoint→0.5");
    // Generic dispatch by kind agrees with the per-kind helpers.
    check(approx(envelopeValueForY("opacity", 125, body), 0.5), "envelopeValueForY opacity mid");
    check(approx(envelopeValueForY("volume", yForDb(-12, body), body), -12), "envelopeValueForY volume -12");
  }

  // --- envelope hit-test: grabbing the drawn line at a given y ---
  {
    const layout = makeLayout(Defaults.pixelsPerFrame, [50]);
    // Audio clip, static volume 1.0 → 0 dB → line at body.y; body for clipRect(track0).
    const aclip = baseClip({ mediaType: "audio", sourceClipType: "audio", volume: 1, durationFrames: 60 });
    const arect = clipRect(layout, aclip, 0);
    const abody = envelopeBodyRect(arect);
    const lineY = yForDb(VolumeScale.dbFromLinear(aclip.volume), abody);
    // Cursor right on the line, mid-clip → hit; value read back is ~0 dB.
    const hit = hitTestEnvelope(aclip, arect, arect.x + arect.w / 2, lineY, 30);
    check(hit !== null && hit.kind === "volume", "hitTestEnvelope volume on-line hits");
    check(hit !== null && approx(hit.lineY, lineY), "hitTestEnvelope reports line y");
    // Cursor far from the line (bottom of body, ~full-scale away) → miss (beyond tol).
    const miss = hitTestEnvelope(aclip, arect, arect.x + arect.w / 2, abody.y + abody.h, 30);
    check(miss === null, "hitTestEnvelope off-line misses");
    // Dragging to a new y computes the value the renderer would draw there.
    const targetY = yForDb(-12, abody);
    const dragged = envelopeValueForY("volume", targetY, abody);
    check(approx(dragged, -12), `envelope drag y→-12 dB (got ${dragged})`);
    // → stored linear volume via VolumeScale (band value is dB).
    check(approx(VolumeScale.linearFromDb(dragged), VolumeScale.linearFromDb(-12)), "drag→linear volume");
  }

  // --- Alt-drag keyframe row: merge into a sorted [frame,value,interp] list ---
  {
    const existing = [
      { frame: 0, value: -6, interpolationOut: "linear" as const },
      { frame: 40, value: -3, interpolationOut: "smooth" as const },
    ];
    // Insert a new keyframe at frame 20 with value -12.
    const rows = mergeScalarKeyframe(existing, 20, -12);
    check(rows.length === 3, `merge inserts new kf (len ${rows.length})`);
    check(rows[0][0] === 0 && rows[1][0] === 20 && rows[2][0] === 40, "merge keeps frame-sorted");
    check(rows[1][1] === -12, "merge new kf value");
    // Replacing an existing frame updates in place (no duplicate).
    const replaced = mergeScalarKeyframe(existing, 40, -1);
    check(replaced.length === 2, `merge replaces same-frame kf (len ${replaced.length})`);
    check(replaced[1][0] === 40 && replaced[1][1] === -1, "merge replaced value at frame 40");
  }

  return fail;
}
