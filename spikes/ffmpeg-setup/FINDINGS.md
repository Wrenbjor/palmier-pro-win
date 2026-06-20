# FFmpeg-on-Windows toolchain - setup & findings

Provisions the `ffmpeg-next` 7.x build/link toolchain so stories **E5-S2, E4-S3/S4/S5, E6-S5** can use FFmpeg for decode/encode. Done on branch `infra/ffmpeg-toolchain`.

**Status: VERIFIED.** `ffmpeg-next = "7"` builds, links, and runs on this box via `pwsh -File scripts/with-msvc.ps1 cargo run` from `spikes/ffmpeg-probe/`. Output:

```
ffmpeg-next OK: libavutil version 3876708 (59.39.100)
ffmpeg configuration: ... --enable-shared --disable-static --enable-version3 ... (LGPL, no --enable-gpl)
PROBE_SUCCESS
```

`libavutil 59.39.100` = FFmpeg 7.1 ABI. ffmpeg-next resolved to **7.1.0** (the "7" line caps at the 7.x FFmpeg ABI; 8.x is available but pinned out to match the 7.1 libs).

---

## What ffmpeg-sys-next needs (and how each was satisfied)

| Need | Why | Provided by |
|------|-----|-------------|
| FFmpeg **shared** dev libs: include/ headers + lib/*.lib import libs | compile + link the bindings | BtbN win64 **LGPL shared** 7.1 -> C:\ffmpeg |
| FFmpeg runtime bin/*.dll on PATH | the linked exe loads avcodec-61.dll etc. at startup | C:\ffmpeg\bin prepended to PATH |
| **libclang.dll** | ffmpeg-sys-next runs bindgen to generate the FFI from the headers | PyPI libclang wheel -> %LOCALAPPDATA%\Programs\libclang\bin |
| MSVC env (INCLUDE/LIB/link.exe) | this box's VS isn't vswhere-visible | existing scripts/with-msvc.ps1 (vcvars64) |

---

## Install steps (exact)

### 1. FFmpeg shared dev libraries
- **Source:** BtbN FFmpeg-Builds, `latest` release, asset `ffmpeg-n7.1-latest-win64-lgpl-shared-7.1.zip` (https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n7.1-latest-win64-lgpl-shared-7.1.zip).
- Downloaded (~62 MB), extracted, flattened the top-level folder into C:\ffmpeg so:
  - C:\ffmpeg\include\libav* (avcodec/avformat/avutil/avfilter/avdevice/swscale/swresample headers)
  - C:\ffmpeg\lib\*.lib (import libs: avcodec, avdevice, avfilter, avformat, avutil, swresample, swscale)
  - C:\ffmpeg\bin\*.dll (avcodec-61, avdevice-61, avfilter-10, avformat-61, avutil-59, swresample-5, swscale-8)
- C:\ root isn't writable for arbitrary files; download to $env:TEMP, then Move/Expand into C:\ffmpeg. ~147 MB total.

### 2. libclang (for bindgen)
**The winget LLVM.LLVM install FAILS non-interactively on this box** (exit 199 / 0x800704c7 "operation was canceled by the user"): the LLVM **NSIS installer has a requireAdministrator UAC manifest**, and the agent shell is non-elevated, so the elevation prompt is auto-denied. Running LLVM-22.1.8-win64.exe /S /D=<peruser> fails the same way at process launch (UAC blocks the launch itself, even for a per-user target).

**Workaround that needs no elevation:** the PyPI **libclang** wheel bundles a standalone libclang.dll. bindgen (via clang-sys + LIBCLANG_PATH) only needs that one DLL.

```
python -m pip download libclang --only-binary=:all: --no-deps -d <tmp>
# -> libclang-18.1.1-py2.py3-none-win_amd64.whl  (a zip)
# extract clang/native/libclang.dll  ->  %LOCALAPPDATA%\Programs\libclang\bin\libclang.dll
```

libclang.dll is **18.1.1** (84 MB). bindgen 0.70 works fine with libclang 18 against FFmpeg 7.1 headers (verified). libclang version is decoupled from the FFmpeg libs - it just parses C headers.

### 3. Env wiring (durable, in the build wrapper)
- New **scripts/ffmpeg-env.ps1** (idempotent) sets FFMPEG_DIR=C:\ffmpeg, LIBCLANG_PATH=%LOCALAPPDATA%\Programs\libclang\bin (falls back to system LLVM dirs if present), and prepends C:\ffmpeg\bin + LIBCLANG_PATH to PATH. Respects pre-set env (override-friendly); PATH entries de-duplicated.
- **scripts/with-msvc.ps1** now dot-sources ffmpeg-env.ps1 (if present) before the `cmd /c "call vcvars64.bat && <cmd>"` line. $env: vars set in the PowerShell process are inherited by the cmd child, so **every** Rust build through the wrapper gets the FFmpeg env automatically. with-msvc's existing behavior is otherwise unchanged; absent file = no-op.

---

## Env vars (values on this box)

| Var | Value |
|-----|-------|
| FFMPEG_DIR | C:\ffmpeg |
| LIBCLANG_PATH | C:\Users\Wren\AppData\Local\Programs\libclang\bin |
| PATH additions | C:\ffmpeg\bin ; C:\Users\Wren\AppData\Local\Programs\libclang\bin |

NOT set in the user/machine environment - they live only in scripts/ffmpeg-env.ps1, sourced by with-msvc.ps1. Any worker invoking Rust builds through the wrapper inherits them.

---

## Licensing (important for distribution)

- **FFmpeg build = LGPL v3** (C:\ffmpeg\LICENSE.txt is LGPL-3.0; the build is --enable-version3 **without** --enable-gpl, and GPL-only encoders **libx264/libx265/libxvid/libxavs2 are disabled** - confirmed in the runtime configuration string). LGPL permits linking from a proprietary/closed app **as long as FFmpeg is dynamically linked** (we use the shared DLLs, satisfying this) and the user can replace the FFmpeg DLLs. We ship the DLLs unmodified.
- **Contrast with the gyan.dev ffmpeg.exe already on PATH:** that's an --enable-gpl static full build (v8.1). Do **not** link the app against a GPL build - it would force the whole app to GPL. We deliberately chose the BtbN **LGPL shared** 7.1 build instead.
- **Implication for E5/E4/E6 + packaging:** keep FFmpeg **dynamically linked** (ship .dlls, don't statically embed), include FFmpeg attribution + the LGPL license text in the app's third-party notices, and keep the FFmpeg DLLs user-replaceable (they are, as separate files).
- **x264/x265 (H.264/HEVC) software encode is NOT in this LGPL build.** If a story needs H.264/HEVC *encode*, use FFmpeg hardware encoders (NVENC/QSV/AMF/MediaFoundation - available without GPL libs) rather than switching to a GPL build. H.264/HEVC *decode* is available without GPL. Revisit per-story if software x264/x265 encode is truly required.

## Runtime DLL bundling (for the app later)

The 7 FFmpeg DLLs in C:\ffmpeg\bin (avcodec-61, avformat-61, avutil-59, avfilter-10, avdevice-61, swresample-5, swscale-8 - plus any transitive deps BtbN bundles in that dir) must ship **next to the app .exe** (Tauri: bundle them as resources/externalBin so they land beside the binary; Windows resolves the app dir first in the DLL search). At dev time they're found via the C:\ffmpeg\bin PATH entry ffmpeg-env.ps1 adds. The packaging story (Tauri bundler config) must copy them from $FFMPEG_DIR\bin.

---

## Gotchas

- **winget LLVM is a dead end non-interactively** (UAC). Use the libclang wheel route above. If a future box has admin, interactive winget install LLVM.LLVM also works and ffmpeg-env.ps1 auto-detects C:\Program Files\LLVM\bin.
- **C:\ root not writable** for arbitrary files (Invoke-WebRequest to C:\ffmpeg-dl.zip denied), but creating C:\ffmpeg via Move-Item succeeded - download to $env:TEMP, then move/extract.
- **The pre-existing ffmpeg.exe on PATH (winget gyan.dev, v8 GPL static) is a red herring** - no dev libs, wrong version/license. FFMPEG_DIR points ffmpeg-sys-next at C:\ffmpeg explicitly, so PATH order vs that exe doesn't matter for the build.
- **ffmpeg-next 7 vs 8:** cargo notes 8.1.0 available, but "7" pins to the 7.x ABI matching the FFmpeg 7.1 libs. If we move to FFmpeg 8 libs, bump both together.
- Probe crate spikes/ffmpeg-probe/ declares an **empty [workspace]** table so it is NOT a member of the 17-crate root workspace (same isolation pattern as spikes/s1-wgpu-webview).

---

## Files changed by this task
- scripts/ffmpeg-env.ps1 (new) - the sourceable env.
- scripts/with-msvc.ps1 (edit) - dot-source ffmpeg-env.ps1.
- spikes/ffmpeg-probe/{Cargo.toml,src/main.rs} (new) - throwaway verification crate.
- spikes/ffmpeg-setup/FINDINGS.md (this file).

Machine-global installs (NOT committed): C:\ffmpeg (FFmpeg 7.1 LGPL shared), %LOCALAPPDATA%\Programs\libclang\bin\libclang.dll (libclang 18.1.1).
