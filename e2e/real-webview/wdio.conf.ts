// wdio.conf.ts — WebdriverIO config for real-webview e2e via tauri-driver (Layer B).
//
// OPTIONAL / DEFERRED: this is scaffolding. It has NOT been executed in CI yet because
// tauri-driver + msedgedriver are not provisioned on the build box (see README.md).
// Once those are installed, `pnpm test` here drives the real WebView2 window.
//
// How it works: tauri-driver is a WebDriver proxy. We spawn it as a child process; it
// in turn launches the app binary under WebDriver and forwards to msedgedriver (the
// Edge WebView2 driver). WebdriverIO connects to tauri-driver on 127.0.0.1:4444.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";

const APP_BIN =
  process.env.PALMIER_APP_BIN ??
  resolve(__dirname, "..", "..", "target", "debug", "palmier-tauri.exe");

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
    },
  ],
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  reporters: ["spec"],
  logLevel: "warn",

  // Boot tauri-driver before the session, kill it after.
  onPrepare: () => {
    // Verify the proxy exists; fail loudly with the install hint rather than hanging.
    const probe = spawnSync("tauri-driver", ["--help"], { shell: true });
    if (probe.status !== 0) {
      throw new Error(
        "tauri-driver not found. Install it: `cargo install tauri-driver --locked` " +
          "and ensure a matching msedgedriver is on PATH. See e2e/real-webview/README.md.",
      );
    }
    tauriDriver = spawn("tauri-driver", [], { stdio: [null, process.stdout, process.stderr], shell: true });
  },
  onComplete: () => {
    tauriDriver?.kill();
  },
};
