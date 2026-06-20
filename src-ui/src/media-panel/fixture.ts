// Mock/fixture media library — the data source the panel renders until the real
// Tauri media commands land (Epic 7: `get_media`/`import`; Epic 11: `search_media`).
// `MediaPanelController` reads from this; swapping in `invoke('get_media')` is the
// only change when Epic 7 lands (see controller.ts TODO(E7)).

import type {
  GenJob,
  MediaFolderView,
  MediaSnapshot,
  MediaAssetView,
} from "./types";

const folders: MediaFolderView[] = [
  { id: "f-shoot", name: "Shoot 2026-06", parentFolderId: null },
  { id: "f-bcam", name: "B-Cam", parentFolderId: "f-shoot" },
  { id: "f-music", name: "Music Beds", parentFolderId: null },
  { id: "f-graphics", name: "Graphics", parentFolderId: null },
];

const assets: MediaAssetView[] = [
  {
    id: "a-intro",
    name: "intro_take3.mp4",
    path: "C:\\Media\\Shoot\\intro_take3.mp4",
    type: "video",
    folderId: "f-shoot",
    durationSeconds: 42.5,
    isGenerated: false,
  },
  {
    id: "a-broll",
    name: "bcam_pan_01.mov",
    path: "C:\\Media\\Shoot\\BCam\\bcam_pan_01.mov",
    type: "video",
    folderId: "f-bcam",
    durationSeconds: 18.2,
    isGenerated: false,
  },
  {
    id: "a-bgm",
    name: "ambient_bed.wav",
    path: "C:\\Media\\Music\\ambient_bed.wav",
    type: "audio",
    folderId: "f-music",
    durationSeconds: 130.0,
    isGenerated: false,
  },
  {
    id: "a-gen-music",
    name: "generated_score.mp3",
    path: "C:\\Media\\Music\\generated_score.mp3",
    type: "audio",
    folderId: "f-music",
    durationSeconds: 60.0,
    isGenerated: true,
  },
  {
    id: "a-logo",
    name: "logo_overlay.png",
    path: "C:\\Media\\Graphics\\logo_overlay.png",
    type: "image",
    folderId: "f-graphics",
    durationSeconds: null,
    isGenerated: false,
  },
  {
    id: "a-gen-bg",
    name: "ai_backdrop.png",
    path: "C:\\Media\\Graphics\\ai_backdrop.png",
    type: "image",
    folderId: "f-graphics",
    durationSeconds: null,
    isGenerated: true,
  },
  {
    id: "a-lower3rd",
    name: "lower_third.json",
    path: "C:\\Media\\Graphics\\lower_third.json",
    type: "lottie",
    folderId: "f-graphics",
    durationSeconds: 3.0,
    isGenerated: false,
  },
  {
    id: "a-title",
    name: "title_card",
    path: "",
    type: "text",
    folderId: null,
    durationSeconds: null,
    isGenerated: false,
  },
  {
    id: "a-root-clip",
    name: "unsorted_clip.mp4",
    path: "C:\\Media\\unsorted_clip.mp4",
    type: "video",
    folderId: null,
    durationSeconds: 9.8,
    isGenerated: false,
  },
  {
    id: "a-missing",
    name: "relink_me.mov",
    path: "C:\\Media\\Shoot\\relink_me.mov",
    type: "video",
    folderId: "f-shoot",
    durationSeconds: 5.0,
    isGenerated: false,
    missing: true,
  },
];

export function makeFixtureSnapshot(): MediaSnapshot {
  // Deep-ish copy so consumers can mutate (folder create/rename) without touching
  // the canonical fixture.
  return {
    folders: folders.map((f) => ({ ...f })),
    assets: assets.map((a) => ({ ...a })),
  };
}

/** A couple of in-flight + failed generation jobs for the E4-S11 panel demo. */
export function makeFixtureJobs(): GenJob[] {
  return [
    {
      id: "job-1",
      prompt: "cinematic aerial over a coastline at golden hour",
      model: "kling-v2",
      status: { kind: "running", progress: 0.42 },
      createdAt: 3,
    },
    {
      id: "job-2",
      prompt: "uplifting orchestral score, 60s",
      model: "music-gen-v1",
      status: { kind: "queued" },
      createdAt: 2,
    },
    {
      id: "job-3",
      prompt: "neon city street, rain",
      model: "kling-v2",
      status: { kind: "failed", message: "model timed out after 120s" },
      createdAt: 1,
    },
  ];
}
