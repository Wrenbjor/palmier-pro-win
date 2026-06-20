// Account tab (E1-S9): signed-in subscription + credits vs signed-out plan cards.
//
// The tab is only rendered when configured (Settings.tsx filters it out when
// `isMisconfigured`). Reference `AccountPane`:
// - isLoading → "Loading…"
// - signed-in + paid → subscription section (plan label, cancel notice) + credits
//   (remaining + Buy-more with `TopOffField`, range $5–$1000, default $20).
// - signed-in + free → plan cards (Pro / Max).
// - signed-out → "Sign in with Google".
//
// The Clerk webview embed + the billing-checkout invocation are a later story (full
// Clerk React embed); this tab reads the `palmier-auth` managed account state and shows
// the correct cards/credits. Billing/sign-in buttons are present but call into the
// not-yet-wired flow (logged no-op via the missing command, degrading gracefully).
import { useState } from "react";
import type { AccountSnapshot } from "../../app/api";

interface AccountTabProps {
  account: AccountSnapshot | null;
  onRefresh: () => void;
}

export default function AccountTab({ account, onRefresh }: AccountTabProps) {
  const [topOff, setTopOff] = useState<number>(account?.topOffDefault ?? 20);

  if (!account || account.isLoading) {
    return <p className="text-sm text-white/50">Loading…</p>;
  }

  return (
    <div>
      <h1 className="mb-6 text-lg font-semibold">Account</h1>

      {account.lastError && (
        <p className="mb-4 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          {account.lastError}
        </p>
      )}

      {account.isSignedIn ? (
        <>
          <section className="mb-8">
            <div className="text-sm text-white/60">Signed in{account.email ? ` as ${account.email}` : ""}</div>
            <div className="mt-1 text-lg font-medium">{account.planLabel}</div>
          </section>

          {account.tier !== "none" ? (
            <section className="mb-8 grid grid-cols-2 gap-4">
              <div className="rounded-lg border border-white/10 bg-white/5 p-4">
                <div className="text-xs uppercase tracking-wide text-white/40">Remaining credits</div>
                <div className="mt-1 text-2xl font-semibold">{account.remainingCredits}</div>
                {account.budgetCredits != null && (
                  <div className="text-xs text-white/40">of {account.budgetCredits}</div>
                )}
              </div>
              <div className="rounded-lg border border-white/10 bg-white/5 p-4">
                <div className="mb-2 text-xs uppercase tracking-wide text-white/40">Buy more</div>
                <div className="flex items-center gap-2">
                  <span className="text-sm">$</span>
                  <input
                    type="number"
                    min={account.topOffMin}
                    max={account.topOffMax}
                    value={topOff}
                    onChange={(e) => {
                      const v = Number(e.target.value);
                      setTopOff(Math.min(account.topOffMax, Math.max(account.topOffMin, v)));
                    }}
                    className="w-24 rounded-md border border-white/15 bg-black/30 px-2 py-1.5 text-sm outline-none focus:border-[#F29933]"
                  />
                  <button
                    type="button"
                    className="rounded-md bg-[#F29933] px-3 py-1.5 text-sm font-medium text-black hover:brightness-110"
                  >
                    Buy
                  </button>
                </div>
                <div className="mt-1 text-xs text-white/40">
                  ${account.topOffMin}–${account.topOffMax}
                </div>
              </div>
            </section>
          ) : (
            <section className="mb-8 grid grid-cols-2 gap-4">
              <div className="rounded-lg border border-[#F29933]/50 bg-[#F29933]/10 p-5">
                <div className="text-lg font-semibold">Pro plan</div>
                <div className="text-sm text-white/60">Upgrade for more credits.</div>
              </div>
              <div className="rounded-lg border border-white/10 bg-white/5 p-5">
                <div className="text-lg font-semibold">Max plan</div>
                <div className="text-sm text-white/60">For heavy use.</div>
              </div>
            </section>
          )}

          <button
            type="button"
            onClick={onRefresh}
            className="text-sm text-white/60 hover:underline"
          >
            Refresh
          </button>
        </>
      ) : (
        <section>
          <p className="mb-4 text-sm text-white/60">
            Sign in to use AI generation and sync your credits.
          </p>
          <button
            type="button"
            className="rounded-md border border-white/15 px-4 py-2 text-sm font-medium hover:bg-white/10"
          >
            Sign in with Google
          </button>
        </section>
      )}
    </div>
  );
}
