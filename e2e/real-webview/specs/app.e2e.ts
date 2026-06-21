// app.e2e.ts — real-webview smoke (Layer B, OPTIONAL/DEFERRED).
//
// Drives the actual WebView2 window via tauri-driver. Asserts the same three surfaces
// as the Layer-A mock suite, but against the real Rust invoke bridge and real boot.
//
// Windows window routing is owned by Rust (each window gets a label + #/<label> hash),
// so the app opens Home first; we drive between surfaces via the app's own UI/menu
// rather than rewriting location.hash (which the native side controls).
//
// NOT YET RUN — see e2e/real-webview/README.md. Selectors mirror the mock suite.

describe("Palmier Pro — real webview smoke", () => {
  it("Home shows the project browser", async () => {
    // The first window is Home; wait for its title heading to render.
    const title = await $("h1*=Palmier Pro");
    await title.waitForDisplayed({ timeout: 60_000 });
    await expect(title).toBeDisplayed();

    const recent = await $("h2*=Recent Projects");
    await expect(recent).toBeDisplayed();
  });

  it("Settings window renders its tabs", async () => {
    // Open Settings via the app (its own button/menu); the Rust side opens a new
    // labelled window. tauri-driver follows the active webview.
    const settingsBtn = await $("button*=Settings");
    if (await settingsBtn.isExisting()) {
      await settingsBtn.click();
    }
    const general = await $("button*=General");
    await general.waitForDisplayed({ timeout: 30_000 });
    await expect(general).toBeDisplayed();
  });

  it("Project window renders the timeline canvas + agent panel", async () => {
    // A project must exist to open the editor; create one, then assert the editor
    // chrome. This is the most environment-sensitive case (wgpu preview init) — assert
    // on DOM, never pixels.
    const newProject = await $("button*=New Project");
    if (await newProject.isExisting()) {
      await newProject.click();
    }
    const canvas = await $("canvas");
    await canvas.waitForExist({ timeout: 60_000 });
    await expect(canvas).toBeExisting();

    const agent = await $("span*=Agent");
    await expect(agent).toBeDisplayed();
  });
});
