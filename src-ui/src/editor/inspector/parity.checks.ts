// Pure-math parity checks for the Inspector shell (E12-S2 acceptance).
//
// Framework-free + type-checked (same convention as the editor's
// `parity.checks.ts`): covered by `tsc --noEmit` so it can't drift from the
// types, and runnable directly (`_run-parity.mts`). Golden values verified
// against the reference `InspectorView.swift`:
//   headerTitle/headerIcon, availableTabs (order), aiEditEligible, activeTab,
//   resolvePreferredTab, formatDuration, formatAspectRatio, fileStem.

import type { ClipView, TimelineView } from "../types";
import {
  activeTab,
  aiEditEligible,
  availableTabs,
  fileStem,
  formatAspectRatio,
  formatDuration,
  gcd,
  middleTruncate,
  resolveHeader,
  resolveInspector,
  resolvePreferredTab,
  totalFrames,
} from "./logic";
import type {
  AccountState,
  InspectorInput,
  MediaAssetView,
} from "./types";

function eq<T>(label: string, got: T, want: T, out: string[]): void {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    out.push(`${label}: got ${JSON.stringify(got)} want ${JSON.stringify(want)}`);
  }
}

const baseClip = (over: Partial<ClipView>): ClipView => ({
  id: "c",
  name: "c",
  mediaRef: "asset-c",
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

function timelineWith(clips: ClipView[]): TimelineView {
  return {
    fps: 30,
    width: 1920,
    height: 1080,
    tracks: [
      { id: "t1", type: "video", muted: false, hidden: false, syncLocked: false, displayHeight: 50, clips },
    ],
  };
}

const OK_ACCOUNT: AccountState = { isMisconfigured: false };
const BAD_ACCOUNT: AccountState = { isMisconfigured: true };

function input(over: Partial<InspectorInput>): InspectorInput {
  return {
    timeline: null,
    selectedClipIds: new Set(),
    selectedMediaAssetIds: new Set(),
    mediaAssets: [],
    isMarqueeSelecting: false,
    account: OK_ACCOUNT,
    projectPath: null,
    ...over,
  };
}

const visualAsset = (id: string): MediaAssetView => ({ id, isVisual: true });
const audioAsset = (id: string): MediaAssetView => ({ id, isVisual: false });

export function runInspectorParityChecks(): string[] {
  const out: string[] = [];

  // ── Header / title / icon ──────────────────────────────────────────────────
  eq("header.timeline", resolveHeader(input({})), { title: "Timeline", icon: "info.circle" }, out);

  {
    const asset = visualAsset("a1");
    const i = input({ selectedMediaAssetIds: new Set(["a1"]), mediaAssets: [asset] });
    eq("header.source", resolveHeader(i), { title: "Source", icon: "info.circle" }, out);
  }

  {
    const clip = baseClip({ id: "v", mediaRef: "a1" });
    const i = input({
      timeline: timelineWith([clip]),
      selectedClipIds: new Set(["v"]),
      mediaAssets: [visualAsset("a1")],
    });
    eq("header.inspector.visual", resolveHeader(i), { title: "Inspector", icon: "slider.horizontal.3" }, out);
  }

  {
    const clip = baseClip({ id: "a", mediaType: "audio", sourceClipType: "audio", mediaRef: "au1" });
    const i = input({
      timeline: timelineWith([clip]),
      selectedClipIds: new Set(["a"]),
      mediaAssets: [audioAsset("au1")],
    });
    eq("header.inspector.audio", resolveHeader(i), { title: "Inspector", icon: "slider.horizontal.3" }, out);
  }

  // Marquee override: title always "Inspector" + slider icon.
  {
    const i = input({ isMarqueeSelecting: true, selectedClipIds: new Set(["x", "y"]) });
    eq("header.marquee", resolveHeader(i), { title: "Inspector", icon: "slider.horizontal.3" }, out);
    const st = resolveInspector(i, "video");
    eq("marquee.mode", st.mode, "marquee", out);
    eq("marquee.count", st.marqueeCount, 2, out);
  }

  // ── availableTabs (ORDER MATTERS) ──────────────────────────────────────────
  // Single text clip → [text]. Tab bar hidden (1 tab).
  // Text clips synthesize their content (hasNoSourceMedia) — `mediaRef` does NOT
  // resolve to a backing visual MediaAsset, so AI Edit is not offered.
  {
    const clip = baseClip({ id: "t", mediaType: "text", sourceClipType: "text", mediaRef: "text:body" });
    const i = input({
      timeline: timelineWith([clip]),
      selectedClipIds: new Set(["t"]),
      mediaAssets: [],
    });
    eq("tabs.singleText", availableTabs(i), ["text"], out);
    eq("tabs.singleText.showBar", resolveInspector(i, "video").showTabBar, false, out);
    // single text forces preferredTab "text".
    eq("preferred.singleText", resolvePreferredTab(i, "video"), "text", out);
    eq("active.singleText", activeTab(["text"], "video"), "text", out);
  }

  // Single video clip resolving to a visual asset, account OK → [video, ai].
  {
    const clip = baseClip({ id: "v", mediaRef: "a1" });
    const i = input({
      timeline: timelineWith([clip]),
      selectedClipIds: new Set(["v"]),
      mediaAssets: [visualAsset("a1")],
    });
    eq("tabs.singleVideo.ok", availableTabs(i), ["video", "ai"], out);
    eq("ai.eligible.singleVideo", aiEditEligible(i), true, out);
    // misconfigured account drops AI Edit.
    eq("tabs.singleVideo.bad", availableTabs({ ...i, account: BAD_ACCOUNT }), ["video"], out);
  }

  // Two video clips (not single) → [video] only, ai NOT eligible.
  {
    const c1 = baseClip({ id: "v1", mediaRef: "a1" });
    const c2 = baseClip({ id: "v2", mediaRef: "a2" });
    const i = input({
      timeline: timelineWith([c1, c2]),
      selectedClipIds: new Set(["v1", "v2"]),
      mediaAssets: [visualAsset("a1"), visualAsset("a2")],
    });
    eq("tabs.twoVideo", availableTabs(i), ["video"], out);
    eq("ai.eligible.twoVideo", aiEditEligible(i), false, out);
  }

  // Video + audio NOT linked → [video, audio], ai NOT eligible (audio not partner).
  {
    const v = baseClip({ id: "v", mediaRef: "a1" });
    const a = baseClip({ id: "a", mediaType: "audio", sourceClipType: "audio", mediaRef: "au1" });
    const i = input({
      timeline: timelineWith([v, a]),
      selectedClipIds: new Set(["v", "a"]),
      mediaAssets: [visualAsset("a1"), audioAsset("au1")],
    });
    eq("tabs.videoAudio.unlinked", availableTabs(i), ["video", "audio"], out);
    eq("ai.eligible.videoAudio.unlinked", aiEditEligible(i), false, out);
  }

  // Linked video+audio pair → counts as one: ai ELIGIBLE → [video, audio, ai].
  {
    const v = baseClip({ id: "v", mediaRef: "a1", linkGroupId: "g1" });
    const a = baseClip({ id: "a", mediaType: "audio", sourceClipType: "audio", mediaRef: "au1", linkGroupId: "g1" });
    const i = input({
      timeline: timelineWith([v, a]),
      selectedClipIds: new Set(["v", "a"]),
      mediaAssets: [visualAsset("a1"), audioAsset("au1")],
    });
    eq("tabs.videoAudio.linked", availableTabs(i), ["video", "audio", "ai"], out);
    eq("ai.eligible.videoAudio.linked", aiEditEligible(i), true, out);
  }

  // Single visual clip whose asset is NOT visual (e.g. missing/audio) → no AI.
  {
    const clip = baseClip({ id: "v", mediaRef: "au1" });
    const i = input({
      timeline: timelineWith([clip]),
      selectedClipIds: new Set(["v"]),
      mediaAssets: [audioAsset("au1")],
    });
    eq("tabs.nonVisualAsset", availableTabs(i), ["video"], out);
    eq("ai.eligible.nonVisualAsset", aiEditEligible(i), false, out);
  }

  // ── activeTab + resolvePreferredTab (leaving text → video) ─────────────────
  eq("active.preferredAvailable", activeTab(["video", "audio"], "audio"), "audio", out);
  eq("active.fallbackFirst", activeTab(["video", "audio"], "ai"), "video", out);
  eq("active.empty", activeTab([], "video"), null, out);
  {
    // current "text" but selection no longer single text → drop to "video".
    const c1 = baseClip({ id: "v1", mediaRef: "a1" });
    const c2 = baseClip({ id: "v2", mediaRef: "a2" });
    const i = input({
      timeline: timelineWith([c1, c2]),
      selectedClipIds: new Set(["v1", "v2"]),
      mediaAssets: [visualAsset("a1"), visualAsset("a2")],
    });
    eq("preferred.leavingText", resolvePreferredTab(i, "text"), "video", out);
  }

  // ── Project / Format metadata math ─────────────────────────────────────────
  eq("gcd.1920x1080", gcd(1920, 1080), 120, out);
  eq("aspect.1920x1080", formatAspectRatio(1920, 1080), "16:9", out);
  eq("aspect.1080x1920", formatAspectRatio(1080, 1920), "9:16", out);
  eq("aspect.640x480", formatAspectRatio(640, 480), "4:3", out);

  // formatDuration: M:SS below an hour, H:MM:SS at/over an hour; minutes unpadded.
  eq("dur.0", formatDuration(0), "0:00", out);
  eq("dur.5s", formatDuration(5), "0:05", out);
  eq("dur.65s", formatDuration(65), "1:05", out);
  eq("dur.605s", formatDuration(605), "10:05", out);
  eq("dur.3661s", formatDuration(3661), "1:01:01", out);
  eq("dur.round", formatDuration(89.6), "1:30", out); // rounds to 90s

  // fileStem strips dir + extension.
  eq("stem.win", fileStem("C:\\Users\\me\\My Project.palmier"), "My Project", out);
  eq("stem.posix", fileStem("/home/me/clip.final.palmier"), "clip.final", out);
  eq("stem.noext", fileStem("/home/me/Untitled"), "Untitled", out);

  // totalFrames = max clip end across tracks; duration via fps.
  {
    const t = timelineWith([
      baseClip({ id: "a", startFrame: 0, durationFrames: 90 }),
      baseClip({ id: "b", startFrame: 60, durationFrames: 120 }), // ends 180
    ]);
    eq("totalFrames", totalFrames(t.tracks), 180, out);
    const i = input({ timeline: t, projectPath: "/x/Demo.palmier" });
    const st = resolveInspector(i, "video");
    eq("project.mode", st.mode, "project", out);
    eq("project.name", st.noSelection?.project?.name, "Demo", out);
    eq("format.resolution", st.noSelection?.format?.resolution, "1920 × 1080", out);
    eq("format.frameRate", st.noSelection?.format?.frameRate, "30 fps", out);
    eq("format.aspect", st.noSelection?.format?.aspectRatio, "16:9", out);
    eq("format.duration", st.noSelection?.format?.duration, "0:06", out); // 180/30 = 6s
  }

  // No project path → Project section omitted, Format still present.
  {
    const i = input({ timeline: timelineWith([]), projectPath: null });
    const st = resolveInspector(i, "video");
    eq("project.noPath.project", st.noSelection?.project, null, out);
    eq("project.noPath.format.present", st.noSelection?.format !== null, true, out);
  }

  // Middle-truncation keeps head + tail, elides the middle.
  eq("middleTruncate.short", middleTruncate("abc", 10), "abc", out);
  eq(
    "middleTruncate.long",
    middleTruncate("0123456789ABCDEF", 9),
    "0123…CDEF",
    out,
  );

  return out;
}
