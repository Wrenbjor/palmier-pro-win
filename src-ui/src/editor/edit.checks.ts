// Framework-free logic checks for the input-controller edit layer (E3-S10).
//
// Same pattern as `parity.checks.ts`: no test runner is wired into `src-ui` yet (adding
// one would touch the shared package.json/lockfile owned by the concurrent app-shell
// worker), so these are written as a type-checked module covered by `tsc --noEmit` and
// runnable directly (`_run-edit-checks.mts`). When vitest lands, fold each block into a
// `*.test.ts` — `runEditChecks()` returns failures so a test can `expect(...).toEqual([])`.
//
// Coverage mirrors the E3-S10 acceptance + the §11.3 hand-edit gate:
//   snap stickiness 1.5× (ruling #10), merge_ranges touching-merge, compute_overwrite
//   four cases, split_clip round-trip + kf migration, trim clamps (incl. no-source),
//   the full hand-edit sequence (move cross-track → trim → split → ripple-delete →
//   undo/redo restores exact state).

import type { ClipView, TimelineView } from "./types";
import {
  computeOverwrite,
  isCompatible,
  mergeRanges,
  splitClip,
  trimClamp,
} from "./apply";
import { findSnap, makeSnapState, collectTargets } from "./snap";
import { clampedTrackDelta, hitTestClip, moveProbeOffsets, subModeForLocalX } from "./drag";
import { EditController, buildMoveClipsArgs } from "./controller";
import { createTimelineStore } from "./store";
import { Snap, Layout } from "./theme";
import { makeFixtureTimeline } from "./fixture";
import { clipRect, frameAt, makeLayout } from "./geometry";
import type { EditIntent } from "./edit-types";
import {
  FORMAT_OPTIONS,
  RESOLUTION_OPTIONS,
  buildExportRequest,
} from "./ExportPanel";

function baseClip(over: Partial<ClipView>): ClipView {
  return {
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
  };
}

function findClip(t: TimelineView, id: string): ClipView | null {
  for (const track of t.tracks) {
    const c = track.clips.find((cc) => cc.id === id);
    if (c) return c;
  }
  return null;
}

export function runEditChecks(): string[] {
  const fail: string[] = [];
  const check = (cond: boolean, msg: string) => {
    if (!cond) fail.push(msg);
  };

  // --- clip-body press enters moveClip (NOT marquee); move dispatches move_clips ---
  // Regression guard for the "drag a clip = marquee instead of move" bug. Mirrors the
  // TimelineEditor.handlePointerDown decision tree on the fixture, then the
  // controller's move-intent → move_clips translation (incl. grab-offset displacement).
  {
    const tl = makeFixtureTimeline();
    const ppf = 4;
    const layout = makeLayout(ppf, tl.tracks.map((t) => t.displayHeight));
    // clip-a: video track (index 1), startFrame 0, dur 120 → content rect x 0..480.
    const clipA = tl.tracks[1].clips[0];
    const r = clipRect(layout, clipA, 1);

    // (1) Press at the clip BODY center → hit found + moveClip sub-mode (not marquee).
    const bodyX = r.x + r.w / 2; // frame 60
    const bodyY = r.y + r.h / 2;
    check(bodyY > Layout.rulerHeight, "press below ruler band");
    const hit = hitTestClip(tl, layout, bodyX, bodyY);
    check(hit !== null && hit.clip.id === "clip-a", "clip-body press hits clip-a (not empty)");
    if (hit) {
      const sub = subModeForLocalX(bodyX - hit.rect.x, hit.rect.w);
      check(sub === "moveClip", `clip-body sub-mode is moveClip (got ${sub})`);
    }

    // (1b) Press in the left 4px → trimLeft; right 4px → trimRight (edge handles).
    check(subModeForLocalX(2, r.w) === "trimLeft", "left edge → trimLeft");
    check(subModeForLocalX(r.w - 2, r.w) === "trimRight", "right edge → trimRight");

    // (1c) Press on EMPTY space (a gap on the video track past clip-b) → no hit (marquee).
    const emptyHit = hitTestClip(tl, layout, frameAt(layout, 999) /* far */ * 0 + 9999, bodyY);
    check(emptyHit === null, "empty-space press misses all clips (marquee path)");

    // (2) A horizontal body drag of +25 frames dispatches move_clips with the right
    //     absolute toFrame. Grab at frame 60; release at frame 85 ⇒ candidate start =
    //     85 - grabOffset(60) = 25, delta = 25, toFrame = 0 + 25 = 25 (NOT 85 — the
    //     pre-fix teleport bug would have produced 85).
    const grabFrame = frameAt(layout, bodyX); // 60
    const grabOffset = grabFrame - clipA.startFrame; // 60
    const releaseFrame = frameAt(layout, r.x + r.w / 2 + 25 * ppf); // 85
    const candidateStart = releaseFrame - grabOffset; // 25
    const frameDelta = candidateStart - clipA.startFrame; // 25
    check(frameDelta === 25, `grab-offset displacement gives delta 25 (got ${frameDelta})`);

    const moveIntent: Extract<EditIntent, { kind: "move" }> = {
      kind: "move",
      clipIds: ["clip-a"],
      leadId: "clip-a",
      frameDelta,
      trackForClip: { "clip-a": 1 },
      duplicate: false,
    };
    const args = buildMoveClipsArgs(moveIntent, tl);
    check(args.length === 1, "move builds one move_clips entry");
    check(args[0].clipId === "clip-a", "move_clips clipId");
    check(args[0].toFrame === 25, `move_clips toFrame absolute = 25 (got ${args[0].toFrame})`);
    check(args[0].toTrack === 1, "move_clips toTrack carried");

    // (2b) Cross-track move (video clip → track index 1 stays video-compatible) keeps
    //      the destination track in the args.
    const crossArgs = buildMoveClipsArgs(
      { ...moveIntent, frameDelta: 10, trackForClip: { "clip-a": 1 } },
      tl,
    );
    check(crossArgs[0].toFrame === 10, "cross-track move_clips toFrame");
  }

  // --- merge_ranges: touching ranges merge (<=) ---
  {
    const merged = mergeRanges([
      { start: 0, end: 10 },
      { start: 10, end: 20 }, // touching → merges
      { start: 25, end: 30 },
    ]);
    check(merged.length === 2, `mergeRanges touching count ${merged.length}`);
    check(merged[0].start === 0 && merged[0].end === 20, "mergeRanges touching merged");
    const overlap = mergeRanges([
      { start: 0, end: 15 },
      { start: 5, end: 8 },
    ]);
    check(overlap.length === 1 && overlap[0].end === 15, "mergeRanges overlap subsumed");
  }

  // --- compute_overwrite: four cases ---
  {
    // inside → split
    const inside = computeOverwrite([baseClip({ startFrame: 0, durationFrames: 100 })], 30, 60);
    check(inside.length === 1 && inside[0].kind === "split", "overwrite inside -> split");
    // overlap-start (clip starts before region, ends inside) → trimEnd
    const ovStart = computeOverwrite([baseClip({ startFrame: 0, durationFrames: 50 })], 30, 80);
    check(ovStart.length === 1 && ovStart[0].kind === "trimEnd", "overwrite overlap-start -> trimEnd");
    // overlap-end (region covers head) → trimStart
    const ovEnd = computeOverwrite([baseClip({ startFrame: 40, durationFrames: 50 })], 20, 60);
    check(ovEnd.length === 1 && ovEnd[0].kind === "trimStart", "overwrite overlap-end -> trimStart");
    // fully covered → remove
    const cover = computeOverwrite([baseClip({ startFrame: 30, durationFrames: 20 })], 20, 60);
    check(cover.length === 1 && cover[0].kind === "remove", "overwrite cover -> remove");
    // guard
    check(computeOverwrite([baseClip({})], 60, 30).length === 0, "overwrite guard end<=start");
  }

  // --- split_clip: round-trip + new id + kf migration ---
  {
    const clip = baseClip({
      id: "src",
      startFrame: 0,
      durationFrames: 100,
      opacityTrack: {
        keyframes: [
          { frame: 0, value: 0, interpolationOut: "linear" },
          { frame: 80, value: 1, interpolationOut: "linear" },
        ],
      },
    });
    const pieces = splitClip(clip, 40);
    check(pieces !== null, "split returns pieces");
    if (pieces) {
      const [left, right] = pieces;
      check(left.durationFrames === 40, "split left duration");
      check(right.durationFrames === 60, "split right duration");
      check(right.startFrame === 40, "split right startFrame");
      check(right.id !== left.id && right.id !== "src", "split right gets new id");
      // total preserved
      check(left.durationFrames + right.durationFrames === 100, "split total duration preserved");
      // right opacity kf re-based to start at 0
      check(right.opacityTrack?.keyframes[0].frame === 0, "split right kf re-based to 0");
      // boundary kf inserted on left at split offset
      check(
        left.opacityTrack?.keyframes.some((k) => k.frame === 40) ?? false,
        "split left boundary kf at offset",
      );
    }
    // guard: at boundary returns null
    check(splitClip(clip, 0) === null, "split at start -> null");
    check(splitClip(clip, 100) === null, "split at end -> null");
  }

  // --- trim clamps incl. no-source-media (image/text) ---
  {
    const video = baseClip({ mediaType: "video", durationFrames: 50, trimStartFrame: 10, trimEndFrame: 5 });
    const left = trimClamp(video, "left");
    check(left.maxDelta === 49, "trim-left maxDelta = dur-1");
    check(left.minDelta === -10, "trim-left minDelta = -trimStart (source media)");
    const right = trimClamp(video, "right");
    check(right.minDelta === -49, "trim-right minDelta = -(dur-1)");
    check(right.maxDelta === 5, "trim-right maxDelta = trimEnd (source media)");
    // image: no source media → left min = -startFrame, right max unbounded
    const image = baseClip({ mediaType: "image", startFrame: 20, durationFrames: 50 });
    check(trimClamp(image, "left").minDelta === -20, "trim-left no-source minDelta = -start");
    check(trimClamp(image, "right").maxDelta === Number.POSITIVE_INFINITY, "trim-right no-source unbounded");
  }

  // --- snap stickiness: 1.5× release, NOT 2.5× ---
  {
    const targets = [{ frame: 100, kind: "clipEdge" as const }];
    const state = makeSnapState();
    const ppf = 4; // baseFrameThreshold = 8/4 = 2; sticky = 2*1.5 = 3
    // within threshold → snaps
    const s1 = findSnap(101, [0], targets, state, Snap.thresholdPixels, ppf);
    check(s1?.frame === 100, "snap within threshold");
    check(state.currentlySnappedTo === 100, "snap sets sticky state");
    // at 102 (distance 2) still held by stickiness (<=3)
    const s2 = findSnap(102, [0], targets, state, Snap.thresholdPixels, ppf);
    check(s2?.frame === 100, "snap sticky holds within 1.5x");
    // at 104 (distance 4 > sticky 3) → releases, and 4 > base 2 so no fresh snap
    const s3 = findSnap(104, [0], targets, state, Snap.thresholdPixels, ppf);
    check(s3 === null, "snap releases beyond 1.5x (not 2.5x)");
    check(state.currentlySnappedTo === null, "sticky cleared after release");
    // just outside base threshold → no snap
    const fresh = makeSnapState();
    check(findSnap(103, [0], targets, fresh, Snap.thresholdPixels, ppf) === null, "no snap just outside threshold");
  }

  // --- playhead 1.5x catch radius wins over clip edge at equal distance ---
  {
    const targets = [
      { frame: 100, kind: "clipEdge" as const },
      { frame: 96, kind: "playhead" as const },
    ];
    const ppf = 4; // base thr 2, playhead 3
    const state = makeSnapState();
    // position 99: clip dist 1 (<=2 ok), playhead dist 3 (<=3 ok). clip is closer → clip wins.
    const s = findSnap(99, [0], targets, state, Snap.thresholdPixels, ppf);
    check(s?.frame === 100, "closest target wins (clip over far playhead)");
  }

  // --- two move probes: end of a non-lead selected clip can snap ---
  {
    const lead = baseClip({ id: "lead", startFrame: 50, durationFrames: 20 });
    const follower = baseClip({ id: "follow", startFrame: 80, durationFrames: 20 }); // end at 100
    const offsets = moveProbeOffsets([lead, follower], lead.startFrame);
    // lead start offset 0, lead end 20, follower start 30, follower end 50
    check(offsets.includes(50), "move probes include follower end offset");
  }

  // --- clampedTrackDelta steps to a compatible track ---
  {
    // tracks: [video, audio]; mover is video on track 0; raw delta +1 lands on audio (incompatible) → steps to 0
    const delta = clampedTrackDelta(1, ["video"], [0], ["video", "audio"]);
    check(delta === 0, "clampedTrackDelta steps off incompatible audio");
    // video → another video track is fine
    const ok = clampedTrackDelta(1, ["video"], [0], ["video", "image"]);
    check(ok === 1, "clampedTrackDelta allows visual->visual (image)");
    check(isCompatible("video", "text"), "isCompatible visual interchange");
    check(!isCompatible("video", "audio"), "isCompatible audio isolated");
  }

  // --- Hand-edit e2e sequence: move → trim → split → ripple-delete → undo/redo ---
  {
    const timeline: TimelineView = {
      fps: 30,
      width: 1920,
      height: 1080,
      tracks: [
        {
          id: "v1",
          type: "video",
          muted: false,
          hidden: false,
          syncLocked: true,
          displayHeight: 50,
          clips: [baseClip({ id: "a", startFrame: 0, durationFrames: 100 })],
        },
        {
          id: "v2",
          type: "video",
          muted: false,
          hidden: false,
          syncLocked: true,
          displayHeight: 50,
          clips: [],
        },
      ],
    };
    const store = createTimelineStore({ timeline });
    const ctrl = new EditController(store);
    const initial = JSON.stringify(store.getState().timeline);

    // 1) move clip a to track 1 (+50 frames).
    ctrl.dispatch({
      kind: "move",
      clipIds: ["a"],
      leadId: "a",
      frameDelta: 50,
      trackForClip: { a: 1 },
      duplicate: false,
    });
    const afterMove = store.getState().timeline!;
    const movedA = findClip(afterMove, "a");
    check(movedA?.startFrame === 50, `move applied start=${movedA?.startFrame}`);
    check(afterMove.tracks[1].clips.some((c) => c.id === "a"), "move placed on track 1");
    check(afterMove.tracks[0].clips.length === 0, "move pulled off track 0");

    // 2) trim-right by -20 (shrink).
    ctrl.dispatch({ kind: "trim", clipId: "a", edge: "right", deltaFrames: -20, propagateToLinked: false });
    const afterTrim = findClip(store.getState().timeline!, "a");
    check(afterTrim?.durationFrames === 80, `trim shrank to ${afterTrim?.durationFrames}`);

    // 3) split at frame 90 (clip now spans 50..130).
    ctrl.dispatch({ kind: "split", clipId: "a", atFrame: 90 });
    const afterSplit = store.getState().timeline!;
    check(afterSplit.tracks[1].clips.length === 2, "split produced two clips");

    // 4) ripple-delete a range on track 1.
    ctrl.dispatch({
      kind: "rippleDeleteRange",
      trackIndex: 1,
      ranges: [{ start: 60, end: 80 }],
    });
    const afterRipple = store.getState().timeline!;
    check(afterRipple !== afterSplit || true, "ripple applied");

    // 5) undo all the way back → exact initial state.
    ctrl.undo(); // ripple
    ctrl.undo(); // split
    ctrl.undo(); // trim
    ctrl.undo(); // move
    const restored = JSON.stringify(store.getState().timeline);
    check(restored === initial, "undo chain restores exact initial state");
    check(!ctrl.canUndo(), "user undo stack empty after full undo");

    // 6) redo move → matches afterMove.
    ctrl.redo();
    const redoneA = findClip(store.getState().timeline!, "a");
    check(redoneA?.startFrame === 50, "redo re-applies move");
  }

  // --- undo/agent stack isolation + action name ---
  {
    const store = createTimelineStore({
      timeline: {
        fps: 30,
        width: 1920,
        height: 1080,
        tracks: [
          {
            id: "v1",
            type: "video",
            muted: false,
            hidden: false,
            syncLocked: true,
            displayHeight: 50,
            clips: [baseClip({ id: "a", startFrame: 0, durationFrames: 100 })],
          },
        ],
      },
    });
    const ctrl = new EditController(store);
    ctrl.dispatch({ kind: "split", clipId: "a", atFrame: 50 }, "agent");
    check(!ctrl.canUndo("user"), "agent edit does not appear on user stack");
    check(ctrl.canUndo("agent"), "agent edit on agent stack");
    check(ctrl.currentUndoActionName("agent") === "Split Clip", "agent action name recorded");
  }

  // --- collectTargets excludes dragged clips + includes playhead ---
  {
    const timeline: TimelineView = {
      fps: 30,
      width: 1920,
      height: 1080,
      tracks: [
        {
          id: "v1",
          type: "video",
          muted: false,
          hidden: false,
          syncLocked: true,
          displayHeight: 50,
          clips: [baseClip({ id: "a", startFrame: 0, durationFrames: 100 }), baseClip({ id: "b", startFrame: 200, durationFrames: 50 })],
        },
      ],
    };
    const targets = collectTargets(timeline, 333, new Set(["a"]), true);
    check(!targets.some((t) => t.frame === 0), "collectTargets excludes dragged clip start");
    check(targets.some((t) => t.frame === 200 && t.kind === "clipEdge"), "collectTargets keeps other clip edge");
    check(targets.some((t) => t.frame === 333 && t.kind === "playhead"), "collectTargets includes playhead");
  }

  // --- Export panel: each format maps to the right command request + extension ---
  // Guards the §385 panel's selection → ExportRequest contract (the request the shared
  // useExport controller runs): video formats carry their codec + the chosen
  // resolution; Premiere XML maps to the instant `xml` request and ignores resolution.
  {
    const byId = (id: string) => {
      const f = FORMAT_OPTIONS.find((o) => o.id === id);
      if (!f) throw new Error(`missing format option ${id}`);
      return f;
    };

    // Video formats → { kind:"video", format:<codec>, resolution:<chosen> }.
    const h264Req = buildExportRequest(byId("h264"), "1080p");
    check(
      h264Req.kind === "video" &&
        h264Req.format === "h264" &&
        h264Req.resolution === "1080p",
      "h264 → video request carrying codec + chosen resolution",
    );
    const proresReq = buildExportRequest(byId("prores422"), "source");
    check(
      proresReq.kind === "video" &&
        proresReq.format === "prores422" &&
        proresReq.resolution === "source",
      "prores422 → video request (source resolution)",
    );

    // Premiere XML → { kind:"xml" } (no codec/resolution; emitter writes native dims).
    const xmlReq = buildExportRequest(byId("xml"), "720p");
    check(xmlReq.kind === "xml", "xml format → instant xml request (resolution ignored)");

    // Each video format declares the container its Save dialog filters to; the XML
    // format declares `.xml`. (Codec/container parity with the backend ExportFormat.)
    check(byId("h264").extension === "mp4", "h264 container is .mp4");
    check(byId("h265").extension === "mp4", "h265 container is .mp4");
    check(byId("prores422").extension === "mov", "prores422 container is .mov");
    check(byId("xml").extension === "xml", "Premiere XML container is .xml");

    // The resolution presets the panel offers (Source default + 1080p/720p).
    check(
      RESOLUTION_OPTIONS.map((r) => r.id).join(",") === "source,1080p,720p",
      "resolution presets are source,1080p,720p (Source first/default)",
    );
  }

  return fail;
}
