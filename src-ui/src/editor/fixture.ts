// Mock timeline fixture (E3-S9).
//
// The real `get_timeline` Tauri command does not exist yet (Epic 7). This fixture
// stands in as the data source so the canvas renders a representative timeline
// exercising every visual: video thumbnails, an audio waveform + volume rubber band
// with keyframes + fades, an image tile, a text clip, a lottie clip, a linked pair
// (underlined names), a missing-media clip (red wash), opacity keyframes, and
// opacity fade wedges. When the command lands, replace `makeFixtureTimeline()` with
// an adapter over the serde payload — the `TimelineView` shape is the contract.

import type { TimelineView } from "./types";

function syntheticWaveform(seed: number, n = 600): number[] {
  // dB-normalised peaks in [0,1] (0 = loud, 1 = silent), faithful to the reference
  // sample convention. Deterministic so golden render tests are stable.
  const out: number[] = [];
  let s = seed;
  for (let i = 0; i < n; i++) {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    const r = (s / 0x7fffffff) * 0.6;
    const env = 0.3 + 0.5 * Math.abs(Math.sin(i / 40));
    out.push(Math.min(1, Math.max(0, 1 - env + r * 0.4)));
  }
  return out;
}

export function makeFixtureTimeline(): TimelineView {
  return {
    fps: 30,
    width: 1920,
    height: 1080,
    tracks: [
      // Text track (top visual lane).
      {
        id: "track-text",
        type: "text",
        muted: false,
        hidden: false,
        syncLocked: true,
        displayHeight: 50,
        clips: [
          {
            id: "clip-title",
            name: "Opening Title",
            mediaRef: "title.txt",
            mediaType: "text",
            sourceClipType: "text",
            startFrame: 15,
            durationFrames: 90,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 20,
            fadeOutFrames: 20,
            fadeInInterpolation: "smooth",
            fadeOutInterpolation: "smooth",
          },
        ],
      },
      // Video track with a linked pair + a missing-media clip + opacity kfs.
      {
        id: "track-video",
        type: "video",
        muted: false,
        hidden: false,
        syncLocked: true,
        displayHeight: 50,
        clips: [
          {
            id: "clip-a",
            name: "intro.mp4",
            mediaRef: "intro.mp4",
            mediaType: "video",
            sourceClipType: "video",
            startFrame: 0,
            durationFrames: 120,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 0,
            fadeOutFrames: 0,
            fadeInInterpolation: "linear",
            fadeOutInterpolation: "linear",
            linkGroupId: "link-1",
            opacityTrack: {
              keyframes: [
                { frame: 0, value: 0, interpolationOut: "smooth" },
                { frame: 20, value: 1, interpolationOut: "linear" },
                { frame: 100, value: 1, interpolationOut: "linear" },
              ],
            },
          },
          {
            id: "clip-b",
            name: "broll.mp4",
            mediaRef: "broll.mp4",
            mediaType: "video",
            sourceClipType: "video",
            startFrame: 130,
            durationFrames: 150,
            trimStartFrame: 30,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 0,
            fadeOutFrames: 25,
            fadeInInterpolation: "linear",
            fadeOutInterpolation: "smooth",
          },
          {
            id: "clip-missing",
            name: "lost-clip.mp4",
            mediaRef: "lost-clip.mp4",
            mediaType: "video",
            sourceClipType: "video",
            startFrame: 300,
            durationFrames: 80,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 0,
            fadeOutFrames: 0,
            fadeInInterpolation: "linear",
            fadeOutInterpolation: "linear",
            isMissing: true,
          },
        ],
      },
      // Image track.
      {
        id: "track-image",
        type: "image",
        muted: false,
        hidden: false,
        syncLocked: true,
        displayHeight: 50,
        clips: [
          {
            id: "clip-logo",
            name: "logo.png",
            mediaRef: "logo.png",
            mediaType: "image",
            sourceClipType: "image",
            startFrame: 60,
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
          },
        ],
      },
      // Lottie track.
      {
        id: "track-lottie",
        type: "lottie",
        muted: false,
        hidden: false,
        syncLocked: true,
        displayHeight: 50,
        clips: [
          {
            id: "clip-anim",
            name: "burst.json",
            mediaRef: "burst.json",
            mediaType: "lottie",
            sourceClipType: "lottie",
            startFrame: 200,
            durationFrames: 60,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 0,
            fadeOutFrames: 0,
            fadeInInterpolation: "linear",
            fadeOutInterpolation: "linear",
          },
        ],
      },
      // Audio track (below visuals): waveform + volume rubber band + kfs + fades.
      {
        id: "track-audio",
        type: "audio",
        muted: false,
        hidden: false,
        syncLocked: true,
        displayHeight: 50,
        clips: [
          {
            id: "clip-music",
            name: "music.mp3",
            mediaRef: "music.mp3",
            mediaType: "audio",
            sourceClipType: "audio",
            startFrame: 0,
            durationFrames: 280,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 0.8,
            opacity: 1,
            fadeInFrames: 30,
            fadeOutFrames: 40,
            fadeInInterpolation: "smooth",
            fadeOutInterpolation: "linear",
            linkGroupId: "link-1",
            waveform: syntheticWaveform(42),
            volumeTrack: {
              keyframes: [
                { frame: 40, value: 0, interpolationOut: "linear" },
                { frame: 140, value: -6, interpolationOut: "smooth" },
                { frame: 240, value: -3, interpolationOut: "hold" },
              ],
            },
          },
          {
            id: "clip-vo",
            name: "voiceover.wav",
            mediaRef: "voiceover.wav",
            mediaType: "audio",
            sourceClipType: "audio",
            startFrame: 300,
            durationFrames: 120,
            trimStartFrame: 0,
            trimEndFrame: 0,
            speed: 1,
            volume: 1,
            opacity: 1,
            fadeInFrames: 0,
            fadeOutFrames: 0,
            fadeInInterpolation: "linear",
            fadeOutInterpolation: "linear",
            waveform: syntheticWaveform(7),
          },
        ],
      },
    ],
  };
}
