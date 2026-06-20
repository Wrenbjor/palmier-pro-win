---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# design-tokens — reference port notes

## Purpose
Authoritative extraction of every design token from the macOS reference `UI/AppTheme.swift` (plus
hardcoded values in sibling UI files), verified against FOUNDATION SS9. This is the source the
frontend turns into `tokens.json` / CSS variables. Where the reference and FOUNDATION SS9 disagree,
**both are recorded and flagged**; the reference Swift source is ground truth for parity.

## Key types & files (cite paths under Sources/PalmierPro/UI/...)
- `UI/AppTheme.swift` — the entire token system; nested `enum`s (Background, Border, BorderWidth,
  Accent, Glass, Status, Text, Opacity, TrackColor, Radius, Spacing, FontSize, FontWeight,
  Tracking, IconSize, ComponentSize, Window, Caption, GenerationPanel, MediaPanel) + `ShadowStyle`
  struct, `Shadow`, `Anim`, two module-level `LinearGradient`s (`aiGradient`, `aiGradientDark`).
- `UI/CapsuleButton.swift` — consumes tokens; defines per-variant padding/font selection logic.
- `UI/HoverHighlight.swift` — hover-fill opacity state machine (uses `Opacity.muted/soft/faint`).
- `UI/GeneratingOverlay.swift` — shimmer gradient (white@0.42, location 0.48) + 1.35s linear loop;
  progress bar 45s easeOut to 0.9; bar sizes 160x4 (preview) / 60x3 (thumbnail).
- `Utilities/Constants.swift` — `enum Layout` (panelHeaderHeight=28, toolbarHeight=38, panelGap=5,
  trackHeight=50, rulerHeight=24, etc.) — layout constants NOT in AppTheme but referenced by it.

NSColor channels are `n/255` (sRGB). `Color(white: x)` is grayscale x in 0..1. SwiftUI default
color space differs from NSColor; treat `Color(red:green:blue:)` literals as sRGB for the port.

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### Colors (hex computed from Swift literals)
| Token | Swift source | Hex | Alpha |
|---|---|---|---|
| bg-base | NSColor(10,10,10) | #0A0A0A | 1 |
| bg-surface | NSColor(22,22,22) | #161616 | 1 |
| bg-raised | NSColor(30,30,30) | #1E1E1E | 1 |
| bg-prominent | NSColor(44,44,44) | #2C2C2C | 1 |
| bg-preview | .black | #000000 | 1 |
| bg-placeholder | alias of raised | #1E1E1E | 1 |
| border-primary | white | #FFFFFF | 0.16 |
| border-subtle | white | #FFFFFF | 0.12 |
| border-divider | white | #FFFFFF | 0.44 |
| accent-primary | (0.961,0.937,0.894) | #F5EFE4 | 1 |
| accent-timecode | (0.95,0.6,0.2) | **#F29933** | 1 |
| accent-spotlight | (1.0,0.27,0.27) | #FF4545 | 1 |
| status-error | (0xE5,0x4F,0x4F) | #E54F4F | 1 |
| text-primary | white | #FFFFFF | 1.0 |
| text-secondary | white | #FFFFFF | 0.80 |
| text-tertiary | white | #FFFFFF | 0.62 |
| text-muted | white | #FFFFFF | 0.34 |
| glass-primary-tint | accent-primary | #F5EFE4 | 0.05 |
| track-video | (0x00,0x91,0xC2) | #0091C2 | 1 |
| track-audio | (0x58,0xA8,0x22) | #58A822 | 1 |
| track-image | (0xB7,0x2D,0xD2) | #B72DD2 | 1 |
| track-text | (0xB7,0x2D,0xD2) | #B72DD2 | 1 |
| track-lottie | (0xE0,0xA8,0x00) | #E0A800 | 1 |

### Gradients (FOUNDATION SS9 omits all of these — must add to tokens.json)
- `accent-spotlight-gradient` — LinearGradient topLeading→bottomTrailing, stops:
  `#FF574D` (1.0,0.34,0.30) → `#F2264700`? no — `#F22647` (0.95,0.15,0.28) → `#FF7A38` (1.0,0.48,0.22).
- `ai-gradient` (silver shimmer) — topLeading→bottomTrailing, stops by location:
  white@0.00=#FFFFFF, 0.45=#C7C7C7 (white 0.78), 0.55=#999999 (white 0.60), 1.00=#FFFFFF.
- `ai-gradient-dark` — top→bottom: 0.00=#1C1C1C (white 0.11), 1.00=#0F0F0F (white 0.06).
- `shimmer-gradient` (GeneratingOverlay) — top→bottom: clear@0, white@0.42 at 0.48, clear@1;
  width = 45% of host, rotated 18°, swept x=-1→2 over 1.35s linear repeatForever, blendMode `.screen`.

### Scale tokens (px unless noted)
- Spacing: xxs=2 xs=4 sm=6 smMd=8 md=10 mdLg=12 lg=14 lgXl=16 xl=20 xlXxl=24 xxl=28
- Radius: xs=3 xsSm=4 sm=6 md=10 mdLg=12 lg=14 xl=20; **concentric(outer,padding)=max(outer-padding,0)**
- BorderWidth: hairline=0.5 thin=1 medium=1.5 thick=2
- FontSize: micro=8 xxs=9 xs=10 sm=11 smMd=12 md=13 mdLg=14 lg=15 xl=18 title1=22 title2=28 display=36
- FontWeight: light regular medium semibold bold (map to CSS 300/400/500/600/700)
- Tracking (letter-spacing): tight=-0.5 normal=0 wide=1.5 (Swift `.tracking()` is points, ≈px)
- IconSize: xxs=12 xs=14 sm=18 smMd=20 md=22 mdLg=24 lg=26 lgXl=28 xl=30
- Opacity: opaque=1 subtle=0.04 hint=0.06 faint=0.08 soft=0.10 muted=0.15 moderate=0.25 medium=0.35 strong=0.55 prominent=0.80
- Shadow: sm `0 0.5px 1px rgba(0,0,0,0.30)`; md `0 2px 4px rgba(0,0,0,0.30)`; lg `0 8px 24px rgba(0,0,0,0.25)` (x,y,radius reordered to CSS `x y blur color`)
- Anim: hover=0.15s, transition=0.20s (easeOut is the default curve used in CapsuleButton/HoverHighlight)

### Component / layout sizing (NOT in FOUNDATION SS9 — needed for parity, add to tokens.json)
- ComponentSize: captionPreviewMaxHeight=150, captionPreviewMaxTextWidthRatio=0.9,
  toolImagePreviewMaxHeight=50, projectCardWidth=150, projectCardHeight=120
- Window: homeDefault 1200x1200, homeMin 760x480, projectDefault 1600x1000, projectMin 960x600,
  projectTitlebarTrailingWidth=280
- Caption: defaultFontSize=48, min=12, max=300, minPosition=0, maxPosition=1, centerSnapValue=0.5,
  centerSnapThreshold=0.02, defaultCenterY=0.9, defaultCenter=(0.5,0.9), minDisplayDuration=0.7
- GenerationPanel: mediaAreaMinHeight=120, loadingHeight=180, promptMinHeight=40,
  referenceTile 80x56
- MediaPanel: tabRailWidth = IconSize.lg(26) + Spacing.sm(6)*2 = **38**; contextRowHeight = IconSize.md = **22**
- Layout (Constants.swift): panelHeaderHeight=28, toolbarHeight=38, panelGap=5, trackHeight=50,
  rulerHeight=24, mediaPanel 500/280, inspector 260/150, agentPanel 240–640, timeline 100–700

### Component behaviors (derived token usage — replicate exactly)
- CapsuleButton: small → fontSize=xs(10), hPad=smMd(8), vPad=xs(4); regular → smMd(12)/lgXl(16)/smMd(8);
  prominent fg=bg-base, bg=accent-primary (or override fill); secondary fg=text-secondary, bg=bg-prominent;
  hover overlay white@faint(0.08); pressed opacity=strong(0.55); continuous-corner capsule.
- HoverHighlight fill state machine: (active,hover) → (T,T)=white@muted(0.15), (T,F)=white@soft(0.10),
  (F,T)=white@faint(0.08), (F,F)=clear. Default cornerRadius=Radius.sm(6).
- panelHeaderBar: height=28, bg=raised, 1px bottom border = border-primary.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `NSColor(red:green:blue:alpha:)` / `Color(red:green:blue:)` → CSS sRGB hex + `rgba()` for alpha
  variants. Generate at build time from `tokens.json`.
- `NSColor.white.withAlphaComponent(a)` → `rgba(255,255,255,a)`.
- `Color(white:)` grayscale → equivalent hex (computed above).
- `LinearGradient(stops:startPoint:endPoint:)` → CSS `linear-gradient(deg, color pos%, ...)`.
  topLeading→bottomTrailing ≈ `135deg`; top→bottom = `180deg`.
- `Font.Weight` (.light…bold) → CSS `font-weight` 300–700.
- SwiftUI `.shadow(color:radius:x:y:)` (radius = blur) → CSS `box-shadow: x y blur color` /
  `drop-shadow`. No spread in source (omit / 0).
- `.tracking()` (points) → CSS `letter-spacing` (px; value is small, treat as px not em).
- `Animation.easeOut(duration:)` → CSS `transition: … ease-out <dur>s` or framer-motion easeOut.
- `NSSize` window constants → Tauri `tauri.conf.json` window `width/height/minWidth/minHeight`.
- `.blendMode(.screen)` (shimmer) → CSS `mix-blend-mode: screen`; `.continuous` corners → CSS
  `border-radius` (no squircle; acceptable visual delta).
- System font (`.system(size:weight:)`) → FOUNDATION SS9.3 `--platform-font-stack` (Inter Variable
  primary; Segoe UI Variable on Windows, system-ui/Cantarell on Linux). **No font family in
  AppTheme — it relies on the OS default; the port must inject the platform stack.**

## Mapping to FOUNDATION crates (src-ui/design)
- Emit `src-ui/design/tokens.json` with groups: color, gradient, spacing, radius, borderWidth,
  fontSize, fontWeight, tracking, iconSize, opacity, shadow, anim, component, window, caption.
- Build step generates `:root { --token: value }` CSS + a typed TS export. Keep token names
  kebab-cased matching FOUNDATION SS9 (`--bg-base`, etc.) so SS9's table stays the contract.
- Add SS9.3 platform tokens (`--window-controls-padding`, `--scrollbar-width`,
  `--platform-font-stack`) — these have NO reference equivalent (Windows/Linux-only), set per-OS.

## Port risks & gotchas
- **DISCREPANCY (timecode):** FOUNDATION SS9 lists `--accent-timecode = #F2994A`. The Swift source
  `NSColor(0.95,0.6,0.2)` computes to **#F29933** (0.2×255=51=0x33, not 0x4A). Use **#F29933** for
  parity; FOUNDATION's hex is a rounding error — flag to update SS9.
- **DISCREPANCY (accent-primary):** SS9 lists `#F5F0E4`. Swift (0.961,0.937,0.894) → G=0.937×255=239=0xEF,
  so **#F5EFE4** (SS9 shows #F5F0E4 = G 0xF0). 1-bit delta; prefer the Swift value #F5EFE4.
- **OMISSIONS in SS9:** spotlight 3-stop gradient, `ai-gradient`, `ai-gradient-dark`,
  `shimmer-gradient`, `--glass-primary-tint` (#F5EFE4 @0.05), `--bg-placeholder`, and ALL
  component/window/caption/layout sizing tokens. tokens.json must carry these even though SS9 omits
  them, or component parity breaks.
- Alpha-over-dark compositing: many tokens are white@low-alpha over near-black bg. Ensure the
  webview composites in sRGB (not linear) to match AppKit, or borders/hover will look wrong.
- `concentric(outer,padding)` radius helper is an algorithm, not a constant — port as a function.
- Tracking unit: Swift `.tracking` is in points and applied to specific Text views, not global;
  do not blanket-apply `wide=1.5` letter-spacing — it is opt-in per label.

## Open questions
- Should `track-text` truly equal `track-image` (#B72DD2)? Reference duplicates it; confirm intended
  vs. a copy-paste bug before locking SS9.
- Continuous (squircle) corners are unavailable in CSS — is the slight visual delta acceptable, or
  do we need an SVG/clip-path squircle for cards/capsules?
- Shadow `radius` maps to CSS blur with no spread; confirm no spread is the intended look on
  Windows/Linux compositors (AppKit shadow falloff differs from CSS box-shadow).
- FOUNDATION SS9 should be updated for the two hex discrepancies — who owns that edit?
