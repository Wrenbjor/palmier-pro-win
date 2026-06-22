// InspectorController — the command/event seam feeding the Inspector its account
// state (E12-S2). Mirrors the editor's `EditController` / media-panel
// `MediaPanelController` boundary convention.
//
// The Inspector is a PURE VIEW over reactive state. The ONE piece of state it
// cannot derive from the timeline/selection is the account/AI configuration
// (`AccountService.isMisconfigured`), which gates the "AI Edit" tab. That state
// lives in `palmier-auth` and is surfaced to the frontend by the app shell's
// `get_account` Tauri command + the account update event (`app/api.ts`).
//
// COMMAND/EVENT SEAM — this controller is the boundary the epic asks for:
//   - `refresh()` pulls the current snapshot via `getAccount()` (Tauri command).
//   - `subscribe()` listens for live account updates via the Tauri event stream.
// Outside Tauri (plain `vite dev`) or before a live account command is wired,
// BOTH degrade to a MOCK snapshot behind the SAME seam — exactly the pattern the
// media-panel uses for its not-yet-live commands. Consumers never know which.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type AccountSnapshot,
  getAccount,
  inTauri,
} from "../../app/api";
import type { AccountState } from "./types";

/** The account event the app shell emits when the snapshot changes. */
const ACCOUNT_EVENT = "account://snapshot";

/**
 * The default MOCK account used when no live `get_account` is available
 * (browser dev, or before Epic 1's account command is reachable from here).
 * `isMisconfigured: false` so the AI Edit tab is exercisable in design preview;
 * `aiAllowed: false` so the actions render in their (correct) gated state until a
 * real signed-in account flows in. Flip via `mockAccount` for other paths.
 */
export const MOCK_ACCOUNT: AccountState = {
  isMisconfigured: false,
  isSignedIn: false,
  aiAllowed: false,
};

/** Narrow a full `AccountSnapshot` down to the slice the Inspector gates on. */
export function accountStateFromSnapshot(
  snap: AccountSnapshot,
): AccountState {
  return {
    isMisconfigured: snap.isMisconfigured,
    isSignedIn: snap.isSignedIn,
    aiAllowed: snap.aiAllowed,
  };
}

export class InspectorController {
  private state: AccountState;
  private listeners = new Set<(s: AccountState) => void>();
  private unlisten: UnlistenFn | null = null;

  constructor(initial: AccountState = MOCK_ACCOUNT) {
    this.state = initial;
  }

  /** Current account state (synchronous read for the resolver). */
  get account(): AccountState {
    return this.state;
  }

  private set(next: AccountState): void {
    this.state = next;
    for (const l of this.listeners) l(next);
  }

  /** Override the account state directly — used by tests / design preview. */
  mockAccount(next: AccountState): void {
    this.set(next);
  }

  /**
   * Pull the current account snapshot.
   * SEAM: `getAccount()` runs the real `get_account` Tauri command in the app;
   * outside Tauri it returns undefined and we keep the mock. Mirrors how
   * `MediaPanelController.loadMedia` degrades to the fixture.
   */
  async refresh(): Promise<AccountState> {
    if (!inTauri()) return this.state;
    const snap = await getAccount();
    if (snap) this.set(accountStateFromSnapshot(snap));
    return this.state;
  }

  /**
   * Subscribe to account state changes. Calls `cb` immediately with the current
   * value, then on every update. Returns an unsubscribe fn.
   *
   * SEAM: when in Tauri, also wires the `account://snapshot` event so live
   * sign-in / key changes re-gate the AI Edit tab without a manual refresh.
   * Outside Tauri it is a pure in-memory subscription over the mock.
   */
  subscribe(cb: (s: AccountState) => void): () => void {
    this.listeners.add(cb);
    cb(this.state);

    if (inTauri() && !this.unlisten) {
      void listen<AccountSnapshot>(ACCOUNT_EVENT, (e) => {
        this.set(accountStateFromSnapshot(e.payload));
      }).then((un) => {
        this.unlisten = un;
      });
      // Kick an initial pull so the first paint reflects the real snapshot.
      void this.refresh();
    }

    return () => {
      this.listeners.delete(cb);
    };
  }

  /** Tear down the event listener (call on unmount). */
  dispose(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.listeners.clear();
  }
}
