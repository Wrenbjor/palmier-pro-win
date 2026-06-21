// ui-smoke.spec.ts — Layer A: mock-Tauri UI smoke (headless Chromium against Vite).
//
// Proves each top-level surface RENDERS its real content given realistic backend
// payloads (seeded via tauri-mock.ts). This catches "white screen of death" / crashed
// surface regressions without a human clicking and without the native WebView2.
//
// Surfaces & assertions:
//   Home (#/home)            -> project browser: title + "New Project" + a recent tile
//   Project (#/project/<id>) -> timeline <canvas> + agent panel (NOT blank)
//   Settings (#/settings)    -> settings nav with the 5 tabs
//
// A screenshot of every surface is captured as a build artifact (e2e/artifacts/).
//
// Run: cd e2e && npx playwright test  (Vite auto-starts via webServer in the config)

import { test, expect } from "@playwright/test";
import { tauriMockScript, type SurfaceLabel } from "./tauri-mock";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ARTIFACTS = join(dirname(fileURLToPath(import.meta.url)), "..", "artifacts");
mkdirSync(ARTIFACTS, { recursive: true });

// Install the seeded Tauri bridge before the app bundle loads, then navigate to the
// hash route for that surface. `label` drives route.ts (it prefers the window label).
async function gotoSurface(page: import("@playwright/test").Page, label: SurfaceLabel, hash: string) {
  await page.addInitScript(tauriMockScript(label));
  // Fail loudly on an uncaught page error — a crashed surface is a smoke failure.
  const pageErrors: string[] = [];
  page.on("pageerror", (e) => pageErrors.push(String(e)));
  await page.goto(`/#/${hash}`);
  // give React a tick to mount + resolve the seeded invokes
  await page.waitForLoadState("networkidle");
  return pageErrors;
}

test("Home renders the project browser", async ({ page }) => {
  const errors = await gotoSurface(page, "home", "home");

  // Title + primary action prove the project-browser shell mounted.
  await expect(page.getByRole("heading", { name: "Palmier Pro" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Recent Projects" })).toBeVisible();
  await expect(page.getByText("New Project").first()).toBeVisible();
  // A seeded recent project tile is shown (proves list_recent data rendered).
  await expect(page.getByText("Demo Reel")).toBeVisible();

  await page.screenshot({ path: join(ARTIFACTS, "home.png"), fullPage: true });
  expect(errors, `page errors: ${errors.join("\n")}`).toHaveLength(0);
});

test("Project renders the timeline canvas + agent panel", async ({ page }) => {
  const errors = await gotoSurface(page, "project/proj-fixture-1", "project/proj-fixture-1");

  // The editor mount carries the project id (proves the Project surface mounted).
  await expect(page.locator("[data-project-id='proj-fixture-1']")).toBeVisible();
  // Timeline is drawn to a <canvas>; at least one must be present (not a blank page).
  await expect(page.locator("canvas").first()).toBeVisible();
  // The agent dock renders its header + collapse control.
  await expect(page.getByText("Agent", { exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Collapse agent panel" })).toBeVisible();

  await page.screenshot({ path: join(ARTIFACTS, "project.png"), fullPage: true });
  expect(errors, `page errors: ${errors.join("\n")}`).toHaveLength(0);
});

test("Settings renders the tabs", async ({ page }) => {
  const errors = await gotoSurface(page, "settings", "settings");

  // The 5-tab settings nav (Account hidden only when backend misconfigured; our mock
  // reports configured + signed-in, so Account is present).
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Models" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Storage" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Account" })).toBeVisible();

  await page.screenshot({ path: join(ARTIFACTS, "settings.png"), fullPage: true });
  expect(errors, `page errors: ${errors.join("\n")}`).toHaveLength(0);
});
