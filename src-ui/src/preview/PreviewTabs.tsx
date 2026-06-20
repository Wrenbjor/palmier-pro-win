// Preview tab bar (E5-S10).
//
// Port of the macOS reference `PreviewContainerView.tabBar`: a horizontally-scrollable
// row with the always-present, non-closable `Timeline` tab plus closable per-asset
// tabs, each underlined in its clip-type theme color when active, with a close button.
// Selecting a tab activates it (engine `Transport::activate_tab`); closing one drops
// its retained transport state.

import { trackColor } from "../editor/theme";
import { isCloseable, tabDisplayName, tabId, type PreviewTab } from "./types";

export interface PreviewTabsProps {
  tabs: PreviewTab[];
  activeTabId: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onCloseAll: () => void;
}

export function PreviewTabs({ tabs, activeTabId, onSelect, onClose, onCloseAll }: PreviewTabsProps) {
  return (
    <div className="flex items-center gap-1 border-b border-white/10 px-2">
      <div className="flex flex-1 gap-3 overflow-x-auto py-1" style={{ scrollbarWidth: "none" }}>
        {tabs.map((tab) => {
          const id = tabId(tab);
          const active = id === activeTabId;
          const underline = underlineColor(tab);
          return (
            <div
              key={id}
              className="group flex shrink-0 cursor-pointer items-center gap-1 pb-1"
              style={{ borderBottom: `2px solid ${active ? underline : "transparent"}` }}
              onClick={() => onSelect(id)}
            >
              <span
                className={`whitespace-nowrap text-xs ${
                  active ? "font-semibold text-white" : "text-white/70"
                }`}
              >
                {tabDisplayName(tab)}
              </span>
              {isCloseable(tab) && (
                <button
                  type="button"
                  aria-label={`Close ${tabDisplayName(tab)}`}
                  className="flex h-4 w-4 items-center justify-center rounded text-[10px] text-white/40 hover:bg-white/10 hover:text-white"
                  onClick={(e) => {
                    e.stopPropagation();
                    onClose(id);
                  }}
                >
                  ✕
                </button>
              )}
            </div>
          );
        })}
      </div>

      <button
        type="button"
        aria-label="More"
        title="Close all tabs"
        disabled={tabs.length <= 1}
        className="flex h-6 w-6 items-center justify-center rounded text-white/70 hover:bg-white/10 disabled:opacity-30"
        onClick={onCloseAll}
      >
        ⋯
      </button>
    </div>
  );
}

/** The underline accent for a tab (reference `underlineColor`): clip-type theme color. */
function underlineColor(tab: PreviewTab): string {
  if (tab.kind === "timeline") return "rgb(99,102,241)"; // accent primary (indigo)
  return trackColor(tab.clipType, 1);
}
