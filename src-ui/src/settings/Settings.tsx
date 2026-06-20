// Settings window surface (E1-S9): the 5 tabs Account / General / Models / Agent /
// Storage. The Account tab is hidden when the backend is misconfigured
// (settings-account-app.md "Settings tabs"). Reads booted prefs + account/key/MCP state
// from the Rust commands (`src/app/api.ts`).
import { useEffect, useState } from "react";
import { getAccount, getSettings, type AccountSnapshot, type SettingsSnapshot } from "../app/api";
import AccountTab from "./tabs/AccountTab";
import GeneralTab from "./tabs/GeneralTab";
import ModelsTab from "./tabs/ModelsTab";
import AgentTab from "./tabs/AgentTab";
import StorageTab from "./tabs/StorageTab";

type TabId = "account" | "general" | "models" | "agent" | "storage";

const TAB_LABELS: Record<TabId, string> = {
  account: "Account",
  general: "General",
  models: "Models",
  agent: "Agent",
  storage: "Storage",
};

export default function Settings() {
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [account, setAccount] = useState<AccountSnapshot | null>(null);
  const [active, setActive] = useState<TabId>("general");

  useEffect(() => {
    void getSettings().then((s) => s && setSettings(s));
    void getAccount().then((a) => a && setAccount(a));
  }, []);

  // Account tab hidden when misconfigured (Clerk/Convex not configured).
  const isMisconfigured = account?.isMisconfigured ?? true;
  const visibleTabs: TabId[] = (
    ["account", "general", "models", "agent", "storage"] as TabId[]
  ).filter((t) => t !== "account" || !isMisconfigured);

  // If the active tab got hidden (account), fall back to general.
  useEffect(() => {
    if (!visibleTabs.includes(active)) setActive("general");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isMisconfigured]);

  return (
    <div className="flex h-screen bg-[#161616] text-white">
      <nav className="w-44 shrink-0 border-r border-white/10 p-3">
        <ul className="space-y-1">
          {visibleTabs.map((t) => (
            <li key={t}>
              <button
                type="button"
                onClick={() => setActive(t)}
                className={`w-full rounded-md px-3 py-2 text-left text-sm ${
                  active === t ? "bg-white/15 font-medium" : "text-white/70 hover:bg-white/10"
                }`}
              >
                {TAB_LABELS[t]}
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <main className="flex-1 overflow-auto p-8">
        {active === "account" && <AccountTab account={account} onRefresh={() => void getAccount().then((a) => a && setAccount(a))} />}
        {active === "general" && settings && (
          <GeneralTab settings={settings} onChange={setSettings} />
        )}
        {active === "models" && <ModelsTab />}
        {active === "agent" && <AgentTab />}
        {active === "storage" && <StorageTab />}
      </main>
    </div>
  );
}
