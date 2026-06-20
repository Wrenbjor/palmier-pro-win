// Pure-logic parity checks for the agent panel (E8-S8 acceptance).
//
// As with the editor/media-panel, there is no test runner wired into `src-ui` yet
// (adding one would touch the shared package.json/lockfile owned by another worker). So
// these checks are a framework-free, type-checked module: covered by `tsc --noEmit`,
// and runnable directly (see `_run-parity.mts`). `runAgentParityChecks()` returns the
// failures so a future vitest can `expect(runAgentParityChecks()).toEqual([])`.
//
// Golden values verified against `docs/reference/agent-panel.md`:
//   availableModels / effectiveModel / canStream (lines 51-54, ruling #20),
//   send gating (line 232), mention tokens + referenced/pruned (lines 98, 133-135),
//   @mention autocomplete query/apply, title derivation (line 156), text-delta append
//   (line 106), empty-turn detection (lines 109-110), 7 starter prompts (line 30).

import {
  appendTextDelta,
  applyMentionPick,
  attachMentionToken,
  availableModels,
  canSend,
  canStream,
  deriveTitle,
  disambiguateMentions,
  draftContainsToken,
  effectiveModel,
  filterMentionCandidates,
  isEmptyAssistantTurn,
  isPinnedToBottom,
  makeDisplayName,
  mentionQueryAt,
  pruneDetachedMentions,
  referencedMentions,
  selectedBackend,
} from "./logic";
import {
  STARTER_PROMPTS,
  type AgentMention,
  type AgentMessage,
  type BackendStatus,
} from "./types";

const backend = (over: Partial<BackendStatus>): BackendStatus => ({
  hasApiKey: false,
  isSignedIn: false,
  isPaid: false,
  hasCredits: false,
  ...over,
});

const mention = (over: Partial<AgentMention>): AgentMention => ({
  id: "id-000000",
  kind: "mediaAsset",
  displayName: "Asset",
  label: "Asset",
  ...over,
});

const userMsg = (text: string): AgentMessage => ({
  id: "u",
  role: "user",
  blocks: [{ kind: "text", text }],
});

export function runAgentParityChecks(): string[] {
  const fail: string[] = [];
  const check = (cond: boolean, msg: string) => {
    if (!cond) fail.push(msg);
  };

  // --- model availability (lines 53-54, ruling #20) --------------------------
  {
    // BYOK → all three.
    const byok = availableModels(backend({ hasApiKey: true }));
    check(byok.length === 3, `BYOK must offer all 3 models, got ${byok.length}`);

    // Signed-in free → Haiku only.
    const free = availableModels(backend({ isSignedIn: true, isPaid: false }));
    check(
      free.length === 1 && free[0] === "claude-haiku-4-5-20251001",
      `free tier must be Haiku-only, got ${free.join()}`,
    );

    // Signed-in paid, no catalog → default Sonnet 4.6 (ruling #20).
    const paid = availableModels(backend({ isSignedIn: true, isPaid: true }));
    check(
      paid.length === 1 && paid[0] === "claude-sonnet-4-6",
      `paid (no catalog) must default to Sonnet 4.6, got ${paid.join()}`,
    );

    // Signed-in paid WITH catalog enabling Opus → catalog wins.
    const catalog = availableModels(
      backend({
        isSignedIn: true,
        isPaid: true,
        paidCatalog: ["claude-sonnet-4-6", "claude-opus-4-8"],
      }),
    );
    check(
      catalog.length === 2 && catalog.includes("claude-opus-4-8"),
      `paid catalog must drive availability, got ${catalog.join()}`,
    );

    // No backend → empty.
    check(
      availableModels(backend({})).length === 0,
      "no backend → no available models",
    );
  }

  // --- effectiveModel fallback chain (line 54) -------------------------------
  {
    // preferred allowed → preferred.
    check(
      effectiveModel(backend({ hasApiKey: true }), "claude-opus-4-8") ===
        "claude-opus-4-8",
      "effectiveModel keeps an allowed preference",
    );
    // preferred NOT allowed → first available.
    check(
      effectiveModel(
        backend({ isSignedIn: true, isPaid: false }),
        "claude-opus-4-8",
      ) === "claude-haiku-4-5-20251001",
      "effectiveModel falls back to first available when preference is disallowed",
    );
    // no backend → default Sonnet 4.6.
    check(
      effectiveModel(backend({}), null) === "claude-sonnet-4-6",
      "effectiveModel defaults to Sonnet 4.6 with no backend",
    );
  }

  // --- canStream + backend selection (lines 47-52) ---------------------------
  {
    check(canStream(backend({ hasApiKey: true })), "key → canStream");
    check(
      canStream(backend({ isSignedIn: true, hasCredits: true })),
      "signed-in with credits → canStream",
    );
    check(
      !canStream(backend({ isSignedIn: true, hasCredits: false })),
      "signed-in without credits → cannot stream",
    );
    check(!canStream(backend({})), "no backend → cannot stream");

    check(
      selectedBackend(backend({ hasApiKey: true })) === "anthropic",
      "key selects the BYOK Anthropic backend",
    );
    check(
      selectedBackend(backend({ isSignedIn: true })) === "palmier",
      "signed-in (no key) selects the Palmier proxy backend",
    );
    check(
      selectedBackend(backend({})) === "none",
      "no key + not signed-in → no backend",
    );
  }

  // --- send gating (line 232) ------------------------------------------------
  {
    const ok = backend({ hasApiKey: true });
    check(canSend(ok, false, "hi"), "send enabled: can stream, not streaming, non-empty");
    check(!canSend(ok, true, "hi"), "send disabled while streaming");
    check(!canSend(ok, false, "   "), "send disabled on whitespace-only draft");
    check(
      !canSend(backend({}), false, "hi"),
      "send disabled when cannot stream",
    );
  }

  // --- mention tokens (lines 133-135) ----------------------------------------
  {
    check(
      makeDisplayName("Beach  Sunset") === "Beach-Sunset",
      "makeDisplayName collapses spaces to a single -",
    );
    check(
      makeDisplayName("a - - b") === "a-b",
      "makeDisplayName collapses runs of spaces/dashes",
    );

    const d1 = attachMentionToken("", "Beach-Sunset");
    check(d1 === "@Beach-Sunset ", `attachMentionToken on empty draft, got "${d1}"`);
    const d2 = attachMentionToken("look at ", "Beach-Sunset");
    check(d2 === "look at @Beach-Sunset ", `attach appends a token, got "${d2}"`);
    // de-dupe: attaching the same token again is a no-op.
    check(
      attachMentionToken(d2, "Beach-Sunset") === d2,
      "attachMentionToken de-dupes an existing token",
    );

    check(
      draftContainsToken("use @Clip", "Clip"),
      "draftContainsToken finds a present token",
    );
    check(
      !draftContainsToken("use @Clipper", "Clip"),
      "draftContainsToken does not match a longer token as a prefix",
    );

    // disambiguation: two collapsing to the same name get #<first6>.
    const dis = disambiguateMentions([
      mention({ id: "aaaaaa11", displayName: "Clip" }),
      mention({ id: "bbbbbb22", displayName: "Clip" }),
    ]);
    check(
      dis[0].displayName === "Clip#aaaaaa" && dis[1].displayName === "Clip#bbbbbb",
      `collisions disambiguate with #<first6>, got ${dis.map((m) => m.displayName).join()}`,
    );
    // unique names are untouched.
    check(
      disambiguateMentions([mention({ displayName: "Solo" })])[0].displayName ===
        "Solo",
      "unique mention names are left unchanged",
    );
  }

  // --- referenced + pruned mentions (line 98, 135) ---------------------------
  {
    const ms = [
      mention({ id: "1", displayName: "Beach" }),
      mention({ id: "2", displayName: "City" }),
    ];
    const ref = referencedMentions("trim @Beach please", ms);
    check(
      ref.length === 1 && ref[0].displayName === "Beach",
      "referencedMentions keeps only tokens present in the text",
    );
    const pruned = pruneDetachedMentions("only @City left", ms);
    check(
      pruned.length === 1 && pruned[0].displayName === "City",
      "pruneDetachedMentions drops mentions whose token was deleted",
    );
  }

  // --- @mention autocomplete query/apply -------------------------------------
  {
    check(mentionQueryAt("hi @be", 6) === "be", "mentionQueryAt reads the partial @query");
    check(mentionQueryAt("hi @be", 3) === null, "mentionQueryAt null when caret is before @");
    check(mentionQueryAt("email a@b", 9) === null, "mentionQueryAt ignores @ inside a word");

    const cands = [
      mention({ id: "1", displayName: "Beach-Sunset", label: "Beach Sunset" }),
      mention({ id: "2", displayName: "City-Drone", label: "City Drone" }),
    ];
    const f = filterMentionCandidates(cands, "city");
    check(
      f.length === 1 && f[0].displayName === "City-Drone",
      "filterMentionCandidates matches by label/displayName case-insensitively",
    );

    const applied = applyMentionPick("ref @be", 7, "Beach-Sunset");
    check(
      applied.text === "ref @Beach-Sunset " && applied.caret === applied.text.length,
      `applyMentionPick replaces the partial token, got "${applied.text}"`,
    );
  }

  // --- title derivation (line 156) -------------------------------------------
  {
    check(
      deriveTitle("New chat", [userMsg("Make me a montage")]) ===
        "Make me a montage",
      "deriveTitle uses the first user text",
    );
    const long = "a".repeat(60);
    check(
      deriveTitle("New chat", [userMsg(long)]).length === 40,
      "deriveTitle caps at 40 chars",
    );
    check(
      deriveTitle("Existing title", [userMsg("ignored")]) === "Existing title",
      "deriveTitle does not overwrite a non-default title",
    );
    check(
      deriveTitle("New chat", []) === "New chat",
      "deriveTitle leaves the default when there is no user text",
    );
  }

  // --- text-delta append-in-place (line 106) ---------------------------------
  {
    let a: AgentMessage = { id: "a", role: "assistant", blocks: [] };
    a = appendTextDelta(a, "Hel");
    a = appendTextDelta(a, "lo");
    check(
      a.blocks.length === 1 &&
        a.blocks[0].kind === "text" &&
        a.blocks[0].text === "Hello",
      "appendTextDelta extends the last text block in place",
    );
    // After a tool_use block, a new text delta starts a fresh text block.
    a = {
      ...a,
      blocks: [...a.blocks, { kind: "toolUse", id: "t", name: "x", inputJson: "{}" }],
    };
    a = appendTextDelta(a, "more");
    const lastText = a.blocks[a.blocks.length - 1];
    check(
      lastText.kind === "text" && lastText.text === "more" && a.blocks.length === 3,
      "appendTextDelta starts a new text block after a non-text block",
    );
  }

  // --- empty-turn detection (lines 109-110) ----------------------------------
  {
    check(
      isEmptyAssistantTurn({ id: "a", role: "assistant", blocks: [] }),
      "an assistant turn with no blocks is empty (dropped on cancel)",
    );
    check(
      !isEmptyAssistantTurn({
        id: "a",
        role: "assistant",
        blocks: [{ kind: "text", text: "hi" }],
      }),
      "an assistant turn with text is not empty",
    );
  }

  // --- auto-scroll pin math (line 232) ---------------------------------------
  {
    // 1000 tall content, 400 viewport: pinned when within 48px of the bottom.
    check(
      isPinnedToBottom(560, 1000, 400, 48),
      "pinned when scrolled to (near) the bottom",
    );
    check(
      !isPinnedToBottom(100, 1000, 400, 48),
      "not pinned when scrolled up past the threshold",
    );
  }

  // --- starter prompts (line 30) ---------------------------------------------
  {
    check(STARTER_PROMPTS.length === 7, `expected 7 starter prompts, got ${STARTER_PROMPTS.length}`);
    const ids = new Set(STARTER_PROMPTS.map((p) => p.id));
    check(ids.size === 7, "starter-prompt ids are unique");
    check(
      STARTER_PROMPTS.every((p) => p.prompt.trim().length > 0),
      "every starter prompt has non-empty prompt text",
    );
    // The seven expected kinds are present (B-roll, opening, captions, VO, music,
    // organize, transcript cut).
    for (const id of ["broll", "opening", "captions", "vo", "music", "organize", "cut"]) {
      check(ids.has(id), `starter prompt "${id}" present`);
    }
  }

  return fail;
}
