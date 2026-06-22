// app.e2e.ts — real-webview smoke (Layer B).
//
// Drives the ACTUAL WebView2 window the app ships, via tauri-driver. This is the only
// layer that exercises the real Rust `invoke` bridge end-to-end: a real DOM click in
// the real webview -> the real `@tauri-apps/api` invoke -> the real Rust command ->
// observable app/state change. The Layer-A mock suite stubs that bridge; this does not.
//
// IMPORTANT timing note (WebView2): when tauri-driver hands the session to msedgedriver,
// the WebView2 starts on `about:blank` and only navigates to `http://tauri.localhost/`
// (the frozen frontend) a beat later. Querying elements before that navigation finds an
// empty document. So every test first waits for the URL to become tauri.localhost.

import { strict as assert } from "node:assert";

const MCP_URL = "http://127.0.0.1:19789/mcp";

/** Wait until the WebView2 has navigated from about:blank to the app's frozen content
 *  (http://tauri.localhost/). Returns once the real app document is the active context. */
/** Is the session currently bound to the real app document (not about:blank)? */
async function onAppDocument(): Promise<boolean> {
  const url = await browser.getUrl().catch(() => "");
  if (url.includes("tauri.localhost")) return true;
  const src = await browser.getPageSource().catch(() => "");
  return src.includes('id="root"') && src.length > 200;
}

// tauri-driver's WebView2 session attach is racy on Tauri 2 (wry 0.55): the session
// sometimes lands on the transient `about:blank` host webview that never navigates,
// instead of the app's content webview (http://tauri.localhost/). When that happens the
// only reliable recovery is to tear the session down and start a fresh one — a new
// attach usually lands on the real document. We retry a bounded number of times.
async function ensureAppDocument(maxReloads = 6): Promise<void> {
  for (let attempt = 0; attempt <= maxReloads; attempt++) {
    // Give this session up to ~12s to navigate to the app document.
    const ok = await browser
      .waitUntil(onAppDocument, { timeout: 12_000, interval: 500 })
      .then(() => true)
      .catch(() => false);
    if (ok) return;
    if (attempt < maxReloads) {
      // Fresh session = fresh attach attempt (kills + relaunches the app under WebDriver).
      await browser.reloadSession().catch(() => {});
    }
  }
  throw new Error(
    `WebView2 session never attached to the app document (http://tauri.localhost/) after ${maxReloads + 1} attach attempts — ` +
      "tauri-driver kept binding to the transient about:blank host webview.",
  );
}

/** Poll the live MCP JSON-RPC server (shares ONE EditorState with the UI). Proves the
 *  REAL Rust backend the webview talks to actually booted — not a mock. */
async function mcpPing(timeoutMs = 60_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  const body = JSON.stringify({ jsonrpc: "2.0", id: 0, method: "ping" });
  while (Date.now() < deadline) {
    try {
      const res = await fetch(MCP_URL, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
        body,
      });
      if (res.ok) return true;
    } catch {
      /* server not up yet */
    }
    await new Promise((r) => setTimeout(r, 700));
  }
  return false;
}

describe("Palmier Pro — real webview smoke (Layer B)", () => {
  before(async () => {
    // Block until the session is bound to the real app document (retrying the attach
    // through reloadSession if tauri-driver lands on the about:blank host).
    await ensureAppDocument();
  });

  it("the real WebView2 window boots Home and renders the project browser", async () => {
    // PROVES: the shipping binary launches under WebDriver, the frozen frontend bundle
    // loads in the real WebView2 (http://tauri.localhost/), route.ts resolves the "home"
    // window label, and the real invokes (get_settings / list_recent / list_samples)
    // resolve without crashing the surface (no white-screen-of-death).
    const title = await $("h1*=Palmier Pro");
    await title.waitForDisplayed({ timeout: 30_000 });
    await expect(title).toBeDisplayed();

    const recent = await $("h2*=Recent Projects");
    await expect(recent).toBeDisplayed();

    // The three primary actions render (their click handlers ARE the real bridge).
    await expect($("button*=New Project")).toBeDisplayed();
    await expect($("button*=Open")).toBeDisplayed();
    await expect($("button*=Settings")).toBeDisplayed();
  });

  it("the live MCP backend (shared EditorState) is up — proves the real Rust core booted", async () => {
    // PROVES: boot step 6 started the loopback MCP server bound to 127.0.0.1:19789, i.e.
    // the SAME Arc<ToolExecutor>/EditorState the in-app UI dispatches edits through is
    // live. This is the real backend behind the webview, observed out-of-band.
    const up = await mcpPing();
    assert.equal(up, true, "MCP server at 127.0.0.1:19789 never answered ping within 60s");
  });

  it("clicking Settings fires a REAL invoke that opens a new native window", async () => {
    // PROVES THE BRIDGE: a real click in the real webview -> @tauri-apps/api invoke
    // ("open_settings") -> Rust window::open_or_focus -> a new labelled WebView2 window.
    // We observe the side effect that ONLY the real Rust path can produce: a second
    // top-level window handle appears. The mock suite cannot do this (no real windows).
    const before = await browser.getWindowHandles();

    const settingsBtn = await $("button*=Settings");
    await settingsBtn.waitForClickable({ timeout: 30_000 });
    await settingsBtn.click();

    // Wait for Rust to create the Settings window (a new WebDriver window handle).
    await browser.waitUntil(
      async () => (await browser.getWindowHandles()).length > before.length,
      {
        timeout: 30_000,
        timeoutMsg: "no new window appeared after clicking Settings — invoke bridge did not open the Settings window",
      },
    );

    const after = await browser.getWindowHandles();
    assert.ok(
      after.length > before.length,
      `expected a new window after Settings click (had ${before.length}, now ${after.length})`,
    );

    // Switch into the new window and confirm it is the Settings surface, not a blank
    // window — proves the new window also loaded the bundle and resolved its label.
    const created = after.find((h) => !before.includes(h));
    if (created) {
      await browser.switchToWindow(created);
      // The new window also starts on about:blank; wait for its content too.
      await browser.waitUntil(
        async () => (await browser.getUrl().catch(() => "")).includes("tauri.localhost"),
        { timeout: 30_000, timeoutMsg: "Settings window never loaded tauri.localhost content" },
      );
      const general = await $("button*=General");
      await general.waitForDisplayed({ timeout: 30_000 });
      await expect(general).toBeDisplayed();
      await expect($("button*=Models")).toBeDisplayed();
      // Switch back to Home so teardown is clean.
      await browser.switchToWindow(before[0]);
    }
  });
});
