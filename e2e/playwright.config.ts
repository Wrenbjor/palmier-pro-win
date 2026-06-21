import { defineConfig, devices } from "@playwright/test";

// UI smoke config (Layer A): drives the Vite-served React UI in headless Chromium
// with a mocked Tauri bridge (see tauri-mock.ts). This does NOT touch the native
// WebView2 — it validates that each surface (Home / Project / Settings) renders given
// realistic backend payloads. Real-webview e2e (Layer B) lives under real-webview/.
//
// The webServer block boots Vite (src-ui) automatically and reuses an already-running
// instance, so `npx playwright test` is a single self-contained command.

const VITE_URL = process.env.PALMIER_UI_URL ?? "http://localhost:5173";

export default defineConfig({
  testDir: "./tests",
  // Real-webview specs are opt-in (run via their own runner), keep them out of the
  // default mock-based run.
  testIgnore: ["**/real-webview/**"],
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"], ["html", { outputFolder: "playwright-report", open: "never" }]],
  outputDir: "test-results",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: VITE_URL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    // Start Vite from src-ui via cmd so PATHEXT resolves pnpm -> pnpm.cmd (calling the
    // pnpm.ps1 shim directly pops a Windows file-association dialog).
    command: "cmd /c pnpm --dir ../src-ui dev",
    url: VITE_URL,
    reuseExistingServer: true,
    timeout: 120_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
