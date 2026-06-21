// Pure-math parity checks for the Inspector tab-body logic (E12-S3..S8).
//
// Framework-free + type-checked (same convention as `parity.checks.ts`): covered by
// `tsc --noEmit` and runnable directly (`_run-body-checks.mts`). Golden values
// verified against the reference scrub/format math + the E12-S1 VolumeScale.

import type { ClipView } from "../types";
import { VolumeScale } from "../theme";
import {
  clamp,
  clipPropertiesArgs,
  effectiveMultiplier,
  fadeFramesFromSeconds,
  fadeMaxSeconds,
  fadeSecondsFromFrames,
  formatBytes,
  formatScrub,
  formatVolumeDb,
  hasKeyframeAt,
  hexToRgba,
  nextKeyframeFrame,
  NEG_INF_DB,
  OPACITY_RANGE,
  parseScrub,
  positionRange,
  previousKeyframeFrame,
  rgbaToHex,
  sampleScalarTrack,
  SCALE_RANGE,
  scrubModifierMultiplier,
  scrubNext,
  sharedClipBool,
  sharedClipString,
  sharedClipValue,
  SPEED_RANGE,
  volumeDb,
  volumeLinearFromDb,
  VOLUME_DB_RANGE,
  type ScrubRange,
} from "./bodyLogic";

function eq<T>(label: string, got: T, want: T, out: string[]): void {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    out.push(`${label}: got ${JSON.stringify(got)} want ${JSON.stringify(want)}`);
  }
}

function approx(label: string, got: number, want: number, out: string[], eps = 1e-6): void {
  if (Math.abs(got - want) > eps) {
    out.push(`${label}: got ${got} want ${want} (±${eps})`);
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

export function runInspectorBodyChecks(): string[] {
  const out: string[] = [];

  // ── Scrub modifier multipliers ─────────────────────────────────────────────
  eq("mod.none", scrubModifierMultiplier("none"), 1, out);
  eq("mod.coarse", scrubModifierMultiplier("coarse"), 10, out);
  eq("mod.fine", scrubModifierMultiplier("fine"), 0.1, out);

  // ── scrubNext: next = clamp(start + dx*sens/mult) ──────────────────────────
  // Opacity: sens 0.01, mult 100, range 0..1. dx=+200 → 0.5 + 200*0.01/100 = 0.52
  approx("scrub.opacity", scrubNext(0.5, 200, OPACITY_RANGE, "none"), 0.52, out);
  // coarse ×10 → 0.5 + 200*0.1/100 = 0.7
  approx("scrub.opacity.coarse", scrubNext(0.5, 200, OPACITY_RANGE, "coarse"), 0.7, out);
  // fine ×0.1 → 0.5 + 200*0.001/100 = 0.502
  approx("scrub.opacity.fine", scrubNext(0.5, 200, OPACITY_RANGE, "fine"), 0.502, out);
  // clamp at max
  eq("scrub.clampMax", scrubNext(1, 99999, OPACITY_RANGE, "none"), 1, out);
  eq("scrub.clampMin", scrubNext(0, -99999, OPACITY_RANGE, "none"), 0, out);
  // Speed: sens 0.01, mult 1, range 0.25..4. dx=+100 → 1 + 100*0.01 = 2
  approx("scrub.speed", scrubNext(1, 100, SPEED_RANGE, "none"), 2, out);

  // effectiveMultiplier treats 0 as 1.
  eq("mult.zero", effectiveMultiplier(0), 1, out);
  eq("mult.nonzero", effectiveMultiplier(100), 100, out);

  // ── format + parse round-trips ─────────────────────────────────────────────
  eq("fmt.opacity", formatScrub(0.5, OPACITY_RANGE), "50 %", out);
  eq("fmt.scale", formatScrub(1.5, SCALE_RANGE), "150 %", out);
  eq("fmt.speed", formatScrub(1.5, SPEED_RANGE), "1.50 x", out);
  // parse divides by displayMultiplier + clamps.
  approx("parse.opacity", parseScrub("50 %", OPACITY_RANGE) ?? -1, 0.5, out);
  approx("parse.scale", parseScrub("150 %", SCALE_RANGE) ?? -1, 1.5, out);
  approx("parse.comma", parseScrub("1,50 x", SPEED_RANGE) ?? -1, 1.5, out);
  eq("parse.empty", parseScrub("", OPACITY_RANGE), null, out);
  eq("parse.junk", parseScrub("abc", OPACITY_RANGE), null, out);
  // parse clamps to range (200% opacity → 1.0)
  approx("parse.clamp", parseScrub("200 %", OPACITY_RANGE) ?? -1, 1, out);

  // Position range mult = canvas dimension.
  {
    const r: ScrubRange = positionRange(1920);
    eq("pos.mult", r.displayMultiplier, 1920, out);
    // 0.5 (normalised) shows 960 px.
    eq("pos.fmt", formatScrub(0.5, r), "960", out);
  }

  // ── sharedClipValue / bool / string ────────────────────────────────────────
  const c1 = baseClip({ id: "a", opacity: 0.5, speed: 2 });
  const c2 = baseClip({ id: "b", opacity: 0.5, speed: 1 });
  eq("shared.same", sharedClipValue([c1, c2], (c) => c.opacity), 0.5, out);
  eq("shared.mixed", sharedClipValue([c1, c2], (c) => c.speed), null, out);
  eq("shared.empty", sharedClipValue([], (c) => c.opacity), null, out);
  eq(
    "shared.bool.mixed",
    sharedClipBool([baseClip({}), baseClip({})], () => true),
    true,
    out,
  );
  eq(
    "shared.string.mixed",
    sharedClipString(
      [baseClip({ name: "x" }), baseClip({ name: "y" })],
      (c) => c.name,
    ),
    null,
    out,
  );

  // ── Volume / dB bridge (E12-S1) ────────────────────────────────────────────
  // linear 1.0 → 0 dB.
  approx("vol.unity", VolumeScale.dbFromLinear(1), 0, out);
  // floor: linear 0 → −60 dB → "−∞ dB" + store true-mute 0.
  eq("vol.floorText", formatVolumeDb(VolumeScale.floorDb), NEG_INF_DB, out);
  eq("vol.floorStore", volumeLinearFromDb(VolumeScale.floorDb), 0, out);
  eq("vol.belowFloorStore", volumeLinearFromDb(-100), 0, out);
  eq("vol.fmt", formatVolumeDb(-6), "-6.0 dB", out);
  eq("vol.range.min", VOLUME_DB_RANGE.min, -60, out);
  eq("vol.range.max", VOLUME_DB_RANGE.max, 15, out);
  // volumeDb samples the track at activeFrame when present.
  {
    const kfClip = baseClip({
      volume: 1,
      volumeTrack: { keyframes: [{ frame: 0, value: 0.5, interpolationOut: "linear" }] },
    });
    approx("vol.kfSample", volumeDb(kfClip, 0), VolumeScale.dbFromLinear(0.5), out);
    // No activeFrame → static scalar.
    approx("vol.staticFallback", volumeDb(kfClip), VolumeScale.dbFromLinear(1), out);
  }

  // ── Fade seconds/frames ────────────────────────────────────────────────────
  eq("fade.frames", fadeFramesFromSeconds(0.5, 30), 15, out);
  approx("fade.seconds", fadeSecondsFromFrames(15, 30), 0.5, out);
  // maxSeconds: single clip → duration/fps; else 60.
  approx("fade.max.single", fadeMaxSeconds([baseClip({ durationFrames: 90 })], 30), 3, out);
  eq("fade.max.multi", fadeMaxSeconds([baseClip({}), baseClip({})], 30), 60, out);

  // ── Keyframe sampling / nav ────────────────────────────────────────────────
  {
    const track = {
      keyframes: [
        { frame: 0, value: 10, interpolationOut: "linear" as const },
        { frame: 30, value: 20, interpolationOut: "linear" as const },
        { frame: 60, value: 30, interpolationOut: "linear" as const },
      ],
    };
    eq("kf.sample.before", sampleScalarTrack(track, 15, 0), 10, out);
    eq("kf.sample.at", sampleScalarTrack(track, 30, 0), 20, out);
    eq("kf.sample.fallback", sampleScalarTrack(null, 15, 99), 99, out);
    eq("kf.hasAt", hasKeyframeAt(track, 30), true, out);
    eq("kf.hasAt.no", hasKeyframeAt(track, 31), false, out);
    eq("kf.prev", previousKeyframeFrame(track, 45), 30, out);
    eq("kf.prev.none", previousKeyframeFrame(track, 0), null, out);
    eq("kf.next", nextKeyframeFrame(track, 45), 60, out);
    eq("kf.next.none", nextKeyframeFrame(track, 60), null, out);
  }

  // ── clipPropertiesArgs builder ─────────────────────────────────────────────
  eq(
    "args.basic",
    clipPropertiesArgs(["a", "b"], { volume: 0.5 }),
    { clipIds: ["a", "b"], volume: 0.5 },
    out,
  );
  eq(
    "args.dropUndefined",
    clipPropertiesArgs(["a"], { volume: undefined, opacity: 1 }),
    { clipIds: ["a"], opacity: 1 },
    out,
  );
  eq(
    "args.transform",
    clipPropertiesArgs(["a"], { transform: { centerX: 0.5, centerY: undefined } }),
    { clipIds: ["a"], transform: { centerX: 0.5 } },
    out,
  );
  // Empty transform patch is dropped.
  eq(
    "args.emptyTransform",
    clipPropertiesArgs(["a"], { transform: { centerX: undefined } }),
    { clipIds: ["a"] },
    out,
  );
  // Static rotation is a top-level scalar on set_clip_properties.
  eq(
    "args.rotation",
    clipPropertiesArgs(["a"], { rotation: 45 }),
    { clipIds: ["a"], rotation: 45 },
    out,
  );
  // Fade lengths (frames) are top-level scalars; fadeFramesFromSeconds feeds them.
  eq(
    "args.fades",
    clipPropertiesArgs(["a"], {
      fadeInFrames: fadeFramesFromSeconds(0.5, 30),
      fadeOutFrames: fadeFramesFromSeconds(1, 30),
    }),
    { clipIds: ["a"], fadeInFrames: 15, fadeOutFrames: 30 },
    out,
  );

  // ── Color hex round-trips ──────────────────────────────────────────────────
  eq("hex.white", rgbaToHex(1, 1, 1, 1), "#ffffffff", out);
  eq("hex.red", rgbaToHex(1, 0, 0, 1), "#ff0000ff", out);
  eq("hex.alpha", rgbaToHex(0, 0, 0, 0.5), "#00000080", out);
  {
    const p = hexToRgba("#ff0000");
    approx("hex.parse.r", p?.r ?? -1, 1, out);
    approx("hex.parse.g", p?.g ?? -1, 0, out);
    approx("hex.parse.a", p?.a ?? -1, 1, out);
  }
  eq("hex.parse.short", hexToRgba("#f00") !== null, true, out);
  eq("hex.parse.bad", hexToRgba("nope"), null, out);

  // ── clamp + byte formatter ─────────────────────────────────────────────────
  eq("clamp.mid", clamp(5, 0, 10), 5, out);
  eq("clamp.lo", clamp(-1, 0, 10), 0, out);
  eq("clamp.hi", clamp(11, 0, 10), 10, out);
  eq("bytes.b", formatBytes(512), "512 B", out);
  eq("bytes.kb", formatBytes(2048), "2.0 KB", out);
  eq("bytes.mb", formatBytes(5 * 1024 * 1024), "5.0 MB", out);
  eq("bytes.neg", formatBytes(-1), "—", out);

  return out;
}
