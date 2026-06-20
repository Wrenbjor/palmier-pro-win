---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 3 — Timeline Editor & Editing Engines"
created: 2026-06-20
---

# Epic 3 — Timeline Editor & Editing Engines

## Epic goal

Deliver the manual timeline editor: the pure-function editing engines (Ripple, Overwrite, Snap,
Split, Trim), the supporting model types they consume, the user/agent undo stacks, the orchestration
layer that wires sync-lock fan-out + link-group expansion + atomic undo around those engines, and the
React timeline canvas (geometry, ruler, playhead, per-type clip visuals, rubber bands, fades, range
selection) plus its input controller (tool modes, selection, drag/trim/split, snap). This is the
Premiere-class hand-edit core of UJ-4 and the cut-application substrate for UJ-1.

**Reference (parity authority):** there is **no Slip and no Slide engine** in the macOS source
(reconciliation ruling #11); Trim and Split are **not** separate engines — they live in the
orchestration layer over OverwriteEngine + per-clip frame math (`docs/reference/edit-engines.md`
lines 14-21). Build only Ripple, Overwrite, Snap engines + the orchestration-layer Trim/Split.

### PRD acceptance this epic must satisfy (§4.3 FR-9..FR-13, §10 Epic 3)

- **FR-9** Pointer (V) / Razor (C) tools; single/Shift/Ctrl click + marquee selection persisting
  across re-renders by clip ID.
- **FR-10** Move (cross-track for compatible types — **all visual types interchangeable**, ruling
  #12), trim-left/right, split, with SnapEngine snapping to every clip edge + playhead; base
  threshold **8 px**, playhead **×1.5**, trim handle **4 px**, **sticky multiplier 1.5×** (ruling
  #10); **Slip and Slide not implemented** (ruling #11); add-clip-via-drag **< 100 ms perceived**
  (SM-3).
- **FR-11** RippleEngine: shifts for deletes (single + multi-range, merged), pushes for inserts,
  sync-locked propagation; linked clips ride along; deleting clips closes gaps and shifts only clips
  whose `start >= removed-range end`; unit-tested against the reference algorithm.
- **FR-12** OverwriteEngine: insertion at `(track, start, duration)` returns clips to delete + trim
  for all cases (inside / overlap-start / overlap-end / cover-multi); used by drag-drop, paste, agent
  `add_clips`.
- **FR-13** `split_clip` migrates keyframes with recomputed offsets; linked clips (shared
  `link_group_id`) move together; timing props propagate, volume/opacity/transform/text do not.
- **§10 acceptance:** all of the above plus **user undo/redo** (Ctrl+Z / Ctrl+Shift+Z) on the
  **user** stack, separate from the agent stack; **all source↔timeline conversions use `f64::round`
  ties-away-from-zero, never `round_ties_even`** (carry-forward note).

### Crates touched
`palmier-edit` (pure engines), `palmier-history` (undo stacks), `palmier-model` (model types the
engines consume — extends Epic 2's `palmier-model`), and `src-ui/editor` (timeline canvas + input).
The **orchestration layer** (link-group expansion, sync-lock fan-out, undo grouping, `clearRegion`
apply loop) lives above the pure crates — per `docs/reference/edit-engines.md` lines 216-217 it is a
command layer wrapping `palmier-edit` + `palmier-history`; this epic places it in `palmier-edit`'s
`orchestration` mod (it depends on both pure-edit and history but exposes no UI/IO).

### Milestone
**M1 — Hand-Edit MVP** (PRD §12; Epics 1-6). Epic 3 is a core M1 deliverable and contributes the
hand-edit half of the **§11.3 hand-editing e2e exit gate** (drag/trim/split/undo/redo/save) and
SM-3 (edit latency).

### Spike / gating note
**This epic is NOT spike-gated.** Spike S-1 (wgpu→WebView presentation) gates **Epic 5** only — do
not couple Epic 3 to a presentation mechanism. The timeline canvas here is the **2D timeline canvas**
(clip rects, ruler, playhead, waveforms, rubber bands), rendered with Canvas 2D / WebGL in the
webview as a normal DOM/canvas surface — it does **not** consume the wgpu preview texture and has no
dependency on S-1. (Only the **preview viewport** in Epic 5 needs the composited wgpu frame.)

### Cross-epic dependencies (must land first)
- **Epic 2 (Project I/O & Data Model)** — `palmier-model`'s `Clip`, `Track`, `Timeline`,
  `Transform` (center-based), `KeyframeTrack`, `ClipType::is_compatible`, `f64::round` rounding
  convention, and serde must exist before the engines can operate on them. Stories E3-S1+ extend
  `palmier-model` with edit-specific value types (`FrameRange`, `ClipShift`) but the core model is
  Epic 2's.

---

## Stories

### E3-S1 — Edit value types + ClipType compatibility in palmier-model

**Intent:** As the editing engines, I need the shared value types and the cross-track compatibility
rule in `palmier-model` so every engine and the orchestration layer speak the same vocabulary.

**Acceptance criteria:**
- Add to `palmier-model` (alongside Epic 2's `Clip`/`Track`/`Timeline`):
  - `FrameRange { start: i32, end: i32 }` — **half-open `[start, end)`**, `length = end - start`
    (`docs/reference/edit-engines.md` line 26). `contains(frame) = frame >= start && frame < end`.
  - `ClipShift { clip_id: String, new_start_frame: i32 }` (IDs are **UUID strings**, not `Uuid` —
    ruling/`timeline-model.md` line 58).
  - `GapSelection { track_index: usize, range: FrameRange }`.
  - `TimelineRangeSelection { start_frame: i32, end_frame: i32 }` with `normalized` (swap if
    reversed), `is_valid` (`end > start`), `contains` half-open `[start, end)`
    (`timeline-model.md` lines 195-199).
- `ClipType::is_compatible(other)` = `self == other || (self.is_visual && other.is_visual)` where
  `is_visual = video | image | text | lottie` (`edit-engines.md` line 46; ruling #12 — **all visual
  types interchangeable**, audio its own zone). Unit test the full 5×5 compatibility matrix.
- Frame derivations available on `Clip`: `end_frame = start_frame + duration_frames`;
  `source_frames_consumed = f64::round(duration_frames as f64 * speed) as i32` (ties-away — never
  `round_ties_even`); `source_duration_frames = source_frames_consumed + trim_start_frame +
  trim_end_frame` (`timeline-model.md` lines 54-56).
- **Unit tests:** compatibility matrix; `FrameRange`/`TimelineRangeSelection` half-open `contains`
  boundary (frame == start true, frame == end false); `source_frames_consumed` rounding parity on
  `speed ∈ {0.25, 0.5, 1.0, 1.7, 4.0}` including x.5 ties.

**Implementation context:** crate `palmier-model`. Reference: `Sources/PalmierPro/Models/ClipType.swift`,
`Models/Timeline.swift:75` (Clip derived props), `Timeline/TimelineRangeSelection.swift`,
`Editor/RippleEngine.swift` (FrameRange/ClipShift/GapSelection type defs). Docs:
`docs/reference/edit-engines.md` §"Key types" + `docs/reference/timeline-model.md` §"Data model".

**Dependencies:** Epic 2 (palmier-model core types + `f64::round` convention).

**Parallel-safe?** No — it is the shared foundation for E3-S2..E3-S6; they import these types. Land
first, then the engine stories parallelize.

---

### E3-S2 — RippleEngine (pure)

**Intent:** As ripple-delete/insert operations, I need pure shift/push computation so gap-closing and
insert-push are deterministic and unit-tested independent of UI.

**Acceptance criteria:** Implement `palmier-edit::ripple` matching FOUNDATION §6.4 signatures exactly:
- `merge_ranges(ranges) -> Vec<FrameRange>`: sort by `start`; fold — if `range.start <= last.end`
  extend `last.end = max(last.end, range.end)` else push. **Touching ranges merge** (`<=`)
  (`edit-engines.md` lines 55-56).
- `compute_ripple_shifts_for_ranges(clips, removed_ranges)`: `merged = merge_ranges(removed_ranges)`;
  iterate clips sorted by `start_frame`; `shift = Σ r.length for r in merged where r.end <=
  clip.start_frame`; emit `ClipShift(clip.id, clip.start_frame - shift)` **only when `shift > 0`**
  (a clip overlapping a gap is assumed already removed/cleared) (lines 57-60).
- `compute_ripple_shifts(clips, removed_ids)`: derive removed ranges from removed clips'
  `[start_frame, end_frame)`, then call `…_for_ranges` on the **remaining** clips (lines 61-62).
- `compute_ripple_push(clips, insert_frame, push_amount, exclude_ids)`: every clip with
  `start_frame >= insert_frame` and `id ∉ exclude_ids` → `ClipShift(id, start_frame + push_amount)`
  (lines 63-64).
- `validate_shifts(track, shifts) -> Result<(), RefuseReason>`: apply shift map to that track's
  clips; refuse if any `start < 0` ("would move past timeline start") or any two intervals overlap
  after sort ("no room") (lines 80-82).
- **Unit tests (per FOUNDATION §11.1 / §6.4):** single removed range; multi-range with overlapping +
  **touching** ranges (assert touching merges); a clip starting exactly at a gap end (`r.end ==
  clip.start_frame` → shifts); a clip overlapping a gap (no shift emitted); push with excluded ids;
  `validate_shifts` collision + negative-start refusals.

**Implementation context:** crate `palmier-edit`, mod `ripple`. Reference:
`Sources/PalmierPro/Editor/RippleEngine.swift` (`computeRippleShifts`,
`computeRippleShiftsForRanges`, `computeRipplePush`, `mergeRanges`),
`Editor/ViewModel/EditorViewModel+Ripple.swift` (`validateShifts`). Docs: `edit-engines.md`
§"RippleEngine (pure)".

**Dependencies:** E3-S1.

**Parallel-safe?** Yes — own file `palmier-edit/src/ripple.rs`, no overlap with S3/S4/S5/S6.

---

### E3-S3 — OverwriteEngine (pure)

**Intent:** As drag-drop / paste / agent `add_clips`, I need `compute_overwrite` to return the exact
delete/trim/split actions so an insertion overwrites in place correctly for every overlap case.

**Acceptance criteria:** Implement `palmier-edit::overwrite::compute_overwrite(clips, region_start,
region_end) -> Vec<OverwriteAction>` where `OverwriteAction = Remove | TrimEnd | TrimStart | Split`
(`edit-engines.md` lines 27-28, 85-97). Guard `region_end > region_start`. For each clip with
`cs = start_frame, ce = end_frame`:
1. `ce <= region_start || cs >= region_end` → no overlap, skip.
2. `cs >= region_start && ce <= region_end` → `Remove(clip_id)`.
3. `cs < region_start && ce > region_end` (region strictly inside) → `Split`: `left_duration =
   region_start - cs`; `right_start_frame = region_end`; `right_trim_start = clip.trim_start_frame +
   f64::round((region_end - cs) as f64 * speed)`; `right_duration = ce - region_end`; new UUID for
   right.
4. `cs < region_start` (overlaps left, ce inside) → `TrimEnd(new_duration = region_start - cs)`.
5. else (overlaps right, cs inside) → `TrimStart`: `new_start_frame = region_end`; `new_trim_start =
   clip.trim_start_frame + f64::round((region_end - cs) as f64 * speed)`; `new_duration = ce -
   region_end`.
- **The `Split` action's right-fragment fields are advisory** — the orchestration layer (E3-S6)
  re-derives them via `split_clip`; this story emits them but documents them as advisory
  (`edit-engines.md` lines 96-97, 228-229).
- All `*speed` conversions use `f64::round` ties-away.
- **Unit tests (FOUNDATION §11.1):** the four named cases — **inside / overlap-start / overlap-end /
  cover-multi** (multiple clips, with partial bookends getting trim and fully-covered getting
  remove); `region_end <= region_start` guard returns empty; rounding parity on the split/trim
  source-offset at `speed ∈ {0.5, 1.7, 4.0}`.

**Implementation context:** crate `palmier-edit`, mod `overwrite`. Reference:
`Sources/PalmierPro/Editor/OverwriteEngine.swift` (`computeOverwrite`). Docs: `edit-engines.md`
§"OverwriteEngine (pure)".

**Dependencies:** E3-S1.

**Parallel-safe?** Yes — own file `palmier-edit/src/overwrite.rs`.

---

### E3-S4 — Split & Trim pure math (clip-relative, keyframe migration, source↔timeline round-trip)

**Intent:** As split and trim commits, I need pure clip-relative frame math with correct keyframe
migration and a source↔timeline round-trip so trims/splits are frame-exact and reversible.

**Acceptance criteria:** Implement `palmier-edit::split`:
- `split_clip(clip, at_frame) -> Option<(Clip /*left*/, Clip /*right*/)>` — guard `start_frame <
  at_frame < end_frame`. `split_offset = at_frame - start_frame`;
  `left_source = f64::round(split_offset as f64 * speed)`;
  `right_source = f64::round((duration_frames - split_offset) as f64 * speed)`
  (`edit-engines.md` lines 99-110):
  - **left** = copy: `duration_frames = split_offset`, `trim_end_frame += right_source`,
    `fade_out_frames = 0`, clamp fades.
  - **right** = copy + **new UUID**: `start_frame = at_frame`, `duration_frames = orig.duration_frames
    - split_offset`, `trim_start_frame += left_source`, `fade_in_frames = 0`, clamp fades.
  - **Volume-keyframe migration** (frames are **clip-relative offsets**, FOUNDATION §5.5): boundary
    value = `track.sample(at: split_offset)`. Left keeps kfs `frame <= split_offset`, appends a
    boundary kf at `split_offset` if absent. Right keeps kfs `frame >= split_offset`, **re-bases each
    `frame -= split_offset`**, inserts a boundary kf at frame 0 if absent. (Same migration for the
    other keyframe tracks: keep / re-base / boundary-insert.)
- `trim_values(clip, edge, delta_frames) -> (trim_start, trim_end)` and the round-trip from
  `trimClipInternal` (`edit-engines.md` lines 112-128): `source_delta = f64::round(delta_frames as
  f64 * speed)`. Left edge: new `trim_start_frame = (unbounded ? : max(0, _)) old + source_delta`.
  Right edge: new `trim_end_frame = (unbounded ? : max(0, _)) old - source_delta`. Back-convert:
  `delta_start_timeline = f64::round((new_trim_start - old_trim_start) as f64 / speed)`,
  `delta_end_timeline = f64::round((new_trim_end - old_trim_end) as f64 / speed)`, `new_duration =
  old_duration - delta_start_timeline - delta_end_timeline`, `new_start = old_start +
  delta_start_timeline`. **Trim does NOT ripple neighbors** (overwrite-style, in place).
- Trim drag clamps (consumed by E3-S5/E3-S7): trim-left `max_delta = original_duration - 1`,
  `min_delta = has_no_source_media ? -original_start_frame : -original_trim_start`; trim-right
  `min_delta = -(original_duration - 1)`, `max_delta = has_no_source_media ? unbounded :
  original_trim_end`. `has_no_source_media = image | text` (`edit-engines.md` lines 113-118, 236-238).
- **Golden / unit tests:** **keyframe boundary sampling** — split at a point with kfs both sides,
  exactly on a kf, and with no kf at the boundary (assert boundary kf inserted, right re-based, left
  value continuous); **source↔timeline round-trip** golden over `speed ∈ {0.25, 0.5, 1.0, 1.7, 4.0}`
  asserting trim→back-convert is frame-stable (no ±1 drift — `edit-engines.md` lines 221-222);
  `has_no_source_media` clamp path distinct from video/audio.

**Implementation context:** crate `palmier-edit`, mod `split`; reads `KeyframeTrack::sample` from
`palmier-model` (Epic 2). Reference: `Editor/ViewModel/EditorViewModel+ClipMutations.swift`
(`splitClip`/`splitSingleClip`), `EditorViewModel+Linking.swift` (`commitTrim`, `trimValues`,
`trimClipInternal`), `Models/Keyframe.swift` (`sample`, clamp helpers). Docs: `edit-engines.md`
§"Split" + §"Trim".

**Dependencies:** E3-S1; needs Epic 2's `KeyframeTrack::sample` + fade-clamp helpers.

**Parallel-safe?** Yes — own file `palmier-edit/src/split.rs`.

---

### E3-S5 — SnapEngine (pure, side-effect-free) + geometry

**Intent:** As any drag/trim, I need snap-target collection and a sticky snap finder (no haptics, no
side effects) plus the pure layout geometry so snapping and hit-testing are deterministic.

**Acceptance criteria:** Implement `palmier-edit::snap`:
- `collect_targets(tracks, playhead_frame, exclude_clip_ids, include_playhead) -> Vec<SnapTarget>`:
  every non-excluded clip's start AND end as `SnapTarget { frame, kind: ClipEdge }`; plus playhead as
  `kind: Playhead` when `include_playhead` (`edit-engines.md` lines 131-132).
- `find_snap(position, probe_offsets, targets, &mut SnapState, base_threshold_px, pixels_per_frame)
  -> Option<SnapResult>` — **no side effects** (strip the reference's `NSHapticFeedbackManager`
  call; return the result as a value) (`edit-engines.md` lines 32, 184-185):
  - `base_frame_threshold = base_threshold_px / pixels_per_frame`.
  - **Sticky:** if `state.currently_snapped_to` set and sticky probe within `base_frame_threshold *
    1.5` of it AND target still exists → return the held snap; else clear state. (**Sticky multiplier
    = 1.5**, ruling #10 / `edit-engines.md` lines 135-136 — **NOT** FOUNDATION's 2.5×.)
  - **Find best:** per probe offset `probe_pos = position + offset`; per target, threshold =
    `base_frame_threshold * 1.5` for `Playhead` else `base_frame_threshold`; if `|probe_pos -
    target.frame| <= threshold` and smallest distance so far, record; return closest as `SnapResult {
    frame: target.frame, probe_offset, x: target.frame * pixels_per_frame }`; set sticky state
    (`edit-engines.md` lines 137-141).
  - Constants from `Constants.swift`: base threshold **8 px**, playhead **×1.5**, sticky **1.5×**,
    trim handle **4 px**, `Defaults.pixels_per_frame = 4.0` (`edit-engines.md` lines 47-50).
- `palmier-edit::geometry`: pure `frame_at(x) = max(0, floor((x - header_width) / ppf))`,
  `x_for_frame`, `track_at(y)`, `drop_target_at(y) -> TrackDropTarget { ExistingTrack(i) |
  NewTrackAt(i) }`, `clip_rect` (`timeline-model.md` lines 113-119; `edit-engines.md` lines 36,
  210-211). Constants: `ruler_height = 24`, `drop_zone_height = 60`, `track_height = 50`,
  `insert_threshold = 10` (`edit-engines.md` lines 49-50).
- **Unit tests (FOUNDATION §11.1 "snap stickiness"):** snap within threshold; **just outside**
  threshold (no snap); sticky stays until probe moves > `1.5 × threshold` then releases (assert the
  **1.5** multiplier, not 2.5); playhead's wider 1.5× catch radius wins over a clip edge at equal
  distance; two probe offsets `[start, start+duration]` each able to snap; `drop_target_at` boundary
  hit regions (top drop zone, between-track within `insert_threshold`, past last).

**Implementation context:** crate `palmier-edit`, mods `snap` + `geometry`. Reference:
`Timeline/SnapEngine.swift` (`collectTargets`, `findSnap` — **drop the haptic side effect**),
`Timeline/TimelineGeometry.swift`. Docs: `edit-engines.md` §"SnapEngine" + §"Selection & tool modes"
(probe offsets), `timeline-model.md` §"Geometry".

**Dependencies:** E3-S1.

**Parallel-safe?** Yes — own files `palmier-edit/src/snap.rs` + `geometry.rs`.

---

### E3-S6 — Edit orchestration: ripple/overwrite/split fan-out + atomic apply

**Intent:** As a complete edit command (ripple-delete, ripple-insert, gap-delete, clear-region,
move, split), I need the orchestration layer that fans pure-engine results across sync-locked +
linked tracks and applies them atomically, so multi-track edits are correct and all-or-nothing.

**Acceptance criteria:** Implement `palmier-edit::orchestration` (command layer over the pure mods +
`palmier-history`):
- **Ripple delete selected** (`ripple_delete_selected_clips`): build `global_removed_ranges` from
  selected clips' spans. Per track: if track has own removed clips → `compute_ripple_shifts`; else if
  `track.sync_locked` → `compute_ripple_shifts_for_ranges(global_removed_ranges)` then
  `validate_shifts`; **on failure refuse the WHOLE edit** (no mutation; surface a refuse reason for a
  toast — no `NSSound.beep`). Apply: remove clips, then apply all shifts (`edit-engines.md` lines
  66-70).
- **Ripple delete ranges on a track** (`ripple_delete_ranges_on_track`, drives MCP
  `ripple_delete_ranges`): `merged = merge_ranges(ranges.filter(length > 0))`; `clear_track_ids` =
  anchor track + tracks holding **linked partners** of any clip a range overlaps (`r.start <
  c.end_frame && r.end > c.start_frame`). Pre-validate every **non-cleared** sync-locked track; refuse
  on collision. Inside one undo swap: for each cleared track call `clear_region` per merged range
  (prune = false); for cleared OR sync-locked tracks apply `compute_ripple_shifts_for_ranges(merged)`;
  re-sort. Return a report (`removed_frames, cleared_tracks, shifted_clips, fragments,
  removed_clip_ids`) (`edit-engines.md` lines 71-77).
- **Ripple insert** (`ripple_insert_clips`): `total_push = Σ clip_duration_frames`; for target +
  every sync-locked track, `compute_ripple_push(insert_frame = at_frame, total_push)`; create clips at
  `at_frame` (lines 78-79).
- **Gap ripple** (`ripple_delete_selected_gap`): gap = `[prev_clip_end, next_clip_start)`; shift gap
  track + sync-locked followers left by gap length; followers validated, gap track not (lines 84-85).
- **clear_region apply** of `OverwriteAction`: `TrimEnd` recomputes `trim_end_frame +=
  f64::round((old_dur - new_dur) as f64 * speed)`; the `Split` action **re-splits via `split_clip(at
  = start)`** then removes/re-splits the right fragment — **VM re-derives the right fragment; the
  engine's advisory fields are ignored** (lines 95-97, 228-229).
- **Linked split**: `split_clip` every member of the link group at the same `at_frame`; **regroup the
  right halves under a fresh `link_group_id`** (lines 110, 237). Razor-tool split and Ctrl+K/playhead
  split both route here.
- **move_clips**: pull movers off source tracks, `clear_region` each destination (overwrite), drop
  each at its exact `to_frame`; **pinned companions** (linked partners of lead, OR clips whose type is
  incompatible with lead's destination type) keep their own row; others shift by the track delta
  (`edit-engines.md` lines 164-170).
- **Atomicity:** nested mutations under one `with_timeline_swap` coalesce into **one** user-visible
  undo step (skip registration when nested — `edit-engines.md` lines 248-249). Refuse-the-whole-edit
  runs validation **before any mutation** and aborts atomically (lines 225-227).
- **Unit/integration tests:** sync-locked multi-track ripple (FOUNDATION §11.1 "ripple shifts …
  sync-locked across tracks"); refuse-the-whole-edit leaves the timeline byte-unchanged; clear-region
  split re-derivation matches `split_clip` output; linked split regroups right halves under a new id
  (not cross-linked); pinned companions hold their row on a cross-track move; one undo entry per
  composite edit.

**Implementation context:** crate `palmier-edit`, mod `orchestration`; depends on `ripple`,
`overwrite`, `split`, `palmier-model`, and `palmier-history` (E3-S8). Reference:
`Editor/ViewModel/EditorViewModel+Ripple.swift`, `EditorViewModel+ClipMutations.swift`
(`clearRegion`, `moveClips`, `splitClip`, `withTimelineSwap`), `EditorViewModel+Linking.swift`
(link-group expansion). Docs: `edit-engines.md` §"Ripple orchestration" + §"Mapping to FOUNDATION
crates" (orchestration note, lines 216-217).

**Dependencies:** E3-S2, E3-S3, E3-S4, E3-S8 (palmier-history for the undo swap). Soft-coupled to
E3-S1.

**Parallel-safe?** No — it imports and composes S2/S3/S4 and S8. Schedule after those land. (It does
not touch their files, but its behavior is defined by them, so sequence it.)

---

### E3-S7 — Drag-state machine + clamping (pure transitions)

**Intent:** As the input controller, I need the `DragState` machine and its clamping as pure
transitions driven by pointer events, so trim/move/duplicate/marquee/range behavior is testable
without the canvas.

**Acceptance criteria:** Implement `palmier-edit::drag`:
- `DragState` enum = `Idle | ScrubPlayhead | MoveClip | TrimLeft | TrimRight | AudioVolumeKf |
  FadeKnee | Marquee | TimelineRange`, each with its payload struct (`edit-engines.md` lines 33-34).
- Pure transitions for `mouse_down / dragged / up`:
  - **Hit sub-mode** by `local_x = point.x - clip_rect.min_x`: fade-knee hit → `FadeKnee`; audio
    volume-kf hit → `AudioVolumeKf`; `local_x <= 4` → `TrimLeft`; `local_x >= width - 4` →
    `TrimRight`; else `MoveClip`. `Alt` on a body grab → `is_duplicate = true` (lines 155-158).
  - **Move-drag probes:** for every dragged clip push **two** probe offsets — `start_offset` and
    `start_offset + duration_frames` — relative to the lead's original frame, so any edge of any
    selected clip can snap; `delta_frames = (snap.frame - snap.probe_offset) - lead.original_frame`
    (`edit-engines.md` lines 146-147; FOUNDATION §6.3 probe `[0, duration_frames]`).
  - **Trim clamps** from E3-S4 (trim-left/right min/max with `has_no_source_media`).
  - **Move drop clamp:** `frame_delta = max(-min_orig_frame, delta_frames)` (no clip before frame 0);
    `clamped_track_delta` steps toward 0 until all movers fit a type-compatible track;
    `pinned_companions` = linked partners of lead + type-incompatible co-selected clips
    (`edit-engines.md` lines 164-170, 212).
  - **Marquee:** rect = min/max of origin↔point; select = `base_selection ∪ {clips whose clip_rect
    intersects}`; expand to link group unless Alt; cancels gap selection once rect exceeds
    `drag_threshold = 3` (lines 161-163; constant line 49).
- Modifier mapping: macOS Cmd → **Ctrl**, Option → **Alt** on Win/Linux (`edit-engines.md` line 190).
- **Unit tests:** sub-mode selection at each `local_x` boundary (4 px handle); duplicate flag on
  Alt-body-grab; two-probe move snap (end of a non-lead selected clip snaps); `frame_delta` floors at
  -min-orig-frame; `clamped_track_delta` steps to a compatible track; pinned companions identified;
  marquee threshold cancels gap.

**Implementation context:** crate `palmier-edit`, mod `drag`; consumes `snap`/`geometry` (E3-S5) and
`split` clamps (E3-S4). Reference: `Timeline/DragState.swift`,
`Timeline/TimelineInputController.swift` (mouseDown/Dragged/Up hit-testing + clamping). Docs:
`edit-engines.md` §"Selection & tool modes" + §"drag mod".

**Dependencies:** E3-S1, E3-S4 (trim clamps), E3-S5 (snap/geometry).

**Parallel-safe?** Partly — own file `palmier-edit/src/drag.rs`, but its clamp logic is defined by
S4/S5; land after those (no file collision, sequence for correctness).

---

### E3-S8 — palmier-history: user + agent undo stacks

**Intent:** As every undoable edit, I need separate user and agent undo/redo stacks with the
reference's two registration patterns so undo is atomic and the agent stack never tangles with the
user stack.

**Acceptance criteria:** Implement `palmier-history`:
- Two **separate** stacks: **User Undo Stack** and **Agent Undo Stack** (FOUNDATION §1.5/§6.14;
  glossary). Ctrl+Z / Ctrl+Shift+Z drive the **user** stack only.
- Two registration patterns ported from the reference (`edit-engines.md` lines 194-196):
  - `with_timeline_swap` — whole-`Timeline` before/after snapshot, **atomic**; nested calls do **not**
    re-register (coalesce into one user-visible step).
  - `register_clip_property_swap` / `register_clip_state_swap` — bidirectional per-clip swaps.
- Each undo group carries an **action name** (`undo_action_name`); the agent `undo` tool (Epic 7)
  will refuse unless the current `undo_action_name` matches the pushed name — **expose
  `current_undo_action_name()`** so Epic 7/SM-4 can enforce the refuse-after-user-edit rule
  (carry-forward note; ruling/reconciliation "Agent undo").
- **Unit tests:** push→undo→redo restores exact state for both swap patterns; nested
  `with_timeline_swap` produces **one** undo entry; user and agent stacks are independent (an agent
  push does not appear on the user stack and vice versa); `current_undo_action_name()` reflects the
  last pushed group.

**Implementation context:** crate `palmier-history`. Reference: `UndoManager` usage across
`EditorViewModel+ClipMutations.swift` / `+Ripple.swift` / `+Linking.swift` (`withTimelineSwap`,
`registerClipPropertySwap`, `registerClipStateSwap`, `setActionName`). Docs: `edit-engines.md`
§"macOS/Apple APIs to replace" (UndoManager → palmier-history) + FOUNDATION §6.14 (agent undo).

**Dependencies:** Epic 2 (palmier-model `Timeline`/`Clip` for snapshots).

**Parallel-safe?** Yes — separate crate `palmier-history`, no file overlap with `palmier-edit`. Can
run in parallel with E3-S2..E3-S5; E3-S6 depends on it.

---

### E3-S9 — Timeline canvas rendering (geometry, ruler, playhead, clip visuals, rubber bands, fades)

**Intent:** As the editor UI, I need the 2D timeline canvas drawing the model so the user sees tracks,
clips, ruler, playhead, waveforms, rubber bands, and fades — pixel-faithful to the reference math.

**Acceptance criteria:** Implement the timeline canvas in `src-ui/editor` (Canvas 2D / WebGL DOM
surface — **not** the wgpu preview; **no S-1 dependency**), consuming `palmier-edit::geometry` +
`palmier-model` sampling:
- **Draw order** (`timeline-model.md` line 39): backgrounds → range fill → clips → gaps → generating
  overlays → drag ghosts → marquee → insertion line → razor preview → ruler.
- **Geometry:** track stack Y starts at `ruler_height(24) + drop_zone_height(60)`; `clip_rect =
  (x = header_width + start_frame*ppf, y = track_y + 2, w = duration_frames*ppf, h = track_height -
  4)` (`timeline-model.md` lines 114-116). Video/image/text/lottie tracks above, audio below.
- **Ruler:** major tick target ~80 px from candidates `[1,2,5,10,15,30,60,120,300,600,1200,1800,
  3600]*fps`; minor subdivisions `[10,5,4,2]` (each minor ≥ 12 px); monospaced-digit timecode labels
  (`timeline-model.md` lines 122-127).
- **Playhead:** red vertical line `ruler_height`→bottom + downward triangle (size 8) at top; `x =
  current_frame*ppf - viewport.min_x` (`timeline-model.md` lines 128-132).
- **Clip visuals** (`timeline-model.md` lines 134-152): rounded rect (radius 3), fill =
  `source_clip_type` theme color α 0.45 selected / 0.3; 3 px left color strip; selected border white
  α0.9 width 1.5 else primary 0.5; **missing media** red wash + red border; label bar height 16
  (`"<name>  <timecode(duration)>"`, underline if linked); trim handles 4 px both edges; video →
  thumbnail strip, image → tiled center, audio → waveform; keyframe diamonds (opacity/position/scale/
  crop) near clip bottom (volume kfs **not** here — they live on the rubber band).
- **Waveform** (`timeline-model.md` lines 146-152): map trim window to sample indices, bars =
  `int(draw_width)`, peak-detect MIN per bar, `db_range = 50`, static shift = `db_from_linear(volume)
  / 50`, per-bar volume only when volume track active or fades present.
- **Volume rubber band (audio)** (lines 154-166): body inset 16 top/1 bottom; dB→Y via **draw axis
  +6…−60** (distinct from the −60…+15 editing range and the −120 FOUNDATION value — **keep all three
  dB constants distinct**, ruling #9); polyline through kfs with per-segment interp (linear straight /
  hold step / smooth 12-step); fades as wedges in a fixed fade lane (left knee `min_x+6`, right
  `max_x-6`).
- **Opacity fades (non-audio)** (lines 168-172): fade wedges + knees only — **no draggable opacity
  LINE for video** (FOUNDATION §6.3 overstates this; opacity still sampled per frame via `opacity_at`
  for compositing).
- **Acceptance test:** the geometry/ruler/sampling functions are pure and unit-tested for parity
  (golden tick positions, `clip_rect`, waveform bar mapping); the canvas re-renders selection by clip
  ID across state changes (selection persists — FR-9).

**Implementation context:** `src-ui/editor` (timeline subtree) + reads `palmier-edit::geometry` and
`palmier-model` (`volume_at`/`opacity_at`/`sample`/`VolumeScale`). Reference (drawing math, ports
1:1): `Timeline/ClipRenderer.swift`, `Timeline/TimelineRuler.swift`, `Timeline/PlayheadOverlay.swift`,
`Timeline/TimelineView.swift:201` (`drawContent` order). Docs: `timeline-model.md` §"Clip canvas
visuals" / §"Ruler" / §"Playhead" / §"Volume rubber band" / §"Opacity envelope".

**Dependencies:** E3-S1, E3-S5 (geometry), Epic 2 (`palmier-model` sampling: `volume_at`,
`opacity_at`, `VolumeScale`, `KeyframeTrack::sample`). Independent of E3-S6/E3-S7 internals.

**Parallel-safe?** Yes vs the Rust engine stories (different subtree, `src-ui/editor`). Sequence
before E3-S10 (input controller renders into this canvas).

---

### E3-S10 — Timeline input controller: tools, selection, drag/trim/split, snap wiring, undo

**Intent:** As the user, I drive the timeline like Premiere — Pointer/Razor tools, click + marquee
selection, drag/trim/split with live snap, and Ctrl+Z/Shift+Z undo — wiring the canvas to the
`palmier-edit` engines + `palmier-history` via Tauri commands.

**Acceptance criteria:** Implement the input controller in `src-ui/editor`, dispatching through Tauri
commands into `palmier-edit` + `palmier-history` (frontend never touches the engines directly —
strict layering, FOUNDATION §4):
- **Tool modes (FR-9):** Pointer (V) select/move/trim; Razor (C) split at `razor_preview_frame ??
  frame_at(x)` on mouseDown over a clip (razor previews the cut in mouseMoved using its own snap
  state) (`edit-engines.md` lines 151-152).
- **Selection (FR-9):** single click selects (clears others unless Shift/Ctrl); Shift-click toggles
  membership (expand to link group when `linked_on = !Alt`); Ctrl-click toggles; marquee on empty
  space rubber-band selects (`base_selection ∪` intersecting clips); **selection persists across
  re-renders by clip ID** (`edit-engines.md` lines 153-163).
- **Drag/trim/split (FR-10/FR-13):** mouseDown sub-mode from `local_x` (E3-S7); live snap via E3-S5
  (`find_snap` with two move probes); commit move → `move_clips`, trim → `commit_trim`, split (razor /
  Ctrl+K / Q-W playhead trims) → orchestration `split_clip`/playhead trim (`edit-engines.md` lines
  112-128, 164-170). Cross-track move honors `is_compatible` (**all visual interchangeable**, ruling
  #12). `Alt`+drag = duplicate. **No Slip/Slide** (ruling #11) — do not bind Alt-drag-middle / Ctrl+Alt
  to slip/slide.
- **Range selection:** Shift-drag on ruler → `TimelineRangeSelection` (snaps to edges + playhead),
  edges re-draggable within 8 px slop; drives ripple-delete-range (`edit-engines.md` line 174,
  `timeline-model.md` lines 195-199).
- **Playhead/ruler:** click ruler seeks; J/K/L, Space, Home/End, Shift+arrow per FOUNDATION §6.3
  (transport wiring may stub to a no-op preview until Epic 5; **the timeline-side seek/scrub is in
  scope here**).
- **Undo (FR-9 / §10):** Ctrl+Z / Ctrl+Shift+Z drive the **user** stack (E3-S8); every edit is one
  undo entry.
- **Performance:** add-clip-via-drag **< 100 ms perceived** (SM-3) — assert in the §11.3 hand-edit
  e2e.
- **Acceptance / e2e:** contributes the hand-edit half of the **§11.3 hand-editing e2e exit gate**
  (drag a clip across tracks → trim with snap → split at playhead (Ctrl+K) → ripple-delete a range →
  Ctrl+Z/Ctrl+Shift+Z → state correct); SM-3 latency asserted.

**Implementation context:** `src-ui/editor` (input/controller) → Tauri commands → `palmier-edit`
(`drag`, `snap`, `orchestration`, `split`) + `palmier-history`. Reference:
`Timeline/TimelineInputController.swift` (the behavior spec for drag/selection/tool modes). Docs:
`edit-engines.md` §"Selection & tool modes" + §"Trim" + §"Split"; `timeline-model.md` §"Drag/trim
semantics".

**Dependencies:** E3-S9 (canvas to render into), E3-S5/E3-S6/E3-S7 (engines + drag + orchestration),
E3-S8 (undo). The integrating story — lands last in the epic.

**Parallel-safe?** No — it is the integration point that composes every other story (canvas + all
engines + history). Schedule last.

---

## Story dependency / parallelism summary

| Story | Depends on | Parallel-safe | Crate(s) |
|---|---|---|---|
| E3-S1 model value types | Epic 2 | No (foundation) | palmier-model |
| E3-S2 RippleEngine | E3-S1 | Yes | palmier-edit |
| E3-S3 OverwriteEngine | E3-S1 | Yes | palmier-edit |
| E3-S4 Split/Trim math | E3-S1, Epic 2 | Yes | palmier-edit |
| E3-S5 SnapEngine + geometry | E3-S1 | Yes | palmier-edit |
| E3-S6 Orchestration | E3-S2,S3,S4,S8 | No | palmier-edit |
| E3-S7 Drag state machine | E3-S1,S4,S5 | Partly | palmier-edit |
| E3-S8 palmier-history | Epic 2 | Yes | palmier-history |
| E3-S9 Timeline canvas | E3-S1,S5, Epic 2 | Yes | src-ui/editor |
| E3-S10 Input controller | E3-S5,S6,S7,S8,S9 | No (integration) | src-ui/editor |

**Suggested wave order:** (1) E3-S1 → (2) parallel: E3-S2, E3-S3, E3-S4, E3-S5, E3-S8, E3-S9 → (3)
E3-S6, E3-S7 → (4) E3-S10. Each `palmier-edit` mod is its own file so S2-S5 run in parallel
worktrees; S9 is a different subtree (`src-ui/editor`) and runs alongside the Rust mods.

## Port risks carried into stories (from edit-engines.md §"Port risks")
- **Rounding parity** (E3-S2/S3/S4): every source↔timeline conversion is `f64::round` ties-away —
  golden tests over `speed ∈ {0.25, 0.5, 1.0, 1.7, 4.0}` (edit-engines.md 221-222).
- **Sticky 1.5× not 2.5×** (E3-S5): port the reference 1.5 (ruling #10).
- **Half-open ranges + touching-merge** (E3-S1/S2): `[start, end)`, `merge_ranges` merges on `<=`.
- **Refuse-the-whole-edit atomicity** (E3-S6): validate before any mutation, abort atomically.
- **Overwrite Split is advisory** (E3-S3/S6): VM re-derives right fragment via `split_clip`.
- **Pinned companions** (E3-S6/S7): linked partners + type-incompatible co-selected clips hold row.
- **Two move-snap probes** (E3-S5/S7): both edges of every selected clip, not just the lead start.
- **Linked split regroups right halves under a new `link_group_id`** (E3-S6).
- **`has_no_source_media` (image/text)** removes the source trim cap (E3-S4/S7).
