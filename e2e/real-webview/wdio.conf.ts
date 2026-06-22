// wdio.conf.ts — WebdriverIO config for real-webview e2e via tauri-driver (Layer B).
//
// Drives the REAL WebView2 window the app ships. tauri-driver is Tauri's official
// WebDriver proxy: we spawn it as a child process; it launches the app binary under
// WebDriver and forwards to msedgedriver (the Edge WebView2 driver). WebdriverIO
// connects to tauri-driver on 127.0.0.1:4444.
//
// Provisioned on this box (June 2026):
//   tauri-driver  v2.0.6   (~/.cargo/bin/tauri-driver.exe)
//   msedgedriver  149.0.4022.80  (e2e/real-webview/.driver/, matches Edge/WebView2)
//   app binary    target/debug/palmier-tauri.exe (frozen frontendDist, --no-default-features)

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";

// The testable app binary. tauri build/cargo build embeds the frozen frontendDist
// (src-ui/dist), so the binary is self-contained — no Vite dev server needed.
const APP_BIN =
  process.env.PALMIER_APP_BIN ??
  resolve(__dirname, "..", "..", "target", "debug", "palmier-tauri.exe");

// tauri-driver proxy (Rust). Installed to ~/.cargo/bin by `cargo install tauri-driver`.
const TAURI_DRIVER =
  process.env.TAURI_DRIVER_BIN ??
  resolve(homedir(), ".cargo", "bin", "tauri-driver.exe");

// msedgedriver MUST match the installed Edge/WebView2 major version. We vendor an
// exact-match copy under .driver/ so the suite is hermetic and doesn't depend on PATH.
const NATIVE_DRIVER =
  process.env.MSEDGEDRIVER_BIN ??
  resolve(__dirname, ".driver", "msedgedriver.exe");

const DRIVER_PORT = 4444;

let tauriDriver: ChildProcess | undefined;

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./specs/**/*.e2e.ts"],
  // tauri-driver supports a single app session at a time — keep it serial.
  maxInstances: 1,
  capabilities: [
    {
      // tauri-driver reads these to launch the native app under WebDriver.
      // @ts-expect-error — tauri:options is a tauri-driver extension capability.
      "tauri:options": { application: APP_BIN },
      browserName: "wry",
      // WebdriverIO v9 negotiates WebDriver BiDi by default (webSocketUrl: true). With
      // WebView2 + tauri-driver that makes msedgedriver bind the session to the wrong
      // CoreWebView2 target (the transient about:blank host) — the app's content webview
      // (http://tauri.localhost/) never becomes the active document. Forcing CLASSIC
      // WebDriver makes the session land on the real app document. (A raw classic
      // /session POST lands correctly; the BiDi-enabled WDIO session did not.)
      "wdio:enforceWebDriverClassic": true,
    },
  ],
  hostname: "127.0.0.1",
  port: DRIVER_PORT,
  path: "/",
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 180_000 },
  reporters: ["spec"],
  logLevel: "warn",
  // Cold app boot (settings, MCP bind, wgpu preview init) + WebView2 attach is slow.
  connectionRetryTimeout: 180_000,
  connectionRetryCount: 1,
  waitforTimeout: 30_000,

  // Boot tauri-driver before the session, kill it after.
  onPrepare: async () => {
    if (!existsSync(TAURI_DRIVER)) {
      throw new Error(
        `tauri-driver not found at ${TAURI_DRIVER}. Install: ` +
          "`pwsh -File scripts/with-msvc.ps1 cargo install tauri-driver --locked`.",
      );
    }
    if (!existsSync(NATIVE_DRIVER)) {
      throw new Error(
        `msedgedriver not found at ${NATIVE_DRIVER}. Download the build matching the ` +
          "installed Edge/WebView2 version into e2e/real-webview/.driver/ " +
          "(see e2e/real-webview/README.md).",
      );
    }
    if (!existsSync(APP_BIN)) {
      throw new Error(
        `app binary not found at ${APP_BIN}. Build it: ` +
          "`pwsh -File scripts/with-msvc.ps1 cargo build --package palmier-tauri --no-default-features`.",
      );
    }
    // Sanity: the native driver must match Edge. Log its version so a future skew is
    // obvious in the run output.
    const ver = spawnSync(NATIVE_DRIVER, ["--version"], { encoding: "utf8" });
    console.log(`[wdio] msedgedriver: ${(ver.stdout || ver.stderr || "").trim()}`);

    tauriDriver = spawn(TAURI_DRIVER, ["--port", String(DRIVER_PORT), "--native-driver", NATIVE_DRIVER], {
      stdio: [null, process.stdout, process.stderr],
    });
    tauriDriver.on("error", (e) => {
      throw new Error(`failed to spawn tauri-driver: ${e.message}`);
    });
    // Give tauri-driver AND the native msedgedriver it shells out to time to fully bind.
    // tauri-driver's native-port handshake is racy: creating the WebDriver session too
    // early yields a dead/blank session (observed: "response ended prematurely", or the
    // session attaches to the transient about:blank host instead of tauri.localhost).
    // A few seconds of settle makes the first session land on the real app document.
    await new Promise((r) => setTimeout(r, 4_000));
  },
  onComplete: () => {
    tauriDriver?.kill();
  },
};
