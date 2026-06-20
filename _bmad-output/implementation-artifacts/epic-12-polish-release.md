---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 12 — Polish, Settings, Telemetry & Release"
created: 2026-06-20
---

# Epic 12 — Polish, Settings, Telemetry & Release

## Epic goal

Port the remaining "surround" of the editor and ship it: the right-rail **Inspector** panel
(`Sources/PalmierPro/Inspector/`), the editor **Toolbar** (`Sources/PalmierPro/Toolbar/`), the
**5-tab Settings** window + **Account/billing** + **Help/MCP-instructions** + **Feedback**
(`Sources/PalmierPro/{Settings,Account,Help}/`), **telemetry + logging + crash handling**
(`Sources/PalmierPro/{Telemetry/Telemetry.swift,Utilities/Log.swift}`), the **Tauri Ed25519
updater** (`Sources/PalmierPro/App/Updater.swift`), and the **packaging/release** artifacts
(`.msi` + `.AppImage`/`.deb`/`.rpm`). It is a behavior-parity port: the Inspector's tab-visibility
rules, scrub-field ranges, and keyframe-lane interactions must reproduce the reference 1:1; the
Settings prefs must replicate the reference's "absent ⇒ ON" UserDefaults semantics and the
launch-snapshot telemetry/privacy restart-required UX; the keychain account string, Sentry option
semantics, and billing URL allowlist must be carried over verbatim. This is the **M5 release epic** —
its exit also re-runs the full SM regression set and the §11.6 MCP compatibility suite as the
release gate.

## PRD acceptance this epic must satisfy (PRD §4.12 / §10 Epic 12)

- **FR-41 Inspector.** Selection-driven tabs (Text/Video/Audio/AI-Edit/Details) with scrubbable
  number fields, color/font pickers, keyframes side panel + per-property lanes. Volume field range
  **−60…+15 dB** (ruling #9 — **verify keyframe-storage floor in code before locking the field**).
- **FR-42 Settings & account.** 5 tabs (Account/General/Models/Agent/Storage); Clerk sign-in +
  Convex billing; General toggles use `io.palmier.pro.{notifications,telemetry}.enabled` (ruling #6),
  telemetry restart-required.
- **FR-43 Telemetry, logging, updater.** `tracing` to platform log dirs (rotated daily, 7 days),
  Sentry (DSN build-injected, PII off, **opt-out default** — OQ-2), Tauri **Ed25519** updater
  (manifest URL OQ-9 backend, single `stable` channel OQ-1-update).
- **FR-44 Packaging.** Build `.msi` (Windows) and `.AppImage` + `.deb` + `.rpm` (Linux) — Flatpak is
  OQ-8 (out of v1).

**Cross-cutting NFR validated here:** **SM-10 Memory** — < 800 MB RSS idle / < 2.5 GB RSS
editor+preview on a 200-clip project (PRD §7, FOUNDATION §10).

**Milestone (PRD §12):** **M5 — Export Polish + Release.** Epic 12 + UJ-5 schema finalization.
Realizes **UJ-4** (the manual editor's Inspector/toolbar polish) and completes the product surround.
**Release gate (PRD §12 M5, replaces "overall parity"):** re-run the **full §11.6 MCP compatibility
suite** + the **complete SM regression set (SM-1, SM-1b, SM-2..SM-13, SM-C1..C3)** green, plus the
four §11.3 e2e workflows. Resolve **OQ-10 (branding)** and **OQ-1 (ProRes alpha)** before public
launch. This epic validates **SM-10** and owns the SM-regression release bar.

**Crates:** `palmier-telemetry` (owner: Sentry init + `tracing` subscriber + categorized loggers +
crash file), `palmier-update` (Tauri updater glue), `palmier-auth` (account/plan/credit state,
billing actions, keyring API-key CRUD, misconfigured guard — **most of this crate is built in Epics
1/8/9; Epic 12 adds the Settings/Account UI wiring and the billing-action + feedback commands**),
`palmier-model` (volume-dB keyframe-floor verification only — ruling #9), `palmier-tauri` (bundler
config, updater plugin, menu "Check for Updates"), `src-ui/settings` (5 settings tabs + Help window +
feedback dialog + account cards), `src-ui/editor` (Inspector + Toolbar React ports).

---

## Spike / risk gate

**This epic is NOT spike-gated as a whole** (it is not Epic 5, which is gated by Spike S-1
wgpu→WebView). It lands at **M5**, by which point S-1 (M1), S-1b (M1), S-2 (M3), S-3 (M4), and S-4
(M1) have all resolved. No story below assumes an unresolved presentation mechanism.

Two **narrow** gating concerns, both internal sequencing rather than blocker spikes:

- **OQ — Keyframe-storage dB floor (PRD §13.8, ruling #9).** Three distinct dB constants exist in the
  reference (`VolumeScale.floorDb = -60`, rubber-band draw axis +6…−60, and an unverified keyframe
  *storage* floor that FOUNDATION §5.3/§5.5 claims is −120). **E12-S1 is an engineering-confirm story**
  that resolves this against `palmier-model` before the Inspector volume field (E12-S5) locks its
  clamp. The Inspector **field** clamps to **[−60, +15]** (reference `VolumeScale`); the keyframe
  **storage** floor is confirmed in code — do not silently adopt FOUNDATION's 0 ceiling.
- **External-dependency gates (do not block the epic):** the **updater manifest URL is OQ-9** (Convex
  backend) and the **Sentry DSN is build-injected** (may be a placeholder in dev) — both **must
  degrade silently when unconfigured** (no spurious "update check failed" / no Sentry init), exactly
  as the reference's Sparkle no-ops without `SUFeedURL`. These are handled inside E12-S12/E12-S13, not
  as separate spikes.

The **Inspector** (E12-S2..E12-S9) and the **Settings/Help/Feedback** (E12-S10..E12-S11) are large,
parallel-safe view-layer ports that can start as soon as M1's model + history + auth crates exist;
the **telemetry/logging/updater/packaging** stories (E12-S12..E12-S15) are independent backend +
build-config work. The **release-gate story (E12-S16)** is last and depends on every other epic.

---

## Reference-parity constants (bind every story — cite, do not re-derive)

- **Inspector volume:** `VolumeScale.floorDb = -60`, `ceilingDb = +15`;
  `dbFromLinear(l) = l<=0 ? floor : clamp(20*log10(l), floor, ceil)`;
  `linearFromDb(db) = db<=floor ? 0 : 10^(min(db,ceil)/20)`. At/below floor render **"−∞ dB"** and
  store a hard **0** (true mute). (ruling #9; `inspector.md` §VolumeScale.)
- **Scrub modifiers:** **Shift ⇒ coarse ×10**, **Ctrl ⇒ fine ×0.1** (reference `Command` → Ctrl on
  Win/Linux). Drag threshold `abs(dx) > 3` px. (`inspector.md` §ScrubbableNumberField — FOUNDATION
  §6.7 only mentions fine; preserve coarse.)
- **Position range −10..10** (off-canvas allowed; displayed in px via canvas dims). **Scale upper
  bound ∞** (`0.01...∞`, only lower bound enforced). (`inspector.md` §Scrub-field-ranges.)
- **Keyframe default interpolation = Smooth (smoothstep)** (ruling #8); context menu shows
  Linear/Smooth/Hold with default `.smooth`. Lane snap threshold **4 px**; snap targets include
  in-range playhead, both clip edges, **and keyframe frames of all *other* properties on the same
  clip**. `AnimatableProperty` = position, scale, rotation, opacity, crop, volume. (`inspector.md`
  §KeyframesPanel; FOUNDATION §5.5.)
- **Settings prefs:** `io.palmier.pro.{notifications,telemetry,mcp}.enabled`, **absent ⇒ ON**
  (ruling #6); stored under FOUNDATION's `settings.json` (`%APPDATA%\PalmierProWin\settings.json` /
  `~/.config/palmier-pro/settings.json`), **not** a macOS preference domain.
- **Telemetry launch-snapshot:** `enabledForCurrentLaunch` captured at boot; toggling shows
  **"Restart Palmier Pro to apply"** and takes effect only after restart. (ruling #6;
  `settings-account-app.md`.)
- **Keychain:** service = `palmier-pro`, **account = `anthropic-api-key`** (ruling #5 — wrong name
  silently loses the user's saved key), accessible-after-first-unlock equivalent, update-then-add.
- **Billing URL allowlist (security control — port verbatim):** open a returned billing/checkout URL
  **only if** scheme = `https` **AND** host ∈ {`checkout.stripe.com`, `billing.stripe.com`}.
  (`settings-account-app.md` §Auth/account state machine.)
- **Sentry options:** `sendDefaultPii=false`, env `development`(DEBUG)/`production`,
  `tracesSampleRate=0.1`, app-hang timeout 8.0 s, `attachStacktrace=true`, release name
  **`palmier-pro-win@<version>+<git_sha>`** (FOUNDATION §6.16). Start Sentry **only if** enabled AND
  DSN non-empty.
- **Log targets** (`tracing`): `app, editor, export, preview, mcp, generation, project,
  transcription, search`; daily-rotated, 7-day retention; `%LOCALAPPDATA%\PalmierProWin\Logs\
  palmier.log` (Win) / `~/.local/state/palmier-pro/logs/palmier.log` (Linux); crash →
  `crashes/<timestamp>.log` + forward to Sentry. `warning` → Sentry breadcrumb; `error`/`fault` →
  Sentry capture-message. (FOUNDATION §6.16; `settings-account-app.md`.)
- **Window config:** Settings 980×640 / min 760×480; Help 900×560 / min 820×520; Feedback 480×480 /
  min 480×420. Persist size/pos via `tauri-plugin-window-state` (replaces macOS autosave names
  `PalmierProSettings-v2` / `PalmierProHelp-v1`). (`settings-account-app.md` §Window-config.)
- **Updater:** Tauri 2 updater, **Ed25519**-signed JSON manifest; **silently disabled when no signed
  feed configured** (mirrors Sparkle no-op without `SUFeedURL`). Menu "Check for Updates…" →
  `check now` command; `update_available`/`update_version` surfaced via Tauri event. (ruling — §8.4;
  `settings-account-app.md` §Updater.)
- **Feedback:** message ≤ 10000 chars, optional email when signed-out, include-screenshot toggle,
  "may we contact you" toggle → Convex `feedback:send` / `/v1/feedback` with
  `(message, mayContact, appVersion, osVersion, optional email, screenshotPngBase64)`.
- **Design tokens** for all Inspector/Settings UI come from `tokens.json` (Epic 1/Epic 5 design
  pipeline); use **`#F29933`** accent-timecode and **`#F5EFE4`** accent-primary (ruling #21 — not
  FOUNDATION §9's `#F2994A`/`#F5F0E4`).

---

## Stories

### E12-S1 — Verify keyframe-storage dB floor; lock VolumeScale constants

*As the build team, I want the keyframe-storage volume floor confirmed against `palmier-model` so the
Inspector volume field and the keyframe lane agree on a single, parity-correct dB range before any
volume UI is built.*

**Acceptance criteria:**
- Given the three reference dB constants (`VolumeScale.floorDb = -60`, `ceilingDb = +15`; rubber-band
  draw axis +6…−60; FOUNDATION §5.3/§5.5 claim of a −120 keyframe-storage floor), when the
  `palmier-model` `KeyframeTrack<volume>` / clip `volume` storage is inspected and a round-trip test is
  run, then the **actual storage floor is documented** (−60 vs −120) and a one-line decision recorded
  in this epic's Timeline (ruling #9: field clamps to **[−60, +15]**; storage floor confirmed, **not**
  silently set to FOUNDATION's 0 ceiling).
- A `palmier-model` unit test asserts `dbFromLinear(0.0) == floor` renders "−∞ dB" path and stores
  linear `0.0` (true mute), `dbFromLinear(1.0) == 0.0 dB`, and `linearFromDb(15.0) == 10^(15/20)`
  (amplification > 0 dB is representable — do not clamp to 1.0).
- The decision is exported as a shared constant (`VolumeScale { floor_db: -60.0, ceiling_db: 15.0 }`)
  consumed by both the Inspector field (E12-S5) and the keyframe lane (E12-S8) — **single source of
  truth**, no duplicate literals.

**Implementation context:** `palmier-model` (`Clip.volume`, `KeyframeTrack`, new `VolumeScale`
helper). Reference: `Inspector/InspectorView.swift` (bottom, `VolumeScale`), ruling #9,
`inspector.md` §VolumeScale + §Volume-range-discrepancy gotcha.

**Dependencies:** Epic 2 (`palmier-model`). **Parallel-safe?** No — touches `palmier-model`; must
land before E12-S5/E12-S8. Small (≈0.5 day).

---

### E12-S2 — Inspector shell: header/title resolution + tab gating

*As an editor user, I want the right-rail Inspector header and tab set to switch correctly with my
selection so I always see the right context (Timeline / Inspector / Source) and the right tabs.*

**Acceptance criteria:**
- Given a selection state, when the Inspector renders, then the **header title/icon** resolves
  exactly: visual-or-audio clip(s) → "Inspector" (`slider.horizontal.3`); media asset → "Source"
  (`info.circle`); nothing → "Timeline" (`info.circle`); while marquee-selecting →
  "Inspector" + body "`N` selected" centered. (`inspector.md` §Header/title.)
- Given selected clips, when `availableTabs` is computed, then tabs appear in **this exact order**:
  `Text` (iff `isSingleText`), `Video` (iff `nonText` non-empty), `Audio` (iff `audios` non-empty),
  `AI Edit` (iff `aiEditEligible && !AccountService.isMisconfigured`). The tab bar is **hidden when
  only one tab exists**. (`inspector.md` §Clip-tab-set; FOUNDATION §6.7.)
- `aiEditEligible` = exactly one visual clip resolving to a visual `MediaAsset`, and any selected
  audio clips are all link-partners of that visual (a linked A/V pair counts as one).
- `resolvePreferredTab` on selection change: single text → force `.text`; leaving text → drop to
  `.video`; always clears `cropEditingActive`. Active tab = `preferredTab` if still available, else
  first available.
- Given **no selection**, the "Project" section (name = file stem, path middle-truncated) and
  "Format" section (Resolution `W × H`, Frame Rate `fps fps`, Aspect Ratio = `W:H` reduced by gcd,
  Duration via `formatDuration(totalFrames/fps)` as `H:MM:SS` or `M:SS`) render. (`inspector.md`
  §Project-metadata.)

**Implementation context:** `src-ui/editor` (Inspector root, React/TS — pure view over reactive
state via Tauri commands/events; never touches FFmpeg/wgpu). Wire `aiEditEligible` /
`isMisconfigured` to `palmier-auth` account state. Reference: `Inspector/InspectorView.swift`
(headerTitle/headerIcon, availableTabs, resolvePreferredTab). Govern: `inspector.md`, FOUNDATION §6.7.

**Dependencies:** Epic 3 (selection model), Epic 1 (`palmier-auth` account state). **Parallel-safe?**
Yes — new Inspector files in `src-ui/editor`; coordinate only with E12-S3..S9 siblings (same subtree,
disjoint files).

---

### E12-S3 — ScrubbableNumberField + InspectorPositionFields components

*As an editor user, I want drag-to-scrub numeric fields with coarse/fine modifiers and type-to-edit so
I can adjust clip properties precisely like in the reference.*

**Acceptance criteria:**
- Given a scrub field, when I drag horizontally past **3 px** (window-space), then it scrubs with
  `next = clamp(dragStartValue + dx * sens / mult)` (`mult` = displayMultiplier, treated as 1 if 0);
  **Shift ⇒ sens ×10 (coarse)**, **Ctrl ⇒ sens ×0.1 (fine)** (`Command`→Ctrl); `onChange` fires live
  during drag, `onCommit` on pointer-up; cursor is `ew-resize`. (`inspector.md`
  §ScrubbableNumberField; preserves coarse that FOUNDATION §6.7 omits.)
- Given a click without drag, when I enter edit mode and type, then parse strips the suffix, trims,
  replaces "," with ".", parses `Double`, **divides by displayMultiplier**, clamps to range, commits.
- A **mixed value** (`value == nil`) renders **"—"** and **blocks scrub** (multi-select shared-value
  semantics).
- `InspectorPositionFields` renders X then Y (fieldWidth 36, trailing labels "X"/"Y"), each driven by
  `topLeftAt(frame).x/.y` shared across selected clips; `apply` sets one axis at a time
  (`applyPosition(setX:setY:)`, other axis nil); `commit` wraps all clips in **one** undo group
  "Change Position". (`inspector.md` §InspectorPositionFields.)
- **apply\* creates NO undo entry; only commit\* does**, and multi-clip commits are a **single named
  group** (else SM-4 atomic-undo breaks).

**Implementation context:** `src-ui/editor` (`ScrubbableNumberField`, `InspectorPositionFields`).
JS pointer events on a div replace `NSView` mouse tracking. apply\* → transient "preview" Tauri
command (no undo); commit\* → committed mutation pushing onto the **user** undo stack
(`palmier-history`). Reference: `Inspector/Components/{ScrubbableNumberField,InspectorPositionFields}
.swift`. Govern: `inspector.md` §ScrubbableNumberField/§InspectorPositionFields.

**Dependencies:** Epic 3 (`palmier-history` undo groups), E12-S2 (shell). **Parallel-safe?** Yes —
self-contained component files; consumed by E12-S5/S6/S7.

---

### E12-S4 — ColorField + FontPickerField + TextContentField components

*As an editor user, I want live color, font, and multi-line text editors in the Inspector so text and
appearance edits feel native and don't drop keystrokes.*

**Acceptance criteria:**
- **ColorField:** a swatch that, on click, opens a color picker firing `onChange` **live during drag**
  (not just on mouse-up), with an alpha toggle bound to `supportsOpacity`; emits **sRGB RGBA**; the
  first seed notification is suppressed. (`inspector.md` §ColorField — replaces `NSColorPanel` +
  `colorDidChangeNotification`.)
- **FontPickerField:** two groups — **"Featured" = bundled families** (from a `palmier-text` font
  enumeration command), then **"All fonts" = system families** (`fontdb`/`cosmic-text`); each
  previewable row renders in its own font; **hover fires a non-committing preview** (`onPreview`);
  selecting fires `onChange`; closing without a pick fires `onCancel` (reverts preview); current font
  shows a checkmark. (`inspector.md` §FontPickerField; FOUNDATION §6.6.)
- **TextContentField:** a `<textarea>`, plain text only (no smart quotes/dashes/spell), app owns undo
  (`allowsUndo=false`), commit on blur, live apply on input — and **must not stomp the caret**: only
  overwrite text when the editor is not focused and strings differ (the reference NSTextView-wrapper
  bug to avoid). (`inspector.md` §Text-tab Content + §gotcha.)

**Implementation context:** `src-ui/editor` (`ColorField`, `FontPickerField`, `TextContentField`);
new Tauri command in `palmier-text` for font family enumeration (Featured = bundled `Resources/Fonts/`
families; All = `fontdb` system). Reference: `Inspector/Components/{ColorField,FontPickerField,
TextContentField}.swift`, `Utilities/BundledFonts.swift`. Govern: `inspector.md`, FOUNDATION §6.6.

**Dependencies:** `palmier-text` (Epic 5/10 font stack), E12-S2. **Parallel-safe?** Yes — distinct
component files; the only backend touch is one additive `palmier-text` command.

---

### E12-S5 — Inspector Video + Audio tabs (Transform / Playback / Levels)

*As an editor user, I want the Video and Audio inspector tabs so I can edit transform, opacity, speed,
volume, fades, crop, and flips on one or many selected clips.*

**Acceptance criteria:**
- **Video tab** renders a collapsible **Transform** section (default expanded) with a **"Reset
  Transform"** button (resets `transform = Transform()`, `opacity = 1`, nulls
  opacity/position/scale/rotation tracks, zeroes fade-in/out frames, resets fade interpolations to
  linear — one named undo group), plus rows Position, Scale, Rotation, Opacity, Crop, Flip, and a
  **Playback → Speed** row. (`inspector.md` §Video-tab; FOUNDATION §6.7.)
- Scrub-field ranges are **exact**: Position X/Y `-10..10` (mult = canvasW/H, `%.0f`); Scale `0.01..∞`
  (mult 100, `%.0f %`); Rotation `-3600..3600` (`%.0f °`); Opacity `0..1` (mult 100, `%.0f %`); Speed
  `0.25..4.0` (`%.2f x`, sens 0.01); Fade In/Out `0..maxSeconds` (`%.2f s`, sens 0.02,
  `maxSeconds = durationFrames/fps` single-clip else 60.0, `frames = round(seconds*fps)`). Values
  **sample at `activeFrame`** so they reflect keyframe state. (`inspector.md` §Scrub-field-ranges.)
- **Volume** field binds to `liveVolumeKfDb(at:activeFrame) ?? VolumeScale.dbFromLinear(clip.volume)`,
  range **−60…+15 dB** (E12-S1 constant), `%.1f dB`, overrides **"−∞ dB"** at floor (stores true-mute
  0), sens 0.3. **Audio tab** "Levels" = Volume + Fade In (left edge) + Fade Out (right edge); Speed
  section shown only when no visual clip selected. (`inspector.md` §Audio-tab.)
- **Crop** (single clip only, 40% disabled otherwise): toggle drives `cropEditingActive`;
  `CropAspectLock` menu — `.free` leaves crop untouched, `.original` commits `Crop()`, presets commit
  `cropFittingAspect`; crop is keyframeable. **Flip**: two toggles reading
  `transform.flipHorizontal/.flipVertical`, each commits `!current` across all selected clips under one
  group ("Flip Horizontal"/"Vertical"). (`inspector.md` §Crop/§Flip.)
- **Multi-clip**: each field shows `sharedClipValue` (nil → "—" mixed, scrub disabled); apply\* fans
  out to every selected clip; commit wraps all in **one** named undo group.

**Implementation context:** `src-ui/editor` (Video/Audio tab views) over `palmier-model` Transform +
keyframe ops and `palmier-history` grouped commits; uses E12-S1 `VolumeScale` and E12-S3 scrub
fields. Reference: `Inspector/InspectorView.swift` (Transform/Playback/Levels, `sharedClipValue`,
crop/flip). Govern: `inspector.md` §Video/Audio/Crop/Flip, FOUNDATION §6.7, ruling #9.

**Dependencies:** E12-S1, E12-S2, E12-S3; Epic 3 (history). **Parallel-safe?** Yes within
`src-ui/editor` (distinct tab files), but **shares `palmier-model` VolumeScale with E12-S1/E12-S8** —
sequence after E12-S1.

---

### E12-S6 — Inspector Text tab (Typography / Appearance / Layout / Content)

*As an editor user, I want a full text-clip inspector so I can set font, size, color, background,
border, shadow, alignment, position, and content for a text clip.*

**Acceptance criteria:**
- Visible **iff `isSingleText`**. `style = clip.textStyle ?? TextStyle()`. Sections:
  **Content** (`TextContentField`, min height 80; every keystroke → `applyClipProperty(rebuild:true)`
  set textContent + `fitTextClipToContent`; end-editing → commit); **Typography** (Font via
  `FontPickerField`, Size scrub **12..300 pt**, `%.0f pt`; font/size changes call
  `fitTextClipToContent`); **Appearance** (Color via `ColorField`, Opacity `0..1`→%, plus
  Background/Border/Shadow each a toggle+ColorField pair); **Layout** (Alignment segmented
  left/center/right committing immediately, Position via `InspectorPositionFields`). (`inspector.md`
  §Text-tab; FOUNDATION §6.7.)
- Color edits route through a **debounced** commit per key (`textColor`/`backgroundColor`/
  `borderColor`/`shadowColor`); the **enable toggle commits immediately** (mixing these wrong spams the
  undo stack during color drags). (`inspector.md` §gotcha.)

**Implementation context:** `src-ui/editor` (`TextTab` view) using E12-S3/E12-S4 components over
`palmier-model` `TextStyle`. Reference: `Inspector/TextTab.swift`. Govern: `inspector.md` §Text-tab,
FOUNDATION §6.7. *Note:* `fitTextClipToContent` exact resize math lives in the reference
`EditorViewModel` and is an Open Question (`inspector.md` open Qs) — implement the call site; the
resize algorithm is owned by the Epic 5 text/preview parity work.

**Dependencies:** E12-S2, E12-S3, E12-S4. **Parallel-safe?** Yes — `TextTab` is its own file.

---

### E12-S7 — Inspector Details (Source) + AI Edit tabs

*As an editor user, I want read-only source metadata for a selected media asset and the AI-Edit
controls so I can inspect assets and trigger AI enhance/edit/generate from the Inspector.*

**Acceptance criteria:**
- **Details (Source)** for a selected media asset: tab bar `[Details, AI Edit]` only when asset is
  visual **and** account ok, else Details only. Details = identity header (name + "AI" badge if
  generated), File section (Type, Dimensions if non-audio, Duration if >0 and not image, Size via byte
  formatter, Path middle-truncated); for generated assets: References strip
  (`GenerationReferencesStrip`), Generated (model display name, aspect ratio, resolution, duration),
  Prompt (copyable via clipboard). (`inspector.md` §Media-asset-Source-inspector.)
- **AI Edit** available for a single AI-eligible visual clip or a selected visual media asset (account
  not misconfigured). Scope toggles when clip context ("Replace clip source" preserving
  speed/volume/trim/transform; "Use trimmed portion only" iff `trimStart>0||trimEnd>0`); "AI Enhance"
  (Upscale menu with cost, Edit, Rerun, Create Video for images); video-asset "AI Audio" (Music + SFX,
  "Place on timeline" toggle). Each action computes `availability` (enabled + disabled reason);
  submissions route through the generation lifecycle. (`inspector.md` §AI-Edit-tab.)
- AI-Edit gating depends on `palmier-auth` (`isMisconfigured`, `aiAllowed`); when misconfigured/signed
  out, AI-Edit is hidden/disabled with the reference reason text.

**Implementation context:** `src-ui/editor` (`DetailsTab`, `AIEditTab`, `GenerationReferencesStrip`).
AI-Edit actions call `palmier-gen` (Convex generation lifecycle, Epic 9) + `palmier-tools`
(`upscale_media`, `generate_video/audio`); byte formatter = Rust util or `Intl.NumberFormat`. Prompt
copy → Tauri clipboard. Reference: `Inspector/AIEditTab.swift`, `Inspector/Components/
GenerationReferencesStrip.swift`. Govern: `inspector.md` §AI-Edit/§Source, FOUNDATION §6.7/§6.11.

**Dependencies:** Epic 9 (`palmier-gen` lifecycle + `palmier-tools` upscale/generate), Epic 1
(`palmier-auth`), E12-S2. **Parallel-safe?** Yes — own tab files; only consumes existing Epic 9
commands.

---

### E12-S8 — Keyframes side panel + per-property lanes

*As an editor user, I want a keyframes side panel with per-property lanes so I can add, move, delete,
and re-interpolate keyframes on a single selected clip.*

**Acceptance criteria:**
- "Keyframes" toggle (`keyframesPanelVisible`) is **enabled only when exactly one clip is selected**;
  when on, the tab splits into a two-column HStack (controls left with right padding
  `controlsColumnWidth + sm`, Divider, `KeyframesPanel` right). (`inspector.md` §Video-tab.)
- **Per animatable row** appends [prev-kf chevron][diamond stamp][next-kf chevron]: stamp = filled
  diamond if a keyframe exists at `activeFrame` (toggles remove), hollow otherwise (adds via
  `stampKeyframe`); disabled (40%) when the playhead is outside the clip; chevrons navigate
  `previous/nextKeyframeFrame`, disabled when none. `AnimatableProperty` = position, scale, rotation,
  opacity, crop, volume. (`inspector.md` §Keyframe-controls.)
- **Panel rows:** video clip → Position, Scale, Rotation, Opacity, Crop; audio clip → Volume only.
  `KeyframesMetrics`: rulerHeight 18, stripHeight 14, headerHeight 32, rowHeight 22, stampButtonWidth
  22, navButtonWidth 6, controlsColumnWidth 34, diamondSize 8. Frame↔x:
  `t = clamp((f-clipStart)/span, 0, 1)`, `x = t*width`, `span = max(1, endFrame-startFrame)`,
  inverse rounds. (`inspector.md` §KeyframesPanel.)
- **Lane drag** (minimumDistance 0): within **`hitTolerance = 7` px** of a kf → begin kf drag (raw
  frame → `applySnap` → `applyMoveKeyframe`, commit if moved else revert); else empty-area scrub
  (seek playhead). **Snap** (`snapThresholdPixels = 4`): targets = in-range playhead, both clip edges,
  **and keyframe frames of all *other* properties on the same clip**; on snap draw a dashed yellow
  guide, candidate clamped to `[startFrame, endFrame]`. (`inspector.md` §KeyframesPanel — the
  other-property snap targets are easy to miss.)
- **Context menu** per kf: Linear / Smooth / Hold (checkmark on current; **default shown as
  `.smooth`** — ruling #8) + "Delete Keyframe". Single red playhead overlay drawn only when the
  playhead is inside the clip.

**Implementation context:** `src-ui/editor` (`KeyframesPanel`, `KeyframesLaneRow`, `ClipRulerBlock`)
rendering diamonds/playhead via canvas/SVG; ops map to `palmier-model` `KeyframeTrack`
(stamp/move/remove/setInterpolation) committed through `palmier-history`; snap reuses the Epic 3
`SnapEngine` (`findSnap`, 4 px). Reference: `Inspector/Keyframes/KeyframesLane.swift`. Govern:
`inspector.md` §KeyframesPanel, FOUNDATION §5.5/§6.7, rulings #8/#9.

**Dependencies:** E12-S1 (VolumeScale), E12-S2, E12-S3, Epic 3 (`SnapEngine`, `palmier-history`).
**Parallel-safe?** Yes within `src-ui/editor`; shares `palmier-model` keyframe ops — coordinate with
E12-S5.

---

### E12-S9 — Editor Toolbar (undo/redo, tools, clip-edit, add-text, zoom)

*As an editor user, I want the toolbar strip above the editor so I can switch tools, undo/redo, split/
trim, add text, and zoom with the reference bindings.*

**Acceptance criteria:**
- A horizontal strip renders the reference groups with exact bindings (Cmd→Ctrl): **Undo/Redo**
  (Ctrl+Z / Ctrl+Shift+Z), **Tool mode** (Pointer V, Razor C), **Clip edit** (Split at Playhead
  Ctrl+K, Trim Start Q, Trim End W), **Insert** (Add Text T), and a spacer pushing a **Zoom slider
  log-mapped from `min_zoom_scale` to `max_zoom_scale`** to the right. (FOUNDATION §6.8;
  `settings-account-app.md` §ToolbarView — "log-mapped zoom slider".)
- Each button dispatches the corresponding editor action as a Tauri command/event (not `NSApp
  .sendAction`); active tool mode is visually indicated; undo/redo enabled-state reflects the **user**
  stack. Layout constant `toolbarHeight = 38` (design-tokens Layout). (`design-tokens.md` §Layout.)

**Implementation context:** `src-ui/editor` (`Toolbar`) over Epic 3 editor commands + `palmier-history`
state. Reference: `Toolbar/ToolbarView.swift`. Govern: FOUNDATION §6.8, `settings-account-app.md`,
`design-tokens.md` (toolbarHeight 38, IconSize tokens).

**Dependencies:** Epic 3 (tool modes, split/trim, undo/redo), Epic 5 (zoom scale range).
**Parallel-safe?** Yes — `Toolbar` is its own file.

---

### E12-S10 — Settings window: shell + General/Models/Storage tabs

*As a user, I want a Settings window with General, Models, and Storage tabs so I can toggle
notifications/telemetry and manage on-disk caches and downloaded models.*

**Acceptance criteria:**
- A Settings `WebviewWindow` opens at **980×640 (min 760×480)**, dark, transparent titlebar,
  full-size content, size/pos persisted via `tauri-plugin-window-state` (state key replacing
  `PalmierProSettings-v2`); sidebar + detail with a `SettingsTab` enum
  (account/general/models/agent/storage); the **Account tab is hidden when `isMisconfigured`**
  (`visibleTabs` filter). (`settings-account-app.md` §Window-config/§Settings-tabs.)
- **General** always shows **Notifications** toggle (`io.palmier.pro.notifications.enabled`) +
  **Privacy** toggle (`io.palmier.pro.telemetry.enabled`); both **absent ⇒ ON** (ruling #6); the
  Privacy toggle shows **"Restart Palmier Pro to apply"** when its value ≠ `enabledForCurrentLaunch`
  (launch-snapshot). Prefs persist to `settings.json` (FOUNDATION §6.1 path), not a macOS domain.
- **Models** tab: list downloaded Whisper / SigLIP models + sizes + delete buttons; per-model enable
  toggles via the model-preferences backend; cache size shown. (FOUNDATION §6.15; `settings-account-
  app.md` §ModelsPane.) *Note: `ModelPreferences` exact storage format is an Open Question — use the
  Epic 1 model-catalog/preferences command surface.*
- **Storage** tab: Cache row = sum of thumbnail/waveform/media-visual caches, path `~`-relativized,
  "Clear cache"; Media-search section (toggle → enable/disable, index bytes from embedding-store dir,
  model bytes from models dir, clear/remove buttons). (FOUNDATION §6.15; `settings-account-app.md`
  §StoragePane.) Byte sizes come from backend commands; the frontend never reads the filesystem.

**Implementation context:** `src-ui/settings` (Settings shell + `NotificationsPane`/`PrivacyPane`/
`ModelsPane`/`StoragePane`) reading/writing prefs via Tauri commands into the settings store; cache/
index/model byte queries are Tauri commands into `palmier-media`/`palmier-search`/`palmier-transcribe`
disk-cache roots. Reference: `Settings/{SettingsView,NotificationsPane,PrivacyPane,ModelsPane,
StoragePane}.swift`. Govern: `settings-account-app.md`, FOUNDATION §6.15, ruling #6.

**Dependencies:** Epic 1 (settings store + boot prefs, model catalog), Epics 4/10/11 (cache/index/model
dirs for Storage byte queries). **Parallel-safe?** Yes — `src-ui/settings` is a separate subtree from
`src-ui/editor`; disjoint files from E12-S11.

---

### E12-S11 — Settings Account + Agent tabs; Help (Shortcuts+MCP) + Feedback windows

*As a user, I want the Account and Agent settings tabs plus the Help and Feedback windows so I can
manage my subscription/credits, my Anthropic key + MCP server, view shortcuts/MCP-install
instructions, and send feedback.*

**Acceptance criteria:**
- **Account tab** (hidden when `isMisconfigured`): `isLoading` → "Loading…"; signed-in+paid →
  subscription section (plan label, "Cancels <date>" **orange** if `cancelAtPeriodEnd`, "Manage
  subscription") + credits (Remaining card with `CreditSummaryView` + "Resets <date>"; Buy-more card
  with `TopOffField`, range **$5–$1000**, default top-off $20); signed-in+free → plan cards (Pro
  primary / Max secondary from `availablePlans`); signed-out → **"Sign in with Google"**; `lastError`
  shown red. (`settings-account-app.md` §AccountPane; FOUNDATION §6.15.)
- **Billing actions** (`createCheckoutSession`/`createTopOffCheckoutSession`/`createPortalSession`)
  open the returned URL **only if scheme = https AND host ∈ {checkout.stripe.com, billing.stripe.com}**
  (security allowlist — port verbatim). (`settings-account-app.md` §Auth/account-state-machine.)
- **Agent tab:** SecureField with placeholder `sk-ant-...` or masked key (`•`×36 + last 4); **Save**
  when draft non-empty else trash when a key exists; loads/saves via OS keyring **service
  `palmier-pro`, account `anthropic-api-key`** (ruling #5); "Get key" →
  `https://console.anthropic.com/settings/keys`. MCP section: green/grey dot + "Running on
  127.0.0.1:19789" or "Stopped" (**toggle reflects actual server liveness**, not just the pref — ruling
  re port risk), toggle → `setMCPEnabled`, "Setup instructions" → Help MCP tab. (`settings-account-
  app.md` §AgentPane; FOUNDATION §6.15.)
- **Help window** (900×560, min 820×520): two tabs — **Shortcuts** (matches the §6.1 menu table) and
  **MCP** (endpoint `http://127.0.0.1:19789/mcp`; Cursor deep-link + manual JSON; Claude Desktop
  bundled `palmier-pro.mcpb` + `npx -y mcp-remote <endpoint> --allow-http --transport http-only`;
  Claude Code `claude mcp add --transport http palmier-pro <endpoint>`; Codex
  `codex mcp add palmier-pro --url <endpoint>` — the `--allow-http`/`http-only` flags are load-bearing
  for loopback http). (`settings-account-app.md` §Help/MCP; FOUNDATION §6.14/§6.15.)
- **Feedback window** (480×480, min 480×420, not released on close): multi-line message ≤10000 chars,
  email field when signed-out, include-screenshot toggle, "may we contact you" toggle, Submit → Convex
  `feedback:send`/`/v1/feedback` with `(message, mayContact, appVersion, osVersion, optional email,
  screenshotPngBase64)`. Screenshot via Tauri webview capture → PNG base64. (`settings-account-app.md`
  §AccountService feedback; FOUNDATION §6.15/§8.1.)

**Implementation context:** `src-ui/settings` (`AccountPane`, `AgentPane`, `HelpView`, `ShortcutsPane`,
`MCPInstructionsPane`, `FeedbackView`, account fragments). Account/billing/feedback call `palmier-auth`
commands (Clerk JWT-backed Convex actions; billing URL allowlist enforced **server-of-truth in
`palmier-auth` `open` path**); key CRUD via `palmier-auth` keyring; MCP status/toggle via the Epic 7
`palmier-mcp` liveness + enable commands; "Get key"/Stripe URLs open via `tauri-plugin-opener`.
Reference: `Settings/{AccountPane,AgentPane}.swift`, `Account/*`, `Help/*`. Govern:
`settings-account-app.md`, FOUNDATION §6.13/§6.14/§6.15/§8.1, ruling #5.

**Dependencies:** Epic 1/8/9 (`palmier-auth` account+billing+keyring), Epic 7 (`palmier-mcp` liveness/
enable + `.mcpb`), Epic 1 (menu shortcut table for Shortcuts tab). **Parallel-safe?** Yes —
`src-ui/settings` subtree, disjoint files from E12-S10.

---

### E12-S12 — Telemetry + logging + crash handler (`palmier-telemetry`)

*As the build team, I want categorized tracing, Sentry, and a crash handler so the app produces useful
diagnostics while honoring the opt-out privacy toggle.*

**Acceptance criteria:**
- **Logging:** a `tracing` subscriber with categorized targets `app, editor, export, preview, mcp,
  generation, project, transcription, search` writes
  `%LOCALAPPDATA%\PalmierProWin\Logs\palmier.log` (Win) /
  `~/.local/state/palmier-pro/logs/palmier.log` (Linux), **daily-rotated, 7-day retention**; Info
  default, Debug with `--debug`; all levels mirror to stderr. (FOUNDATION §6.16; `settings-account-
  app.md` §Telemetry/logging.)
- **Sentry** (Rust SDK backend + Browser SDK frontend) starts **only if** the privacy pref is enabled
  **AND** the build-injected DSN is non-empty; options: `sendDefaultPii=false`, env `development`
  (debug)/`production` (release), `tracesSampleRate=0.1`, app-hang timeout 8.0 s,
  `attachStacktrace=true`, release name **`palmier-pro-win@<version>+<git_sha>`**. **Opt-out default**
  (`io.palmier.pro.telemetry.enabled` absent ⇒ ON — OQ-2/ruling #6), **launch-snapshotted**
  (`enabledForCurrentLaunch`; toggling requires restart). `warning` → Sentry breadcrumb;
  `error`/`fault` → Sentry capture-message. (FOUNDATION §6.16; `settings-account-app.md`.)
- **Crash handler:** a Rust panic hook (+ signal handling via `signal-hook`/`backtrace` or Sentry
  native) writes `crashes/<timestamp>.log` under the app data dir (**not** `~/Library/Logs`) and
  forwards to Sentry; writes are async-signal-safe (write/backtrace/fsync only). (FOUNDATION §6.16;
  `settings-account-app.md` §Crash-handler.)
- A unit/integration test asserts: DSN-empty ⇒ Sentry not initialized; pref-off-at-launch ⇒ Sentry not
  initialized even if DSN present; toggling the pref at runtime does **not** start/stop Sentry until
  restart (snapshot honored).

**Implementation context:** `palmier-telemetry` (owner) wired into `palmier-tauri` boot order (after
crash handler, before window). Sentry Rust SDK + `tracing-subscriber` + rolling file appender.
Reference: `Telemetry/Telemetry.swift`, `Utilities/Log.swift`. Govern: FOUNDATION §6.16,
`settings-account-app.md`, ruling #6, OQ-2.

**Dependencies:** Epic 1 (boot order, settings store for the pref snapshot). **Parallel-safe?** Yes —
`palmier-telemetry` is its own crate; the only shared touch is the boot-order call site in
`palmier-tauri` (coordinate with E12-S13/E12-S14).

---

### E12-S13 — Tauri Ed25519 updater (`palmier-update`)

*As a user, I want the app to check for and apply signed updates so I stay current, while dev builds
without a configured feed silently skip update checks.*

**Acceptance criteria:**
- The Tauri 2 updater pulls a per-platform **Ed25519-signed JSON manifest** (URL from build config,
  placeholder `https://updates.palmier.io/win/latest.json` — **OQ-9/§8.4**; **single `stable` channel**
  — OQ-1-update); the **Ed25519 public key** is in `tauri.conf.json`; an unsigned/mismatched manifest
  is rejected. (FOUNDATION §8.4; `settings-account-app.md` §Updater.)
- The updater is **silently disabled when no signed feed is configured** (no manifest URL / dev build)
  — **no spurious "update check failed" UI**, mirroring the reference Sparkle no-op without `SUFeedURL`.
  (`settings-account-app.md` §port-risk.)
- Menu **"Check for Updates…"** invokes a `check now` command; `update_available` /
  `update_version` (from the manifest's display version) are surfaced via a **Tauri event** that an
  update-badge view binds to. (`settings-account-app.md` §Updater + §UpdateBadgeView.)

**Implementation context:** `palmier-update` (Tauri updater plugin glue) + `palmier-tauri` (plugin
init, menu item, public key in `tauri.conf.json`) + `src-ui/app` (update badge bound to the event).
Reference: `App/Updater.swift`, `App/UpdateBadgeView.swift`. Govern: FOUNDATION §8.4,
`settings-account-app.md`, OQ-1-update/OQ-9.

**Dependencies:** Epic 1 (menu, `palmier-tauri` plugin wiring). **Parallel-safe?** Yes — own crate;
shares only the `palmier-tauri` plugin-init site (coordinate with E12-S12/E12-S14).

---

### E12-S14 — Packaging: `.msi` + `.AppImage` + `.deb` + `.rpm` bundler config

*As the build team, I want release artifacts for Windows and Linux so the app can be distributed and
auto-updated on every target platform.*

**Acceptance criteria:**
- `cargo tauri build` (or CI equivalent) produces a **`.msi`** on Windows and **`.AppImage` + `.deb` +
  `.rpm`** on Linux from the same `tauri.conf.json` bundle config; **Flatpak is OQ-8 (out of v1)**.
  (FOUNDATION §3/§8.4; FR-44.) Each platform ships its own updater manifest entry consistent with
  E12-S13's Ed25519 signing.
- Bundled resources are present in the packaged artifact: `Resources/Fonts/` (Featured fonts —
  FOUNDATION §6.6), the bundled Whisper `small.en` (FOUNDATION §6.9), and the `palmier-pro.mcpb`
  (Epic 7) — verified by a post-build artifact check.
- The FFmpeg build is the **LGPL-compatible** configuration (R-2/OQ-11: GPLv3 distribution accepted;
  confirm dependency licenses are GPL-compatible). App data / log / settings paths resolve to the
  Windows/Linux locations (no `~/Library`).
- A smoke test installs the `.msi` (and one Linux artifact) in CI and confirms the app boots to the
  Home window (ties to SM-1 cold-start on the packaged build, not just `cargo run`).

**Implementation context:** `palmier-tauri` (`tauri.conf.json` `bundle` config, resource inclusion,
signing keys) + CI lanes. Reference: `settings-account-app.md` §macOS-APIs-to-replace (CFBundle →
Tauri build config), FOUNDATION §8.4. Govern: FOUNDATION §3/§8.4, FR-44, R-2/OQ-11.

**Dependencies:** All crate epics (the binary must compile with every crate); E12-S13 (signing keys
for manifest consistency). **Parallel-safe?** No — `tauri.conf.json` + CI are shared release config;
sequence near the end, after E12-S12/E12-S13.

---

### E12-S15 — Memory ceiling (SM-10) profiling + remediation

*As the build team, I want the app to meet the SM-10 memory ceilings so it runs comfortably on the
reference hardware under a realistic project load.*

**Acceptance criteria:**
- On the **§10 reference HW** with a **200-clip project**, measured RSS is **< 800 MB idle** and
  **< 2.5 GB editor+preview** (SM-10, FOUNDATION §10). A CI/bench harness records both numbers on a
  representative `golden_project_keyframes`-scale fixture.
- If either ceiling is exceeded, remediation targets the known large consumers — the
  **`palmier-media` LRU FrameCache** sizing (FOUNDATION §6.5), thumbnail/waveform caches, and the
  SigLIP/Whisper model residency — without regressing SM-2 (preview FPS) or SM-C1 (fidelity); the
  chosen cache caps are recorded.
- The measurement runs on the **packaged build** (E12-S14), not a debug `cargo run`, so it reflects
  release memory behavior.

**Implementation context:** Cross-crate profiling (primarily `palmier-media` FrameCache,
`palmier-engine`, `palmier-search`, `palmier-transcribe`); a memory bench in the existing Criterion/CI
harness. Govern: FOUNDATION §10, PRD §7 SM-10, Glossary "Preview Composition" (FrameCache home =
`palmier-media`).

**Dependencies:** Epic 5 (preview + FrameCache), Epic 4 (caches), Epics 10/11 (models), E12-S14
(packaged build). **Parallel-safe?** No — it profiles the integrated app and may tune shared cache
crates; run late, alongside E12-S16.

---

### E12-S16 — M5 release gate: SM regression + §11.6 MCP suite + e2e

*As the release owner, I want the full SM regression set, the MCP compatibility suite, and the four
e2e workflows green so the M5 release bar (replacing "overall parity") is met before public launch.*

**Acceptance criteria:**
- The **complete SM regression set** runs green: **SM-1, SM-1b, SM-2, SM-3, SM-4, SM-5, SM-6, SM-7,
  SM-8, SM-9, SM-10, SM-11, SM-12, SM-13** and the counter-metrics **SM-C1, SM-C2, SM-C3** (no fidelity
  trade for FPS; exactly **30 tools + 2 resources**, no additions — ruling #1/SM-C2; loopback-only MCP
  validators intact — SM-C3). (PRD §12 M5 release gate.)
- The **§11.6 MCP compatibility suite** passes: Claude Desktop, Claude Code, Cursor, and Codex connect
  with **only the server URL changed** from the reference install (SM-8), running the reference test
  prompts with no protocol errors. (PRD §7 SM-8, §10 Epic 7.)
- The **four §11.3 e2e workflows** pass via `tauri-driver` + Playwright: hand-edit, agent-cut,
  generative-augment, B-roll-directed. (PRD §11.3 table.)
- **OQ-10 (branding)** and **OQ-1 (ProRes alpha)** are resolved/recorded before public-launch tagging;
  the social sidecar **frozen schema (OQ-7)** is confirmed landed (Epic 6 / M5 UJ-5 finalization).
  A release checklist + LOG.md line records the green run.

**Implementation context:** Test/CI orchestration only — this story authors no new product code; it
wires the existing per-epic golden/unit/integration/e2e suites into a single M5 release-gate run and
records results. Govern: PRD §12 (M5 release gate), §7 (all SMs), §11 (test strategy), §10 (every
epic's acceptance).

**Dependencies:** **Every other epic (1–11) and all prior Epic 12 stories.** This is the terminal
story of the entire build. **Parallel-safe?** No — it is the final integration gate; runs last.

---

## Coverage map (FR → stories)

| FR | Stories |
|---|---|
| FR-41 Inspector | E12-S1 (dB floor), E12-S2 (shell/tabs), E12-S3/S4 (components), E12-S5 (Video/Audio), E12-S6 (Text), E12-S7 (Details/AI-Edit), E12-S8 (keyframes), E12-S9 (toolbar) |
| FR-42 Settings & account | E12-S10 (General/Models/Storage), E12-S11 (Account/Agent/Help/Feedback) |
| FR-43 Telemetry/logging/updater | E12-S12 (telemetry/logging/crash), E12-S13 (updater) |
| FR-44 Packaging | E12-S14 (.msi/.AppImage/.deb/.rpm) |
| SM-10 Memory (cross-cutting NFR) | E12-S15 |
| M5 release gate (PRD §12) | E12-S16 |

## Parallelization notes

- **Inspector cluster (E12-S2..S9)** and **Settings cluster (E12-S10..S11)** are disjoint `src-ui`
  subtrees — fully parallel across the two clusters. Within the Inspector cluster, E12-S3/S4
  (components) feed S5/S6/S7; E12-S1 (model dB floor) gates S5/S8.
- **Backend cluster (E12-S12 telemetry, E12-S13 updater)** are independent crates, parallel with all
  UI work; they share only the `palmier-tauri` boot/plugin-init site.
- **E12-S14 (packaging), E12-S15 (memory), E12-S16 (release gate)** are terminal/serial — they
  integrate the whole workspace and run at the end of M5.
- Net: the epic fans out into **three parallel lanes** (Inspector UI / Settings UI / telemetry+updater
  backend), converging on the serial release tail (S14→S15→S16).
