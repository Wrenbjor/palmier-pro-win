---
kind: doc
domain: [build-orchestration]
type: decision
status: adopted
links: [[build-orchestration]]
source_reference: ../palmier-pro (palmier-io/palmier-pro, GPLv3, macOS Swift reference)
---

# Palmier-Pro-Windows — Foundation Specification

> **Status:** Foundation document. Source of truth for PRD and execution-plan agents.
> **Scope:** Clean-room Windows + Linux implementation of the Palmier Pro feature set, derived from
> the open-source macOS reference at `palmier-io/palmier-pro` (GPLv3).
> **Out of scope:** Sharing code with the Swift reference. Mac platform. iOS/iPadOS.
>
> _Filed verbatim from the kickoff input (2026-06-20). The macOS reference is checked out at
> `../palmier-pro/` and verified to contain the files this spec cites (`AgentInstructions.swift`,
> `AppTheme.swift`, `Resources/Fonts/`, `mcpb/`)._

---

## 1. PROJECT CHARTER

### 1.1 What we are building
A native **Windows-first (Linux-second)** AI-driven non-linear video editor whose strategic
differentiator is **agent-controlled timeline editing via a local MCP server**. The editor is the
surface that LLMs operate on, not just a tool that humans operate with.

### 1.2 Why
The reference product solves the problem only on macOS. We are extending it to the platforms where
most short-form social-media content actually gets produced and where most AI-tooling users already
live. The local-MCP centerpiece makes this product compatible with the existing Claude Code / Claude
Desktop / Cursor / Codex workflows that operate cross-platform.

### 1.3 Primary user workflows (drive every feature decision)
1. **Long-form-to-shorts:** User records a 25-min monologue → AI transcribes → AI proposes cuts
   (dead-air removal, highlight clipping) → user reviews → exports a feed of vertical shorts.
2. **B-roll-directed short:** User dumps a folder of clips + a script → AI assembles a directed cut,
   inserting B-roll at semantic moments → user refines.
3. **Generative-augmented edit:** User opens a project → asks agent for a missing transition / title
   card / VO line → agent generates via provider → drops it on the timeline.
4. **Hand editing with agent on standby:** User edits like Premiere; agent only acts when invited.
   Edits must be undoable independently.
5. **Export to social platform:** Finished cut exports to an external TypeScript social-media
   generation platform (out of repo) via a defined handoff.

### 1.4 Non-goals (do not implement)
- 8K / cinema-grade color grading. Reference targets 1080p–4K social formats; we match.
- Multi-user collaborative editing. Single-user, single-machine, local-first.
- Cloud project storage. Projects are local file bundles.
- Mac support. Linux is in scope only as a stretch from the same codebase.
- Sharing code or runtime with the Swift reference.

### 1.5 Success criteria
- Feature parity with macOS reference for the workflows in §1.3.
- MCP server compatible with Palmier's existing `.mcpb` client manifest (Claude Desktop / Cursor /
  Claude Code connect with no client-side changes — only the server URL differs in install instructions).
- 4K timeline preview scrubs at 30+ fps on a mid-range GPU (RTX 4060 / Radeon 7600 class).
- Cold start to editable timeline under 3 seconds on NVMe.
- All AI-mutated timeline operations are atomically undoable.

---

## 2. STACK (LOCKED — do not relitigate)

### 2.1 Foundation

| Layer | Choice | Reasoning anchor |
|---|---|---|
| Application shell | **Tauri 2** | Native WebView2 (Win) / WebKitGTK (Linux), tiny binary, built-in updater, code signing, MSI/AppImage/.deb output |
| Backend language | **Rust 2024 edition** | Single language across composition, MCP, project I/O, AI clients; aligns with agent codegen quality |
| Frontend language | **TypeScript + React 19** | Mature ecosystem, agent-friendly, can share design tokens with the external social platform later |
| Renderer (timeline canvas + preview) | **WebGPU via `wgpu` crate**, exposed to webview via shared canvas | D3D12 on Windows, Vulkan on Linux, one API. Cross-platform compute for visual search + compositing |
| Media decode/encode | **FFmpeg via `ffmpeg-next`** (preferred), `gstreamer-rs` fallback for hardware-pipeline edge cases | Battle-tested, NVENC/QSV/AMF/VAAPI HW accel, MIT/LGPL aligns with GPL distribution |
| Audio engine | **`cpal`** playback, **`symphonia`** decode (MP3/AAC/FLAC/WAV/Ogg), **`rubato`** resampling | Pure Rust, deterministic, no platform glue |
| State management (UI) | **Zustand** (global), **TanStack Query** (backend RPCs), **Immer** (immutable timeline mutations) | Lighter than Redux, agent-friendly |
| Styling | **Tailwind CSS 4** + CSS variables from the design-token JSON (§9) | Tokens map 1:1 from reference AppTheme |
| Build / package | **pnpm + Vite** (frontend), **Cargo workspaces** (Rust) | Standard, fast, no surprises |

### 2.2 External integrations (locked)

| Concern | Choice |
|---|---|
| Auth | **Clerk** — same provider as reference. Embed via `@clerk/clerk-react`. Token forwarded to backend for paid-tier access. |
| Backend RPC | **Convex** — same backend as reference, accessed via HTTP (no native Rust SDK; use `reqwest`). Hosts AI provider keys, billing, sample catalog, generation queue. |
| AI generation providers | **Convex-proxied** (we never hold provider keys). Catalog: Seedance, Kling, Veo, Nano Banana Pro, GPT Image, Lyria 3 Pro, others — surfaced via `palmier://models/*` MCP resources. |
| LLM for in-app agent | **Anthropic Messages API** directly with user-supplied key (BYOK) OR Convex-proxied for signed-in/paid users. Models: Claude Sonnet 4.6, Opus 4.8, Haiku 4.5. |
| Updater | **Tauri 2 built-in updater** (Ed25519 signing, signed JSON manifest). Reference uses Sparkle/EdDSA — same signing model. |
| Crash reporting | **Sentry** Rust SDK (backend) + Browser SDK (frontend). Same DSN scheme as reference, injected at build time via Tauri config. |
| Transcription | **whisper.cpp via `whisper-rs`**, CPU or CUDA/Vulkan/DirectML. Reference uses Apple Speech; we use Whisper for on-device parity. |
| Visual semantic search | **CLIP via `candle`** or **`ort`** (ONNX runtime). Reference embeds video frames and queries with text. |
| Tokenizers | **`tokenizers` crate** from HuggingFace (Whisper + CLIP + transcript tooling). |

### 2.3 What we explicitly do NOT do
- No mpv-embedded playback. We render via wgpu to composite layers in real time.
- No Electron. No CEF outside Tauri.
- No Qt. No GTK directly. No WPF / WinUI / .NET runtime dependency.
- No mixed-language agent code (no C++ glue, no FFI to non-Rust crates beyond standard bindings).
- No bundling provider API keys. All paid generation flows through Convex.

---

## 3. TARGET PLATFORMS

| OS | Min version | Arch | Notes |
|---|---|---|---|
| Windows | Windows 10 22H2, Windows 11 | x86_64 | WebView2 runtime auto-installed via Tauri bootstrapper. D3D12 required. |
| Linux | Ubuntu 22.04 / Fedora 40 / Arch (rolling) | x86_64 | WebKitGTK 2.40+, Vulkan 1.2+ required. AppImage + .deb + .rpm artifacts. |

**GPU floor:** D3D12 feature level 12_0 / Vulkan 1.2 with 4 GB VRAM. Below that, fall back to CPU
compositing with FFmpeg `libavfilter` for export and a degraded preview (no live keyframe
interpolation, frame-stepped scrub only).

---

## 4. HIGH-LEVEL ARCHITECTURE

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Tauri Shell (Rust binary)                                                  │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │ WebView (React + TS)                                                  │ │
│  │  • Home / Project Browser                                            │ │
│  │  • Editor (Timeline, Preview, Media Panel, Inspector, Agent Panel)   │ │
│  │  • Settings   • Design system (CSS vars from token JSON)             │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│        ▲  Tauri IPC (commands + events) + shared WebGPU surface  ▼         │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │ Rust Workspace (core crates)                                         │ │
│  │  palmier-model      — Timeline, Track, Clip, Keyframe, MediaAsset    │ │
│  │  palmier-project    — .palmier bundle I/O, registry, autosave        │ │
│  │  palmier-media      — FFmpeg decode/encode, thumbnails, waveforms    │ │
│  │  palmier-engine     — Composition graph, wgpu compositor, transport  │ │
│  │  palmier-text       — Text layout, font registry, caption styling    │ │
│  │  palmier-edit       — Ripple/overwrite, snap, trim, split engines    │ │
│  │  palmier-history    — Undo/redo stacks (user + agent separated)      │ │
│  │  palmier-export     — H.264/H.265/ProRes export, FCP7 XML emitter    │ │
│  │  palmier-transcribe — whisper.cpp wrapper, word/segment alignment    │ │
│  │  palmier-search     — CLIP frame index + transcript full-text        │ │
│  │  palmier-gen        — Convex client, generation lifecycle, queue     │ │
│  │  palmier-agent      — Anthropic + Palmier-proxy clients, SSE         │ │
│  │  palmier-mcp        — MCP server (HTTP on 127.0.0.1:19789)           │ │
│  │  palmier-tools      — Shared tool dispatch (used by MCP + agent)     │ │
│  │  palmier-auth       — Clerk token cache, account state               │ │
│  │  palmier-update     — Tauri updater glue                             │ │
│  │  palmier-telemetry  — Sentry + tracing-subscriber                    │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────┘
          │ HTTP (loopback 127.0.0.1:19789)
          ▼
     External MCP clients (Claude Desktop, Claude Code, Cursor, Codex)
```

**Strict layering rule:** Frontend never touches FFmpeg / wgpu / file system directly. All side
effects go through Tauri commands; all reactive state flows back via Tauri events. The MCP server and
the in-app agent invoke the **same** `palmier-tools` dispatcher — exactly one tool implementation per
tool name, no duplication.

---

## 5. DATA MODEL (authoritative)

All editing operations mutate this model. Persisted as JSON in `timeline.json` inside the `.palmier`
project bundle.

### 5.1 Timeline (root)
```rust
struct Timeline {
    fps: u32,                       // 24 | 25 | 30 | 50 | 60. Frozen after first clip.
    width: u32,                     // pixels
    height: u32,                    // pixels
    settings_configured: bool,      // true after first project save
    tracks: Vec<Track>,
}
// Computed: total_frames = max(track.end_frame for track in tracks)
```

### 5.2 Track
```rust
struct Track {
    id: Uuid,
    track_type: ClipType,           // video | audio | image | text | lottie
    muted: bool,                    // default false — audio silent in render
    hidden: bool,                   // default false — invisible in render
    sync_locked: bool,              // default true — ripple followers
    clips: Vec<Clip>,               // sorted by start_frame, no overlaps
    #[serde(skip)] display_height: f32, // UI only, default 50, reset on open
}
// Computed: end_frame = max(clip.end_frame for clip in clips)
```

### 5.3 Clip — the core entity
```rust
struct Clip {
    id: Uuid,
    media_ref: Uuid,                // points to MediaAsset.id
    media_type: ClipType,
    source_clip_type: ClipType,     // original type (for derived clip color-coding)

    // Timeline placement
    start_frame: i32,
    duration_frames: i32,           // > 0 invariant

    // Source-media trims (in SOURCE frames at SOURCE fps, not timeline frames)
    trim_start_frame: i32,          // default 0
    trim_end_frame: i32,            // default 0

    // Playback
    speed: f64,                     // default 1.0; <1 stretches, >1 shortens
    volume: f64,                    // default 1.0 (linear gain)

    // Fades (in TIMELINE frames at TIMELINE fps)
    fade_in_frames: i32,            // default 0
    fade_out_frames: i32,           // default 0
    fade_in_interpolation: Interpolation,   // default linear
    fade_out_interpolation: Interpolation,  // default linear

    // Static visual properties (sampled when no keyframe track is active)
    opacity: f64,                   // default 1.0
    transform: Transform,
    crop: Crop,

    // Linking
    link_group_id: Option<Uuid>,    // video+audio pair tracking
    caption_group_id: Option<Uuid>,

    // Text-only fields
    text_content: Option<String>,
    text_style: Option<TextStyle>,

    // Keyframe tracks — Some when animation is authored, None otherwise
    opacity_track:  Option<KeyframeTrack<f64>>,
    position_track: Option<KeyframeTrack<AnimPair>>, // (top_left_x, top_left_y) in 0..1 canvas space
    scale_track:    Option<KeyframeTrack<AnimPair>>, // (width, height) in 0..1 canvas space
    rotation_track: Option<KeyframeTrack<f64>>,      // degrees
    crop_track:     Option<KeyframeTrack<Crop>>,
    volume_track:   Option<KeyframeTrack<f64>>,      // dB, floor -120
}

// Derived (do not persist)
fn end_frame(&self) -> i32 { self.start_frame + self.duration_frames }
fn source_frames_consumed(&self) -> i32 { (self.duration_frames as f64 * self.speed).round() as i32 }
fn source_duration_frames(&self) -> i32 { self.source_frames_consumed() + self.trim_start_frame + self.trim_end_frame }
```

### 5.4 Transform / Crop / TextStyle
```rust
struct Transform {
    top_left: (f64, f64),     // normalized 0..1 canvas (NOT center)
    width: f64,                // normalized 0..1
    height: f64,               // normalized 0..1
    rotation: f64,             // degrees, rotation about clip center
    flip_horizontal: bool,
    flip_vertical: bool,
}

struct Crop {
    left: f64, top: f64, right: f64, bottom: f64,  // normalized 0..1 in source space
}

struct TextStyle {
    font_name: String,
    font_size: f64,                      // points
    font_scale: f64,                     // multiplier for transform-driven resize
    color: Color,                        // RGBA
    alignment: TextAlignment,            // left | center | right
    background: Option<Fill>,
    border: Option<Border>,              // color + width
    shadow: Option<Shadow>,              // color + offset + radius + opacity
}
```

### 5.5 Keyframes
```rust
enum Interpolation { Linear, Hold, Smooth }   // Smooth uses smoothstep(t)
struct Keyframe<V> { frame: i32, value: V, interpolation_out: Interpolation }
struct KeyframeTrack<V> { keyframes: Vec<Keyframe<V>> }  // sorted by frame, unique frames
struct AnimPair { a: f64, b: f64 }   // used for position (x,y) and scale (w,h)
```

**Sampling rules (deterministic):**
- Empty track → fallback value.
- Single keyframe → that value.
- Frame ≤ first keyframe → first value.
- Frame ≥ last keyframe → last value.
- Between A and B: `t = (frame - A.frame) / (B.frame - A.frame)`.
  - Hold → `A.value`.
  - Linear → `lerp(A, B, t)`.
  - Smooth → `lerp(A, B, smoothstep(t))` where `smoothstep(t) = t*t*(3 - 2*t)`.
- Keyframe frames are clip-relative offsets in storage. Public API converts to absolute timeline frames.
- Animatable properties (enum): `opacity, position, scale, rotation, crop, volume`.

### 5.6 MediaAsset + MediaManifest
```rust
struct MediaAsset {
    id: Uuid,
    name: String,
    asset_type: ClipType,
    folder_id: Option<Uuid>,
    duration_seconds: Option<f64>,
    source_width: Option<u32>,
    source_height: Option<u32>,
    source_fps: Option<f32>,
    has_audio: bool,
    created_at: DateTime<Utc>,
    generation_input: Option<GenerationInput>,    // None if user-imported
    generation_status: GenerationStatus,           // None | Generating | Downloading | Rendering | Failed(String)
    cached_remote_url: Option<String>,
    cached_remote_url_expires_at: Option<DateTime<Utc>>,
}

enum MediaSource {
    External { absolute_path: PathBuf },           // referenced in place
    Project { relative_path: PathBuf },            // copied into project/media/
}

struct MediaManifest {
    version: u32,                                  // current 2
    entries: Vec<MediaManifestEntry>,
    folders: Vec<MediaFolder>,
}

struct MediaFolder { id: Uuid, name: String, parent_id: Option<Uuid> }
```

### 5.7 Project bundle layout (`.palmier` is a directory)
```
MyProject.palmier/
├── timeline.json                    # required
├── manifest.json                    # MediaManifest
├── generation_log.json              # optional, agent + generation history
├── thumbnail.jpg                    # optional, <=320x180
├── media/                           # internalized media files
│   └── <id-prefix>.<ext>
└── chatsessions/                    # JSON files, one per session
    └── <session-uuid>.json
```

**Critical:** On Windows we use the same directory-as-document convention. The file picker should show
`.palmier` directories as a single document. Implement via Tauri custom file dialog and explorer
extension. On Linux: behaves naturally as a directory.

---

## 6. FEATURE MODULES (detailed)

### 6.1 App shell + project lifecycle

**Boot sequence (Rust `main.rs`):**
1. Install crash handler (Sentry + native panic hook).
2. Initialize tracing subscriber.
3. Read settings: `%APPDATA%\PalmierProWin\settings.json` (Windows) / `~/.config/palmier-pro/settings.json` (Linux).
4. Configure Clerk + Convex clients from build-time config (compiled in via Tauri).
5. Load `ModelCatalog` (Convex `/v1/models` GET, cached for 24h).
6. Start MCP server if `settings.mcp_enabled` (default true).
7. Initialize Tauri window for Home screen.

**Windows:**
- Home window: 1200×1200 default, 760×480 min. Project browser + Recent + Sample carousel + Welcome overlay (dismissed via `has_seen_welcome` setting).
- Project window: 1600×1000 default, 960×600 min. Editor.
- One project window per project. Switching projects auto-saves the previous.

**Main menu items** (replicate exactly — same keyboard shortcuts):

| Menu | Item | Shortcut | Action |
|---|---|---|---|
| Palmier Pro | About | — | Show about dialog |
| | Check for Updates | — | Tauri updater check |
| | Settings | Ctrl+, | Open settings window |
| | Quit | Ctrl+Q | Quit |
| File | New | Ctrl+N | Create project (file dialog) |
| | Open | Ctrl+O | Open project (file dialog) |
| | Save | Ctrl+S | Persist timeline + manifest |
| | Save As | Ctrl+Shift+S | Copy bundle to new location |
| | Import Media | Ctrl+I | Open file picker, multi-select |
| | Export | Ctrl+E | Open export panel |
| Edit | Undo | Ctrl+Z | User undo stack |
| | Redo | Ctrl+Shift+Z | User redo stack |
| | Cut/Copy/Paste/Select All | Ctrl+X/C/V/A | Clipboard for clips |
| | Split at Playhead | Ctrl+K | Split selected clip at playhead |
| | Trim Start to Playhead | Q | Set trim_start so clip begins at playhead |
| | Trim End to Playhead | W | Set trim_end so clip ends at playhead |
| | Delete | Backspace/Del | Remove selected clips |
| View | Media Panel | Ctrl+0 | Toggle |
| | Inspector | Ctrl+Alt+0 | Toggle |
| | Agent Panel | Ctrl+Alt+A | Toggle |
| | Maximize Focused Panel | ` (backtick) | Maximize |
| | Layout → Default/Media/Vertical | Ctrl+1/2/3 | Preset panel layouts |
| | Enter Full Screen | F11 (Windows) | Toggle fullscreen |
| Help | Tutorial | — | Start tutorial walkthrough |
| | Keyboard Shortcuts | ? | Help window, shortcuts tab |
| | MCP Instructions | — | Help window, MCP tab |
| | Send Feedback | — | Feedback dialog |

_Note: Mac shortcuts use Cmd; Windows/Linux use Ctrl. F11 replaces Cmd+F for fullscreen on Windows
(Linux follows desktop convention)._

**Project registry:**
- `%APPDATA%\PalmierProWin\registry.json` — `Vec<ProjectEntry>` with `{ id, url, created_date, last_opened_date }`.
- Methods: register, remove, delete (move to Recycle Bin / Trash), update_url (after Save As), sorted_entries (newest first).

**Sample projects:**
- Convex `GET /v1/samples` → `[{slug, title, poster_url}]`.
- `GET /v1/samples/resolve?slug=<slug>` → `{project, manifest, downloads[], chat_sessions, generation_log, poster_url}`.
- Materialize to `%APPDATA%\PalmierProWin\Samples\<slug>\` as `.palmier` bundle.
- Progress callback during media downloads.

### 6.2 Media panel
Three tabs: **Media, Captions, Music.**

**Media tab — features:**
- Drag-drop import from system file manager. Multi-file. Multi-folder (recursive, mirrors directory tree as folder hierarchy).
- Supported extensions:
  - Video: `.mov, .mp4, .m4v`
  - Audio: `.mp3, .wav, .aac, .m4a`
  - Image: `.png, .jpg, .jpeg, .tiff, .heic, .webp`
  - Lottie: `.json, .lottie`
- Sort modes: `dateAdded | name | duration`.
- Filter chips: video | audio | image; AI-generated only toggle.
- View modes: folder (tree) | flat | grouped.
- Thumbnail size slider: 80–200 px.
- Inline folder creation + rename. Marquee selection. Full-text search across asset names.
- Visual search via CLIP (§6.10). Transcript search across all audio/video clips (§6.10).
- Search result panel with two sections: visual hits (frame grid) + spoken hits (transcript segments with timecodes).
- Generation panel below media list: pending generations with progress + cancel.

_Drag-drop on Windows/Linux: Tauri's native drop event (`window.onFileDropEvent`). The reference's
macOS-specific AppKit drop-area workaround is not relevant here._

**Thumbnail + waveform generation:**
- Video thumbnail strip: extract N frames via FFmpeg seek + scale to 120×68. Cache to `%APPDATA%\PalmierProWin\Cache\thumbnails\<sha256>.bin` (sequence of JPEG-encoded frames).
- Audio waveform: decode via symphonia, downsample to ~2000 amplitude samples per minute, store as `Vec<f32>` in `.../waveforms/<sha256>.bin`.
- Concurrency gates: 2 concurrent waveform extractions, 4 concurrent image thumbnails (mirror reference). In-flight tracking prevents duplicate work.

**Captions tab:** displays transcription results + caption generation controls. See §6.9.
**Music tab:** built-in music library from Convex `/v1/music`. Browse + audition + drag to audio track.

### 6.3 Timeline editor — input + visuals

**Geometry:**
- Horizontal axis: time in frames, with adjustable `pixels_per_frame` (zoom).
- Vertical axis: stacked tracks, each at its `display_height` (default 50 px). Video/image/text/lottie above, audio below (separator).
- Playhead: vertical line; `current_frame` is the source of truth.

**Tool modes (`ToolMode`):** Pointer (V) — select / move / trim (default). Razor (C) — click a clip to split at that frame.

**Selection model:** Single click selects (clears others unless Shift/Ctrl). Shift-click adds. Ctrl-click toggles. Marquee drag (empty area) rubber-band selects. Selection persists across re-renders; identified by clip IDs.

**Drag operations (LMB on selected clips):**
- Move: drag body → translate `start_frame`. Cross-track allowed if compatible (video ↔ image; audio ↔ audio; text/lottie own-type only).
- Trim left: drag left edge → adjust `start_frame` + `trim_start_frame` + `duration_frames` simultaneously.
- Trim right: drag right edge → adjust `duration_frames` + `trim_end_frame`.
- Slip (Alt + drag middle): change `trim_start_frame` and `trim_end_frame` together without changing timeline position. (Reference uses native AppKit gesture; replicate via modifier key.)
- Slide (Ctrl+Alt + drag): move clip while adjusting neighbors' trims to keep total duration. **Stretch goal — verify against reference before implementing.**

**Linked clips:** Clips sharing `link_group_id` move together (auto-created on video-with-audio import). Setting timing-relevant properties propagates to all linked clips. Setting volume/opacity/transform/text fields does NOT propagate.

**Snap behavior (`SnapEngine`):**
- Snap targets: every clip edge (start + end) on every track + the playhead.
- Probe offsets: the moving clip provides `[0, duration_frames]` so both its start and end can snap.
- Base threshold: 8 pixels (configurable). Convert to frame threshold via `pixels_per_frame`.
- Playhead snap threshold = base × 1.5 (priority).
- Sticky snap: once snapped, stay snapped until moved 2.5× the threshold away.
- On snap activation: trigger haptic feedback equivalent (Windows: `mciSendString("set")`; no-op or skip; Linux: skip). Reference uses `NSHapticFeedbackManager` — silent on our platforms.

**Range selection (`TimelineRangeSelection`):** Shift-drag on empty timeline area selects a time range. Exposes `start_frame, end_frame, fps, start_timecode, end_timecode, semantics`. Drives ripple-delete-range tool and several MCP tools.

**Playhead behavior:** Single-click ruler → seek. Shift+arrow → frame step. J/K/L → reverse / pause / play. Space → play/pause. Home/End → first / last frame.

**Visual rendering of clips on the timeline canvas:** Video = track-color + thumbnail strip. Audio = filled + waveform overlay (centered baseline, 0 silent, 1 peak). Image = filled + center-cropped image. Text = filled + text preview. Lottie = type color + Lottie icon. Generating = pulsing overlay + spinner + provider logo. Missing media = red diagonal wash + "Offline" badge. Selected = 2 px white outline. Linked partners = 1 px subtle outline. Volume rubber band = line across audio clips for `volume_at(frame)` (drag to set; Alt-drag inserts keyframe). Opacity envelope = line across video clips for `opacity_at(frame)`.

### 6.4 Editing engines (pure functions, no UI dependency)

**RippleEngine:**
```rust
fn compute_ripple_shifts(clips: &[Clip], removed_ids: &HashSet<Uuid>) -> Vec<ClipShift>;
fn compute_ripple_shifts_for_ranges(clips: &[Clip], removed_ranges: &[FrameRange]) -> Vec<ClipShift>;
fn compute_ripple_push(clips: &[Clip], insert_frame: i32, push_amount: i32, exclude_ids: &HashSet<Uuid>) -> Vec<ClipShift>;
fn merge_ranges(ranges: &[FrameRange]) -> Vec<FrameRange>;
```
Algorithm: Sort removed ranges by start; merge overlapping. For each clip on the affected tracks, sum
the lengths of all removed ranges whose end `<= clip.start_frame`; that sum is the leftward shift. For
inserts: any clip with `start_frame >= insert_frame` (and not excluded) shifts by `push_amount`.

**Sync-locked ripple:** When `track.sync_locked == true`, ripples computed on one track apply to clips on all sync-locked tracks within the same frame range. Linked clips ride along.

**OverwriteEngine:** Given an insertion at `(track_index, start_frame, duration)`, returns clips to delete + clips to trim. Cases: insertion inside an existing clip → split host at insertion start and end, delete middle. Overlaps start → trim host's start to insertion end. Overlaps end → trim host's end to insertion start. Covers multiple → delete all + trim partial bookends. Used by: drag-drop from media panel, agent `add_clips`, paste.

**Split (`split_clip`):** Insert a new clip with `start_frame = split_frame`, `duration_frames = original.end_frame - split_frame`, `trim_start_frame = original.trim_start_frame + (split_frame - original.start_frame) * speed`. Original's `duration_frames` shortens. Keyframes inside the post-split region migrate to the new clip with frame offsets recomputed.

### 6.5 Preview composition + playback engine
**The largest replacement of Apple APIs.** Replace `AVMutableComposition` / `AVAssetExportSession` /
`AVPlayer` with a Rust composition graph rendered via wgpu, sourced from FFmpeg-decoded video frames.

```rust
struct CompositionFrame {
    frame_index: i32,
    layers: Vec<LayerRender>,    // bottom-to-top stacking; track order = render order
}
enum LayerRender {
    Video { texture: GpuTexture, transform: Mat3, opacity: f32, crop: CropRect },
    Image { texture: GpuTexture, transform: Mat3, opacity: f32, crop: CropRect },
    Text  { glyphs: Vec<GlyphRun>, transform: Mat3, opacity: f32 },
    Lottie { texture: GpuTexture, transform: Mat3, opacity: f32 },  // pre-rendered to texture
}
```

**Decode pipeline:** One `DecoderThread` per source asset URL (FFmpeg `AVFormatContext` + `AVCodecContext` with HW decoder when available). Frames pushed into an LRU `FrameCache` keyed by `(media_ref, source_frame)`. Cache size: 1.5 GB VRAM ceiling for textures + 512 MB system RAM for decoded YUV planes. Eviction: distance from current playhead.

**Frame composition (every visible timeline frame):**
1. For each track bottom→top: for each clip overlapping the current frame:
   - Sample animated properties (opacity, transform, scale, rotation, crop, volume) at this frame.
   - Convert timeline frame → source frame using `start_frame`, `trim_start_frame`, `speed`.
   - Request decoded frame from cache; if missing, block briefly or render previous frame + queue a decode (interactive scrub mode).
   - Append a `LayerRender` to the composition.
2. Text clips: build a `GlyphRun` array via `cosmic-text` (cross-platform shaping + layout).
3. Render via wgpu pipeline: textured quads with affine transforms + opacity blending; text via SDF or rasterized glyph atlas.
4. Present the rendered texture to the WebGPU canvas in the webview.

**Audio mixing:** For each timeline frame range played, for each overlapping audio clip: decode via symphonia, resample to project rate (48 kHz), apply time-stretch for `speed != 1.0` (`rubato`, or `signalsmith-stretch` Rust port for pitch-preserving stretch), apply per-frame volume envelope from keyframes + fades. Sum all clips' buffers. Output via `cpal`.

**Transport modes (`SeekMode`):** Exact (zero tolerance, reset cache as needed; playback start, frame stepping). InteractiveScrub (serve nearest available frame; queue precise decode; throttle to one redraw per `1/fps` s; scrub gesture + live-keyframe edits).

**Playback transport API (to frontend):** `play()`, `pause()`, `toggle_playback()`, `seek(frame, mode)`, `step(delta_frames)`, `current_frame` reactive value (Tauri event stream).

**Multiple preview tabs (`PreviewTab`):** `.timeline` (main composition, always present, not closable). `.media_asset { id, name, type }` (single asset preview, closable). Tab bar above viewport, horizontally scrollable with nav arrows. Per-tab playback state.

**Preview viewport overlays:** Transform overlay (4 corner handles, edge handles, rotation handle, center drag; pink center-to-center snap guides). Crop overlay (rule-of-thirds guides; pan inside + resize edges; aspect-lock toggle). Both counter-rotate to clip-local axes for accurate manipulation.

**Aspect / quality / zoom menu:** Aspect 16:9, 9:14, 9:16, 1:1, 4:3, 2.4:1. Frame rate 24, 25, 30, 50, 60 fps. Quality 720p, 1080p, 2K, 4K. Zoom 25, 50, 75, fit, 125, 150, 200 %.

### 6.6 Text rendering
- `cosmic-text` for layout + shaping + multi-line, plus `fontdb` for font discovery (includes user-installed fonts on Windows + Linux).
- Bundle the same font files the reference ships (`Sources/PalmierPro/Resources/Fonts/`).
- Render to a glyph atlas texture via wgpu, or emit individual textured quads per glyph for small clips.
- Text style (color, background fill, border, shadow) as shader uniforms on the text pass.
- Preview-mode: maintain a long-lived render tree; preroll text-clip rasterization 30 frames before its start (mirror reference's CALayer preroll).
- Export-mode: rasterize entire text track to overlay textures per output frame, composite into export pipeline.

### 6.7 Inspector panel
Header: icon + title. Title = "Timeline" (nothing selected → project metadata), "Inspector" (clips selected), "Source" (media asset selected).

No-selection: Section Project (name, file path); Section Format (resolution, frame rate, aspect ratio, duration).

**Per-selection tabs:**

| Tab | Visible when | Contents |
|---|---|---|
| Text | All selected clips are text | Typography (font picker, size 12–300), Appearance (color, opacity, background toggle+color, border toggle+color+width, shadow toggle+color+offset+blur), Layout (alignment, X/Y position), Content (multi-line editor — debounced) |
| Video | At least one visual clip | Transform (position X/Y, scale W/H, rotation, crop toggle); Playback (speed slider 0.25×–4.0×); Keyframes toggle (opens side panel) |
| Audio | At least one audio clip | Levels (volume dB −120…0, fade in/out seconds); Playback (speed if no video selected); Keyframes toggle |
| AI Edit | Single clip + AI-eligible + signed-in | AI-edit controls (re-prompt the clip via generation) |
| Details | Media asset selected | Read-only metadata + folder + rename |

**Components:** `ScrubbableNumberField` (input + drag-to-scrub; Ctrl-drag = fine 0.1×; range validation, display multiplier, custom formatter; live `on_change` + final `on_commit`). `ColorField` (native color picker). `FontPickerField` (system + bundled). `InspectorPositionFields` (X/Y pair with steppers). `KeyframesLane` (horizontal lane of keyframe dots; click empty = add, click dot = select, drag = move, right-click = interpolation menu).

**Keyframes side panel (toggle on, single-clip):** One lane per animatable property the clip has tracks for, plus "+" to add a track. Diamond glyph per keyframe. Click → select, drag → move, Delete → remove, right-click → set interpolation. "Stamp" button per property adds a keyframe at the current playhead capturing the current static value.

### 6.8 Toolbar
Horizontal strip above the editor; spacer pushes zoom right.

| Group | Buttons |
|---|---|
| Undo/Redo | Undo (Ctrl+Z), Redo (Ctrl+Shift+Z) |
| Tool mode | Pointer (V), Razor (C) |
| Clip edit | Split at Playhead (Ctrl+K), Trim Start (Q), Trim End (W) |
| Insert | Add Text (T) |
| Zoom | Slider, log-mapped from `min_zoom_scale` to `max_zoom_scale` |

### 6.9 Transcription + captions
**Transcription:** `whisper-rs` wrapping whisper.cpp. Ship `small.en` bundled; offer `medium.en` + `large-v3` as optional downloads. Compile with CUDA + Vulkan backends on Windows, Vulkan on Linux; CPU fallback. Workflow: extract audio via FFmpeg → 16 kHz mono 16-bit PCM → run Whisper → word + segment timestamps. Profanity censoring: replace matched words with bracketed equivalents (or use Whisper tokenizer suppression). Locale detected from Whisper output, user-overridable. Cache key: `sha256(file_content) + model_id + language`.

```rust
struct TranscriptionWord { text: String, start: f64, end: f64 }       // seconds
struct TranscriptionSegment { text: String, start: f64, end: f64 }    // seconds
struct TranscriptionResult { text: String, language: String, words: Vec<TranscriptionWord>, segments: Vec<TranscriptionSegment> }
```

**Caption builder (`CaptionBuilder`):**
1. Per segment: split text to fit on screen (recursive — fit → return; else break on sentence `.!?` → clause `,;:` → midpoint).
2. Distribute time proportionally by character count.
3. Enforce minimum display duration (default 0.7 s); prevent overlap by cascading later phrases.
4. Per phrase, compute timeline frame range by mapping source seconds through the clip's trim + speed.
5. Emit `TextClipSpec` records.

**Caption tab UI:** Run transcription button. Style controls (font, size, color, background, position centerX/centerY, text case upper/lower/title, profanity censor toggle, language picker). Generated captions create a single shared text track or a `caption_group_id` linking all generated clips.

**Transcript-driven cut** (reference commit `561f04d`): Agent reads transcript via `get_transcript` → identifies dead-air/filler ranges in source seconds → converts to project frames via placement/trim/speed → calls `ripple_delete_ranges`; engine cuts + closes gaps.

### 6.10 Search
**Visual search:** Embed every imported video frame (sampled every N seconds — default 1 fps for videos < 5 min, 2-second interval otherwise) via CLIP image encoder. Embed query text via CLIP text encoder. Cosine similarity → top-K per shot. Index: `<project>/.search/visual_index.bin` — packed `(media_ref, source_seconds, embedding_f16[512])`. States: `ready | indexing | model_not_installed | downloading_model | preparing | disabled | failed`.

**Transcript (spoken) search:** Index all transcribed segments + words. Exact keyword match + semantic match via embedding (BGE-small or all-MiniLM-L6 via `candle`). Always available — no model download for keyword mode.

**Search-result navigation:** Click hit → jump preview to source seconds + select asset. "Use as B-roll" → drop on timeline at playhead.

### 6.11 AI generation
Convex holds provider keys, billing, queue. Client submits jobs, polls/subscribes for status, downloads outputs.

**Catalog (`palmier://models/{video,image,audio,upscale}` resources):** Not hard-coded; fetched from Convex `/v1/models`. Each model carries `name, kind, durations, aspect_ratios, resolutions, qualities, max_reference_images/videos/audios, supports_first_frame/last_frame/source_video/image_reference, voices, audio_category (tts/music/sfx), credits_per_second/credits_per_image/credits_per_thousand_chars, audio_discount_rate`. Validate per call against the catalog before submitting.

**Lifecycle:**
1. User/agent selects model + parameters.
2. Optionally upload reference media via Convex pre-signed upload tickets. Cache uploaded URLs against `MediaAsset` to dedupe.
3. Create placeholder `MediaAsset`s (N for batch image; 1 for video/audio/upscale) with `generation_status = Generating`.
4. Submit job via Convex `generations:submit` mutation → `job_id`.
5. Subscribe to `generations:by_id(job_id)` live query (WebSocket or long-poll fallback).
6. On `Generating → Succeeded`: download output to `<project>/media/<id>.{ext}` (auto-correct extension); status → Downloading → None; notify UI.
7. On failure: status → `Failed(reason)`; toast + inspector error.

**Generation panel UI:** Cards per active job (thumbnail/preview, prompt, model, status, progress, cancel). Failed jobs persist until dismissed.

**System notification on completion:** Windows native toast via `tauri-plugin-notification` → "<name> generated". Click → reveal asset + jump to it.

**Cost gate:** `can_generate = signed_in && tier_allows && has_remaining_credits`. Block generation UI when false. Show "Sign in" / "Out of credits".

### 6.12 Export

| Mode | Output | Codec choices | Format |
|---|---|---|---|
| Video | Single file | H.264, H.265, ProRes 4444 (alpha) | `.mp4` (h264/h265), `.mov` (prores) |
| XML | Single `.xml` | — | FCP 7 XMEML 4 (for Premiere import) |
| Palmier Project | `.palmier` directory | — | Self-contained bundle (collected media) |

Resolution presets: 720p, 1080p, 4K (scales short side; preserves aspect).

**Video export pipeline:** Configure FFmpeg muxer + encoder. For each output frame `0..total_frames * (output_fps / project_fps)`: build composition (same path as preview) → render to wgpu texture → read back to system memory (or zero-copy via NVENC when possible) → push to FFmpeg `AVPacket` queue. Mix audio in parallel → AAC track. Mux + finalize. Progress via Tauri events (0.0–1.0). Cancellation: flag picked up by the render loop each frame boundary.

**XML export (XMEML 4, Premiere):** `<xmeml version="4">` → `<sequence>` (name, duration, rate, timecode, media). Video tracks emitted in reverse order (FCP 7 is bottom-up; ours top-down). Per-clip `<clipitem>`: master clip ID per `(mediaRef, type)`, duration, rate, in/out/start/end frames, file ref, filters. Filters: time remap (speed %), audio levels (linear gain, clamp 3.98), basic motion (scale %, rotation negated, center normalized), crop, opacity (keyframes → setOpacity). Fades: single-sided dissolve to black/silence; alignment start-black or end-black; Premiere uses cut-point ticks for video fade-out. Linked clip cross-refs for A/V pairs. **Not supported by XML:** text overlays, flips, custom keyframe easing (imports with default). Timecode: drop-frame `;` for 29.97/59.94, NDF `:` otherwise.

**Palmier Project export (self-contained):** Rewrite all `MediaSource::External` → `MediaSource::Project { relative_path }`, copy files into `media/`. Copy generation log + chat sessions + thumbnail. Report `collected, copied_internal, missing, total_bytes`.

**Social-platform handoff (out-of-scope, defined here):** "Export to Social" mode produces standard MP4 + sidecar JSON `<export>.palmier-meta.json` (transcript, chapter markers, AI-suggested captions, source project hash). The TypeScript social platform consumes this sidecar.

### 6.13 Agent panel (in-app chat)
**UI:** collapsible right-side panel. Top: floating tab bar (open sessions, "+" new chat, clock = history). Middle: scrolling message list, auto-scroll + jump-to-bottom. Bottom: multi-line editor, @mention picker, send/cancel, model picker (BYOK = all; signed-in = tier-allowed), API-key indicator. Empty state: 7 starter prompts (generate B-roll, generate opening, captions, VO, music, organize media, transcript-driven cut).

```rust
enum AgentContentBlock {
    Text(String),
    ToolUse { id: String, name: String, input_json: serde_json::Value },
    ToolResult { tool_use_id: String, content: Vec<ToolResultBlock>, is_error: bool },
}
enum ToolResultBlock { Text(String), Image { base64: String, media_type: String } }
struct AgentMessage { id: Uuid, role: Role, blocks: Vec<AgentContentBlock>, mentions: Vec<AgentMention>, context_hint: Option<String> }
struct ChatSession { id: Uuid, title: String, updated_at: DateTime<Utc>, messages: Vec<AgentMessage>, is_open: bool }
```

**Session persistence:** Each session → `<project>/chatsessions/<uuid>.json`. Written on tab close + new-session creation. Loaded sorted by `updated_at` desc.

**Mentions (`AgentMentionContext`):** User types `@AssetName` / `@ClipLabel` / `@TimelineRange` — autocomplete from project state. On send, each mention emits a JSON context-hint block prepended to the user message: `mediaAsset: {kind, media_ref, type, [inlined?, inline_error?]}` (images inlined as base64); `timelineClip: {kind, clip_id, clip: {...}}`; `timelineRange: {kind, timeline_range: {start_frame, end_frame, fps, start_timecode, end_timecode}}`.

**Client selection (`AgentService`):** Anthropic key in OS keyring → `AnthropicClient`. Else signed in via Clerk → `PalmierClient` (Convex-proxied). Else → "sign in or add API key" inline.

**API key storage:** Windows = Credential Manager via `keyring` crate. Linux = Secret Service (libsecret) via same crate. Key name: `palmier-pro-anthropic-api-key`.

**Anthropic client (BYOK):** `POST https://api.anthropic.com/v1/messages`, headers `x-api-key`, `anthropic-version: 2023-06-01`, `accept: text/event-stream`. SSE via `reqwest_eventsource`. Body: model, `max_tokens 8192`, system block with `cache_control: ephemeral`, messages (text + image + tool_result), tools. Prompt caching on system block AND last user-message block (ephemeral).

**PalmierClient (Convex-proxied):** `POST {convex_http_url}/v1/agent/stream`, `Authorization: Bearer <clerk_jwt>`. Identical body + SSE protocol.

**Model availability:** BYOK = all (Sonnet 4.6 / Opus 4.8 / Haiku 4.5). Free tier (signed in, no sub) = Haiku 4.5 only. Paid tier = Sonnet 4.6 (and Opus if Convex catalog enables).

**Streaming events → UI:** `message_start` (capture usage: input/cache-create/cache-read tokens), `text_delta` (append), `tool_use_complete(id, name, json)` (append ToolUse, queue for execution), `message_stop(reason)` (tool_use → execute + loop; end_turn → stop).

**Tool execution loop:** After `message_stop(reason="tool_use")`, find all ToolUse blocks → dispatch each to `palmier_tools::execute(name, args)` synchronously within the chat task → collect results → append a user message of ToolResult blocks → resume streaming with accumulated history. Cancellation drops the in-flight assistant turn cleanly (no half-written ToolUse committed).

### 6.14 MCP server (the strategic centerpiece)
**Binding:** HTTP on `127.0.0.1:19789` (default; same as reference for client compatibility). Configurable in settings. Listener: `axum` with TCP listener restricted to `IpAddr::V4(Ipv4Addr::LOCALHOST)`.

**Endpoints:** `POST /mcp` (JSON-RPC over HTTP, single-shot or batched). `GET /.well-known/oauth-protected-resource` → `{"resource":"http://127.0.0.1:19789"}` for Claude Desktop one-click install handshake.

**Validators (request middleware):** (1) Origin validator — require Origin header missing OR `Origin: null` OR `http://127.0.0.1:19789`; reject anything else (defense against drive-by browser attacks). (2) Content-type — require `application/json`. (3) Protocol version — enforce MCP spec version in `mcp-protocol-version` header.

**Server identity (Initialize response):**
```json
{
  "name": "palmier-pro",
  "version": "1.0.0",
  "instructions": "<full text from §7.2>",
  "capabilities": {
    "resources": {"subscribe": false, "listChanged": false},
    "tools":     {"listChanged": false}
  }
}
```

**MCP library:** Use `rmcp` (official Rust MCP SDK) for protocol scaffolding. Wire its tool registry to our `palmier_tools` crate. No protocol re-implementation.

**Tools:** The reference exposes **36 tools**. The Windows port implements the same 36 with identical names, parameters, and semantics. The reference's `AgentInstructions.swift` is the source of truth for the `instructions` field and tool descriptions; port verbatim (replacing macOS-specific phrasing only where required).

**Tool catalogue:**

| Tool | Category | Mutation | Async | Inputs | Output (key fields) |
|---|---|---|---|---|---|
| `get_timeline` | Read | No | No | `start_frame?, end_frame?` | fps, width/height, total_frames, tracks[{type, clips[{id, media_ref, start/duration/trim, speed, volume, opacity, transform, crop, keyframes, text_style?, caption_group?}]}], can_generate |
| `get_media` | Read | No | No | — | assets[{id, name, type, duration, generation_status, folder_id}] |
| `inspect_media` | Read | No | Yes | `media_ref, clip_id?, max_frames?, start_seconds?, end_seconds?, word_timestamps?, overview?` | image bytes / video sample frames + transcript / audio transcript / Lottie frames, paginated |
| `get_transcript` | Read | No | No | `start_frame?, end_frame?, clip_id?` | clips[{clip_id, words[{text, start_frame, end_frame}]}], paginated |
| `inspect_timeline` | Read | No | Yes | `start_frame, end_frame?, max_frames?` | composited frames as base64 PNGs |
| `search_media` | Read | No | Yes | `query, scope?, media_ref?, limit?` | hits[{score, media_ref, range?, image?}], visual_status |
| `list_models` | Read | No | No | `type?` | models[…], loaded |
| `list_folders` | Read | No | No | — | folders[{id, name, parent_folder_id}] |
| `add_clips` | Edit | Yes | No | `entries[{media_ref, track_index?, start_frame, duration_frames}]` | new clip ids |
| `remove_clips` | Edit | Yes | No | `clip_ids[]` | — |
| `remove_tracks` | Edit | Yes | No | `track_indexes[]` | — |
| `move_clips` | Edit | Yes | No | `moves[{clip_id, to_track?, to_frame?}]` | — |
| `set_clip_properties` | Edit | Yes | No | `clip_ids[]` + any of: duration_frames, trim_start/end, speed, volume, opacity, transform, content, font_name, font_size, color, alignment | — |
| `set_keyframes` | Edit | Yes | No | `clip_id, property, keyframes[[frame, …values, interp?]]` | — |
| `split_clip` | Edit | Yes | No | `clip_id, at_frame` | — |
| `ripple_delete_ranges` | Edit | Yes | No | `track_index OR clip_id, ranges[[start, end]], units?` | new layout |
| `undo` | Edit (reverse) | Yes | No | — | — |
| `add_texts` | Edit | Yes | No | `entries[{track_index?, start_frame, duration_frames, content, transform?, font_name?, font_size?, color?, alignment?}]` | new clip ids |
| `add_captions` | Edit | Yes | Yes | `clip_ids[], language?, font_name?, font_size?, color?, center_x?, center_y?, text_case?, censor_profanity?` | — |
| `generate_video` | Generate | Yes | Async | `prompt, name?, model?, duration?, aspect_ratio?, resolution?, start_frame_media_ref?, end_frame_media_ref?, source_video_media_ref?, source_clip_id?, reference_image_media_refs[]?, reference_video_media_refs[]?, reference_audio_media_refs[]?, folder_id?` | placeholder_asset_id |
| `generate_image` | Generate | Yes | Async | `prompt, name?, model?, aspect_ratio?, resolution?, quality?, reference_media_refs[]?, folder_id?` | placeholder_asset_ids[] |
| `generate_audio` | Generate | Yes | Async | `prompt, name?, model?, voice?, lyrics?, style_instructions?, instrumental?, duration?, video_source_start_frame?, video_source_end_frame?, video_source_media_ref?, folder_id?` | placeholder_asset_id |
| `upscale_media` | Generate | Yes | Async | `media_ref, model?, source_clip_id?` | placeholder_asset_id |
| `import_media` | Library | Yes | Yes | `source {url \| path \| bytes, mime_type?}, name?, folder_id?` | new media_ref |
| `create_folder` | Library | Yes | No | `name, parent_folder_id?` \| `entries[]` | folder_id(s) |
| `move_to_folder` | Library | Yes | No | `asset_ids[], folder_id?` \| `entries[]` | — |
| `rename_media` | Library | Yes | No | `media_ref, name` \| `entries[]` | — |
| `rename_folder` | Library | Yes | No | `folder_id, name` \| `entries[]` | — |
| `delete_media` | Library | Yes | No | `asset_ids[]` | — |
| `delete_folder` | Library | Yes | No | `folder_ids[]` | — |

_(Catalogue lists 30 named rows here; the reference's full surface totals 36 tools — the
execution-plan agent must reconcile the exact remaining tool names against `AgentInstructions.swift`
in `../palmier-pro/` and record the delta. **Open item — see §13.)**_

**ID prefix shortening (`ToolExecutor+ShortId` equivalent):** Internal storage is full UUIDs. Outputs use the minimum unique prefix ≥ 8 chars. Inputs accept any prefix that uniquely identifies an ID; ambiguous prefix → tool error. Implement as a single `IdUniverse` snapshot per tool call covering clips, tracks, assets, folders.

**Agent undo stack:** Distinct from the user undo stack. Each mutating tool pushes an action name (e.g. "Move 3 clips"). The `undo` tool pops one entry and reverses it. Refuses if the most recent change came from the user (user manual edit interleaves).

**Tool error shape:** `{"isError": true, "content": [{"type": "text", "text": "<human-readable message>"}]}`

**Resources:** `palmier://models/video` (JSON array, current video models). `palmier://models/image` (JSON array, current image models).

**MCPB bundle for Claude Desktop:** Re-emit `palmier-pro.mcpb` (same `manifest_version: 0.4`, `name: palmier-pro`, `display_name: Palmier Pro Windows`, version, server block) bundled in app resources. Bundle a Node.js stdio→HTTP shim (`server/index.js`) running `mcp-remote` against `http://127.0.0.1:19789/mcp`. Settings → Help → MCP Instructions exposes "Install for Claude Desktop" extracting the `.mcpb` to `%APPDATA%\Claude\Extensions\` (or platform equivalent).

**Help → MCP Instructions tab:** Server URL (copy button) `http://127.0.0.1:19789/mcp`. Cursor install button (`cursor://anysphere.cursor-deeplink/mcp/install?name=palmier-pro&config=…`) + manual JSON. Claude Code CLI: `claude mcp add --transport http palmier-pro http://127.0.0.1:19789/mcp`. Codex CLI: `codex mcp add palmier-pro --url http://127.0.0.1:19789/mcp`. Claude Desktop install button (bundled `.mcpb`) + manual `~/.claude/mcp.json` JSON.

### 6.15 Settings + Account + Help

| Tab | Contents |
|---|---|
| Account | Signed in: tier, period end, cancellation status, credit summary (spent, budget, remaining), Top-Off Credits (Stripe via Convex), Sign Out. Not: Sign In + plan cards (Pro, Max). Loading state + error banner. |
| General | Notifications toggle (`palmier.notifications.enabled`); Privacy toggle (`palmier.telemetry.enabled`) with "Restart required" note. |
| Models | Downloaded Whisper / CLIP models + sizes + delete buttons; cache size. |
| Agent | Anthropic API key SecureField (masked, last 4 visible); Save / Delete; "Get key" → `https://console.anthropic.com/settings/keys`. MCP Server status + enable toggle + port display + "Setup instructions" → Help. |
| Storage | Cache sizes (thumbnails / waveforms / models / samples / transcripts) + cleanup buttons per category. |

**Help window tabs:** Shortcuts (matches §6.1 menu table); MCP (§6.14).
**Feedback window:** Multi-line description (≤10000 chars); email field if not signed in; include-screenshot toggle (if available); "May we contact you" toggle; Submit → Convex `/v1/feedback`.

### 6.16 Telemetry + logging
**Logging:** `tracing` crate, categorized targets `app, editor, export, preview, mcp, generation, project, transcription, search`. Subscriber writes `%LOCALAPPDATA%\PalmierProWin\Logs\palmier.log` (Windows) / `~/.local/state/palmier-pro/logs/palmier.log` (Linux), rotated daily, 7 days retained. Level Info default; Debug with `--debug`. Crash handler writes panic to `crashes/<timestamp>.log` + forwards to Sentry.

**Sentry:** DSN injected at build via `tauri.conf.json` → `PALMIER_SENTRY_DSN`. Environment `development` (debug) or `production` (release). `traces_sample_rate: 0.1`, app-hang detection if available. Release tag `palmier-pro-win@<version>+<git_sha>`. Send-default-PII false. Settings toggle `palmier.telemetry.enabled` (default true; restart required).

---

## 7. AGENT SYSTEM PROMPT (verbatim port)
Use the exact text from the reference `Sources/PalmierPro/Agent/Tools/AgentInstructions.swift` (≈145
lines) as the `instructions` field both in the MCP server and as the system prompt for the in-app
agent. Substitute platform-specific references where they appear. **Do not paraphrase** — clients
have been tuned against this prompt.

### 7.1 Key behavioral mandates (from reference, paraphrased for context)
- Timeline timing is in FRAMES. fps + resolution discovered via `get_timeline`.
- Always call `get_timeline`, `get_media`, and `list_models` early.
- Check `can_generate`; if false, instruct user to sign in.
- `inspect_media` before describing assets. Use `overview=true` for long media, then zoom.
- `search_media` to find moments across the library.
- Edits are free + undoable; don't ask permission for hand edits.
- Generations cost real money + are NOT undoable; propose params and wait for confirmation.
- Image-first iterative flow: get user to approve a still, then promote to `start_frame_media_ref`.
- Model selection heuristics: Nano Banana / GPT Image for stills; Seedance 2.0 Fast (720p) for video iteration.
- TTS vs music vs SFX: distinguish via prompt grammar + model selection.
- Communication: one or two sentences. Lead with outcome. No narration.

### 7.2 Voice (locked)
Quietly capable. Direct. Technical. Calm. Apple HIG-style terseness. Never chatty. Never marketing.

---

## 8. EXTERNAL INTEGRATIONS — interfaces

### 8.1 Convex backend (HTTP)
Base URL: from build-time `PALMIER_CONVEX_HTTP_URL`.

| Endpoint | Method | Auth | Purpose |
|---|---|---|---|
| `/v1/models` | GET | None / Clerk JWT | Model catalog |
| `/v1/samples` | GET | None | Sample project list |
| `/v1/samples/resolve?slug=<slug>` | GET | None | Sample project bundle |
| `/v1/agent/stream` | POST | Clerk JWT | Proxied Anthropic stream |
| `/v1/feedback` | POST | optional Clerk JWT | Feedback submission |
| `/v1/uploads/ticket` | POST | Clerk JWT | Pre-signed upload URL for reference media |
| `/v1/uploads/commit` | POST | Clerk JWT | Commit an upload |
| `generations:submit` (mutation) | RPC | Clerk JWT | Submit AI job |
| `generations:by_id(job_id)` (query) | RPC | Clerk JWT | Subscribe to job updates |

Use `convex-rs` if stable; otherwise hit Convex's HTTP API directly via `reqwest`. Subscriptions:
Convex WebSocket live queries via `tokio-tungstenite`.

### 8.2 Clerk auth
Embed Clerk's React SDK in the webview (sign-in/out, session). Token forwarded to Rust via Tauri
command after each sign-in event; Rust stores in memory + sends to Convex. Refresh: Clerk handles
silently; we refresh the cached JWT every 5 min.

### 8.3 Anthropic Messages API (direct, BYOK)
`POST https://api.anthropic.com/v1/messages`. Headers `x-api-key`, `anthropic-version: 2023-06-01`,
`accept: text/event-stream`. Body: model, system (with `cache_control: ephemeral`), messages, tools
(converted from our tool schemas). SSE events: `message_start`, `content_block_start`,
`content_block_delta` (`text_delta` + `input_json_delta`), `content_block_stop`, `message_delta`
(`stop_reason`), `error`.

### 8.4 Update server
Tauri 2 updater pulls `https://updates.palmier.io/win/latest.json` (**placeholder URL — confirm with
stakeholder**). Ed25519 signature embedded in `tauri.conf.json` public key field. Build artifacts:
`.msi` (Windows), `.AppImage` + `.deb` + `.rpm` (Linux). Each platform has its own updater manifest.

---

## 9. DESIGN TOKENS (CSS variable bridge)
Translated from reference `UI/AppTheme.swift`. Ship as `tokens.json` in the frontend; generate CSS at
build time.

### 9.1 Color

| Token | RGB / hex | Alpha |
|---|---|---|
| `--bg-base` | #0A0A0A | 1 |
| `--bg-surface` | #161616 | 1 |
| `--bg-raised` | #1E1E1E | 1 |
| `--bg-prominent` | #2C2C2C | 1 |
| `--bg-preview` | #000000 | 1 |
| `--border-primary` | white | 0.16 |
| `--border-subtle` | white | 0.12 |
| `--border-divider` | white | 0.44 |
| `--accent-primary` | #F5F0E4 | 1 |
| `--accent-timecode` | #F2994A | 1 |
| `--accent-spotlight` | #FF4545 | 1 |
| `--status-error` | #E54F4F | 1 |
| `--text-primary` | white | 1 |
| `--text-secondary` | white | 0.80 |
| `--text-tertiary` | white | 0.62 |
| `--text-muted` | white | 0.34 |
| `--track-video` | #0091C2 | 1 |
| `--track-audio` | #58A822 | 1 |
| `--track-image` | #B72DD2 | 1 |
| `--track-text` | #B72DD2 | 1 |
| `--track-lottie` | #E0A800 | 1 |

### 9.2 Scale tokens
- **Spacing (px):** xxs=2, xs=4, sm=6, smMd=8, md=10, mdLg=12, lg=14, lgXl=16, xl=20, xlXxl=24, xxl=28
- **Radius (px):** xs=3, xsSm=4, sm=6, md=10, mdLg=12, lg=14, xl=20
- **Border width (px):** hairline=0.5, thin=1, medium=1.5, thick=2
- **Font size (px):** micro=8, xxs=9, xs=10, sm=11, smMd=12, md=13, mdLg=14, lg=15, xl=18, title1=22, title2=28, display=36
- **Font weight:** light, regular, medium, semibold, bold
- **Tracking (em):** tight=-0.5, normal=0, wide=1.5
- **Icon size (px):** xxs=12, xs=14, sm=18, smMd=20, md=22, mdLg=24, lg=26, lgXl=28, xl=30
- **Opacity:** subtle=0.04, hint=0.06, faint=0.08, soft=0.10, muted=0.15, moderate=0.25, medium=0.35, strong=0.55, prominent=0.80, opaque=1.0
- **Shadow:** sm `0 0.5 1 rgba(0,0,0,0.3)`; md `0 2 4 rgba(0,0,0,0.3)`; lg `0 8 24 rgba(0,0,0,0.25)`
- **Animation (s):** hover=0.15, transition=0.20

### 9.3 Windows / Linux platform tokens

| Token | Value | Purpose |
|---|---|---|
| `--window-controls-padding` | 8px on Windows; 0 on Linux | Reserves space for system window controls |
| `--scrollbar-width` | 14px | Custom scrollbar (webview default too thin on Windows) |
| `--platform-font-stack` | `'Inter Variable', 'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif` (Windows); `'Inter Variable', system-ui, 'Cantarell', 'DejaVu Sans', sans-serif` (Linux) | Match platform-native body text |

---

## 10. PERFORMANCE TARGETS

| Metric | Target | Test condition |
|---|---|---|
| Cold start to project window | < 3 s | NVMe SSD, RTX 4060 / Radeon 7600 / Intel A380 |
| Open existing 30-clip 1080p project | < 1 s | Same |
| Timeline scrub at 4K 30 fps | ≥ 30 fps preview | 5 clips, 2 layers, no keyframe motion |
| Timeline scrub at 1080p 60 fps | ≥ 60 fps preview | Same |
| Add clip via drag-drop | < 100 ms perceived | Up to 4K input |
| Agent tool dispatch latency | < 50 ms p50 / 150 ms p99 | In-process |
| MCP tool round trip (loopback) | < 100 ms p50 / 300 ms p99 | `get_timeline` on 200-clip project |
| Export 1 minute 1080p H.264 | < real time on RTX 4060 (NVENC) | preset balanced |
| Memory ceiling, editor idle | < 800 MB RSS | 200-clip project loaded |
| Memory ceiling, editor + preview running | < 2.5 GB RSS | Same |
| Whisper transcription, 25-min recording | < 2 min on RTX 4060 (CUDA) | `small.en` model |

---

## 11. TESTING STRATEGY

### 11.1 Unit tests (Rust, `cargo test`)
Per-crate. Mandatory coverage:
- `palmier-model`: serialization round-trips for every shape, default fallback decode, computed property correctness, keyframe sampling at boundaries.
- `palmier-edit`: ripple shifts (single range, multi-range, sync-locked across tracks); overwrite engine (inside / overlap-start / overlap-end / cover-multi); split keyframe migration; snap stickiness.
- `palmier-engine`: composition graph build from a known timeline; per-frame transform/opacity/crop sample correctness; volume + fade envelope.
- `palmier-text`: caption phrase splitting (sentence/clause/midpoint cascade), duration distribution, minimum-duration cascade.
- `palmier-tools`: every tool with happy-path + 2 error cases. ID prefix expand/shorten with ambiguity tests.
- `palmier-export`: XMEML emission diff against committed golden XMLs; timecode formatting (NDF, drop-frame).

### 11.2 Integration tests (`tests/` per crate)
- Project bundle round-trip: import media → edit → save → reopen → assert identical state.
- Tool dispatcher: simulate a sequence of agent tool calls and assert final timeline state.
- MCP server: spawn server in fixture, issue JSON-RPC over HTTP, assert responses.
- Generation lifecycle: mock Convex client, simulate Generating → Downloading → None, assert UI events emitted.

### 11.3 End-to-end tests (Tauri WebDriver via `tauri-driver` + Playwright)
Cover §1.3 workflows:
- Long-form-to-shorts: open project → run transcription → agent "cut filler words" → assert ≥10 cuts → export MP4.
- B-roll-directed: import folder → agent "make a 30-second cut about X" → assert clips arranged.
- Generative augment: open project → agent generate a 5-sec transition → assert placeholder appears → simulate completion → assert it lands on timeline.
- Hand editing: drag-drop import → drag clip → trim → split → undo → redo → save.

### 11.4 Performance tests (`criterion` benchmarks)
Composition graph build for 50, 200, 1000-clip timelines. Per-frame sample evaluation for animated clips. Tool dispatch latency (in-process). Search index query (1k, 10k, 100k frames).

### 11.5 Golden assets
Maintain `tests/fixtures/`: `golden_project_minimal.palmier` (single video clip, 1 track); `golden_project_keyframes.palmier` (keyframed transform + opacity + crop); `golden_project_text.palmier` (text clips with full style); `golden_xmeml_*.xml` (expected XMEML for fixture exports). Diff against goldens in CI. Update only via explicit `--update-golden` flag (gated by review).

### 11.6 MCP compatibility test suite
Run the reference's MCP test prompts against our server via Claude Code with MCP transport pointed at our server. Prompts: "what's on my timeline?", "cut the filler words", "add a title at the start saying 'Hello'", "generate B-roll for a beach scene". Verify no protocol errors, tool descriptions resolved, tool calls succeed.

---

## 12. PROJECT LAYOUT (Cargo + pnpm workspace)
```
palmier-pro-win/
├── Cargo.toml                     # workspace
├── crates/
│   ├── palmier-model/   palmier-project/   palmier-media/   palmier-engine/
│   ├── palmier-text/    palmier-edit/      palmier-history/ palmier-export/
│   ├── palmier-transcribe/  palmier-search/  palmier-gen/   palmier-agent/
│   ├── palmier-mcp/     palmier-tools/     palmier-auth/    palmier-update/
│   ├── palmier-telemetry/
│   └── palmier-tauri/             # tauri binary, wires everything together
├── src-ui/                        # React + TS frontend
│   ├── package.json  vite.config.ts  tailwind.config.ts
│   └── src/
│       ├── design/                # tokens.json + generated CSS
│       ├── app/                   # routing / shell
│       ├── editor/                # timeline, preview, inspector, panels
│       ├── media-panel/  agent-panel/  settings/  home/
│       └── ipc/                   # Tauri command wrappers
├── tauri.conf.json
├── tests/
│   ├── e2e/   fixtures/
├── docs/
│   ├── FOUNDATION.md             # this document
│   ├── PRD.md                    # produced by PRD agent
│   ├── EXECUTION_PLAN.md         # produced by execution agent
│   └── ADR/                      # architecture decision records
├── scripts/
│   ├── bundle.ps1                # Windows packaging
│   └── bundle.sh                 # Linux packaging
└── .github/workflows/
    ├── ci.yml   release.yml
```

> **Repo-layout reconciliation (loop note):** This repo is *also* the loop-engineering knowledge base
> (`signals/ docs/ domains/`, `_bmad-output/`). The app's `docs/` and the KB's `docs/` are the same
> folder, so `docs/FOUNDATION.md` satisfies both. BMAD drafts planning artifacts under
> `_bmad-output/planning-artifacts/`; the finalized **PRD.md** and **EXECUTION_PLAN.md** are promoted
> to `docs/` per this layout. See `docs/build-orchestration.md`.

---

## 13. OPEN QUESTIONS (defer to PRD)
1. Update channel strategy — single channel vs stable/beta/nightly?
2. Telemetry opt-in vs opt-out — reference is opt-out; Windows/Linux expectations may differ.
3. CLIP model selection — bundled vs downloadable; size/quality tradeoff (ViT-B/32 vs ViT-L/14).
4. Whisper model bundling — `small.en` (~500 MB) vs `base.en` (~150 MB) shipped default.
5. Lottie support priority — required for v1 or v2? Reference ships it; we may defer.
6. Tutorial content — port from reference word-for-word or write fresh?
7. Social-platform sidecar JSON schema — define jointly with the TypeScript platform team.
8. Linux distribution — Flatpak in addition to AppImage / .deb / .rpm?
9. Convex backend access for the Windows port — does the existing deployment accept our client, or stand up a separate Windows-port backend?
10. Naming + branding — "Palmier Pro Windows" risks confusion with the open-source Mac project; consider a distinct fork name.
11. License compatibility — reference is GPLv3; FFmpeg LGPL/GPL builds; Whisper MIT; Tauri MIT/Apache. Cross-check ahead of distribution.
12. **Exact MCP tool surface** — this doc enumerates 30 tools but states the reference exposes 36. Diff against `../palmier-pro/Sources/PalmierPro/Agent/Tools/` and record the missing 6.

---

## 14. RECOMMENDED NEXT STEPS FOR DOWNSTREAM AGENTS

**PRD agent** consumes this doc and produces: a prioritized epic list (App shell → Project I/O →
Timeline editor → Media import → Preview composition → Export → MCP server → Agent → Generation →
Transcription → Search → Polish); per-epic acceptance criteria; risk register; decisions on every
Open Question in §13.

**Execution-plan agent** produces: a milestone schedule (suggest M1 hand-edit MVP, M2 MCP server +
agent, M3 generation + transcription, M4 visual search + captions, M5 export polish + release); a
per-milestone task DAG; cross-crate dependency order; resource allocation (agents per crate).

**Testing agent** consumes §11 and produces: per-crate test plan with concrete test cases;
golden-asset generation procedure; CI matrix (Windows + Linux, debug + release, GPU + CPU fallback).

---
_End of foundation document._

## Timeline
2026-06-20 | kickoff — filed verbatim as the source of truth for PRD + execution-plan phases. macOS reference verified at `../palmier-pro/`.
