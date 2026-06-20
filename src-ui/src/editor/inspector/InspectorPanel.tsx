// InspectorPanel — the right-rail Inspector ROOT (E12-S2).
//
// A PURE VIEW over reactive editor state: header (Timeline / Inspector / Source),
// the clip tab-bar (gated + ordered by `logic.ts`), the marquee "N selected"
// summary, and the no-selection Project/Format metadata. It never touches
// FFmpeg/wgpu — it reads selection + timeline + account state and renders.
//
// SCOPE (E12-S2): the shell + header + tab-bar + no-selection panels. The actual
// tab BODIES (Video/Audio/Text/Details/AI-Edit) are E12-S5/S6/S7. Their content
// areas are STUBBED here via the `tabBodies` render-prop seam — sibling stories
// fill them by passing a `TabBodyRenderer` (see `index.ts` for the contract).
//
// State model: `preferredTab` is the one piece of UI-persistent state the shell
// owns (reference `@State preferredTab`). It is re-resolved on every selection
// change via `resolvePreferredTab`, which also drives the `cropEditingActive`
// clear (surfaced through `onClearCropEditing`).

import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, JSX, ReactNode } from "react";
import { InspectorController } from "./controller";
import {
  middleTruncate,
  resolveInspector,
  resolvePreferredTab,
  shouldClearCropEditing,
} from "./logic";
import { FontSize, Spacing, Theme, Tracking } from "./theme";
import {
  CLIP_TAB_LABEL,
  type AccountState,
  type ClipTab,
  type FormatMetadata,
  type InspectorInput,
  type InspectorState,
  type ProjectMetadata,
} from "./types";

/**
 * Render-prop seam the tab-body stories (E12-S5/S6/S7) plug into. Given the
 * resolved state + active tab + the live input, return the body for that tab.
 * The default (`PlaceholderTabBody`) renders a labelled placeholder so the shell
 * is functional standalone; siblings replace it WITHOUT touching this file.
 */
export type TabBodyRenderer = (ctx: {
  tab: ClipTab;
  input: InspectorInput;
  state: InspectorState;
}) => ReactNode;

/**
 * Render-prop seam for the media-asset "Source" inspector (E12-S9). Returns the
 * asset body; defaults to a placeholder.
 */
export type AssetBodyRenderer = (ctx: {
  input: InspectorInput;
  state: InspectorState;
}) => ReactNode;

export interface InspectorPanelProps {
  /** The reactive input WITHOUT `account` — account flows via the controller. */
  input: Omit<InspectorInput, "account">;
  /** Account seam. Pass a controller, or a static state for preview/tests. */
  controller?: InspectorController;
  account?: AccountState;
  /**
   * Called whenever crop editing must be cleared (every selection change, and
   * when the preferred tab leaves "video") — reference `editor.cropEditingActive`.
   */
  onClearCropEditing?: () => void;
  /** Tab-body seam (E12-S5/S6/S7). Defaults to labelled placeholders. */
  tabBodies?: TabBodyRenderer;
  /** Asset-body seam (E12-S9). Defaults to a labelled placeholder. */
  assetBody?: AssetBodyRenderer;
}

// ── Icon glyphs (SF-Symbol → unicode stand-in; real icons are an app-wide set) ──
const ICON_GLYPH: Record<string, string> = {
  "slider.horizontal.3": "≡", // ≡ sliders
  "info.circle": "ⓘ", // ⓘ
};

export function InspectorPanel(props: InspectorPanelProps): JSX.Element {
  const { input, onClearCropEditing, tabBodies, assetBody } = props;

  // Account state seam: prefer an explicit prop, else subscribe to the controller,
  // else fall back to the controller's mock default.
  const [account, setAccount] = useState<AccountState>(
    props.account ?? props.controller?.account ?? { isMisconfigured: false },
  );

  useEffect(() => {
    if (props.account) {
      setAccount(props.account);
      return;
    }
    const ctrl = props.controller ?? new InspectorController();
    const unsub = ctrl.subscribe(setAccount);
    return unsub;
  }, [props.account, props.controller]);

  const fullInput: InspectorInput = useMemo(
    () => ({ ...input, account }),
    [input, account],
  );

  // preferredTab: shell-owned UI state, re-resolved on selection change.
  const [preferredTab, setPreferredTab] = useState<ClipTab>("video");
  const prevSelKey = useRef<string>("");

  // Selection-change effect: resolve the next preferred tab + clear crop editing.
  // The selection identity is the sorted set of IDs (reference onChange selectedClipIds).
  const selKey = useMemo(
    () => [...fullInput.selectedClipIds].sort().join(","),
    [fullInput.selectedClipIds],
  );
  useEffect(() => {
    if (fullInput.isMarqueeSelecting) return; // reference: resolve only when NOT marqueeing
    if (selKey === prevSelKey.current) return;
    prevSelKey.current = selKey;
    setPreferredTab((cur) => {
      const next = resolvePreferredTab(fullInput, cur);
      // Reference clears cropEditingActive on every selection change.
      onClearCropEditing?.();
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selKey, fullInput.isMarqueeSelecting]);

  // Clearing crop editing also fires whenever the preferred tab leaves "video".
  useEffect(() => {
    if (shouldClearCropEditing(preferredTab)) onClearCropEditing?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preferredTab]);

  const state = useMemo(
    () => resolveInspector(fullInput, preferredTab),
    [fullInput, preferredTab],
  );

  return (
    <div style={rootStyle}>
      <Header
        title={state.header.title}
        icon={state.header.icon}
        marquee={fullInput.isMarqueeSelecting}
      />
      {state.mode === "marquee" && (
        <MarqueeSummary count={state.marqueeCount} />
      )}
      {state.mode === "clip" && (
        <ClipContent
          state={state}
          input={fullInput}
          onSelectTab={setPreferredTab}
          tabBodies={tabBodies ?? defaultTabBodies}
        />
      )}
      {state.mode === "asset" && (
        <div style={bodyScrollStyle}>
          {(assetBody ?? defaultAssetBody)({ input: fullInput, state })}
        </div>
      )}
      {state.mode === "project" && state.noSelection && (
        <ProjectPanel
          project={state.noSelection.project}
          format={state.noSelection.format}
        />
      )}
    </div>
  );
}

// ── Header ──────────────────────────────────────────────────────────────────────

function Header(props: {
  title: string;
  icon: string;
  marquee: boolean;
}): JSX.Element {
  return (
    <div style={headerStyle}>
      <span style={headerIconStyle}>{ICON_GLYPH[props.icon] ?? ""}</span>
      <span style={headerTitleStyle}>{props.title}</span>
    </div>
  );
}

function MarqueeSummary(props: { count: number }): JSX.Element {
  return (
    <div style={marqueeStyle}>
      <span style={{ color: Theme.text.tertiary, fontSize: FontSize.sm }}>
        {props.count} selected
      </span>
    </div>
  );
}

// ── Clip content: tab bar + body seam ────────────────────────────────────────────

function ClipContent(props: {
  state: InspectorState;
  input: InspectorInput;
  onSelectTab: (tab: ClipTab) => void;
  tabBodies: TabBodyRenderer;
}): JSX.Element {
  const { state, input, onSelectTab, tabBodies } = props;
  return (
    <div style={clipContentStyle}>
      {state.showTabBar && (
        <TabBar
          tabs={state.tabs}
          active={state.activeTab}
          onSelect={onSelectTab}
        />
      )}
      <div style={bodyScrollStyle}>
        {state.activeTab
          ? tabBodies({ tab: state.activeTab, input, state })
          : null}
      </div>
    </div>
  );
}

function TabBar(props: {
  tabs: ClipTab[];
  active: ClipTab | null;
  onSelect: (tab: ClipTab) => void;
}): JSX.Element {
  return (
    <div style={tabBarStyle} role="tablist">
      {props.tabs.map((tab) => {
        const isActive = tab === props.active;
        const isAI = tab === "ai";
        const color = isAI
          ? Theme.accentTimecode // AI gradient stand-in (single-color accent)
          : isActive
            ? Theme.text.primary
            : Theme.text.tertiary;
        return (
          <button
            key={tab}
            role="tab"
            aria-selected={isActive}
            onClick={() => props.onSelect(tab)}
            style={tabButtonStyle}
          >
            <span
              style={{
                color,
                fontSize: FontSize.sm,
                fontWeight: isActive ? 500 : 400,
              }}
            >
              {CLIP_TAB_LABEL[tab]}
            </span>
            <span
              style={{
                height: 1.5,
                background: isActive ? color : "transparent",
                marginTop: Spacing.xs,
                width: "100%",
              }}
            />
          </button>
        );
      })}
    </div>
  );
}

// ── No-selection: Project + Format ───────────────────────────────────────────────

function ProjectPanel(props: {
  project: ProjectMetadata | null;
  format: FormatMetadata | null;
}): JSX.Element {
  return (
    <div style={bodyScrollStyle}>
      <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
        {props.project && (
          <Section title="Project">
            <Row label="Name" value={props.project.name} />
            <Row label="Path" value={props.project.path} truncate="middle" />
          </Section>
        )}
        {props.format && (
          <Section title="Format">
            <Row label="Resolution" value={props.format.resolution} />
            <Row label="Frame Rate" value={props.format.frameRate} />
            <Row label="Aspect Ratio" value={props.format.aspectRatio} />
            <Row label="Duration" value={props.format.duration} />
          </Section>
        )}
      </div>
    </div>
  );
}

function Section(props: {
  title: string;
  children: ReactNode;
}): JSX.Element {
  return (
    <div
      style={{ display: "flex", flexDirection: "column", gap: Spacing.smMd }}
    >
      <div style={sectionTitleStyle}>{props.title.toUpperCase()}</div>
      <div style={{ display: "flex", flexDirection: "column", gap: Spacing.sm }}>
        {props.children}
      </div>
    </div>
  );
}

function Row(props: {
  label: string;
  value: string;
  truncate?: "tail" | "middle";
}): JSX.Element {
  // Middle-truncation: keep head + tail, elide the middle (path readability).
  const display =
    props.truncate === "middle" ? middleTruncate(props.value, 44) : props.value;
  return (
    <div style={rowStyle}>
      <span style={rowLabelStyle}>{props.label}</span>
      <span style={rowValueStyle} title={props.value}>
        {display}
      </span>
    </div>
  );
}

// ── Tab-body / asset-body placeholders (filled by E12-S5/S6/S7/S9) ───────────────

/** The default placeholder a sibling story replaces via the `tabBodies` prop. */
export function PlaceholderTabBody(props: { tab: ClipTab }): JSX.Element {
  return (
    <div style={placeholderStyle}>
      {CLIP_TAB_LABEL[props.tab]} tab — content lands in E12-S5/S6/S7
    </div>
  );
}

const defaultTabBodies: TabBodyRenderer = ({ tab }) => (
  <PlaceholderTabBody tab={tab} />
);

const defaultAssetBody: AssetBodyRenderer = () => (
  <div style={placeholderStyle}>Source (asset) inspector — content in E12-S9</div>
);

// ── Styles ────────────────────────────────────────────────────────────────────

const rootStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  background: Theme.background.surface,
  color: Theme.text.secondary,
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.xs,
  padding: `0 ${Spacing.lg}px`,
  height: 36,
  background: Theme.background.headerBar,
  borderBottom: `1px solid ${Theme.border.subtle}`,
  flexShrink: 0,
};

const headerIconStyle: CSSProperties = {
  fontSize: FontSize.xs,
  color: Theme.text.tertiary,
};

const headerTitleStyle: CSSProperties = {
  fontSize: FontSize.sm,
  fontWeight: 500,
  color: Theme.text.secondary,
};

const marqueeStyle: CSSProperties = {
  flex: 1,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const clipContentStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  flex: 1,
  minHeight: 0,
};

const tabBarStyle: CSSProperties = {
  display: "flex",
  gap: Spacing.md,
  padding: `${Spacing.xs}px ${Spacing.lg}px 0`,
};

const tabButtonStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  background: "transparent",
  border: "none",
  cursor: "pointer",
  padding: `${Spacing.xs}px 0`,
};

const bodyScrollStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflowY: "auto",
  padding: `${Spacing.md}px ${Spacing.lg}px`,
};

const sectionTitleStyle: CSSProperties = {
  fontSize: FontSize.xxs,
  fontWeight: 600,
  letterSpacing: Tracking.wide,
  color: Theme.text.muted,
};

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
};

const rowLabelStyle: CSSProperties = {
  fontSize: FontSize.xs,
  color: Theme.text.tertiary,
  flexShrink: 0,
};

const rowValueStyle: CSSProperties = {
  fontSize: FontSize.xs,
  color: Theme.text.secondary,
  marginLeft: "auto",
  textAlign: "right",
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
};

const placeholderStyle: CSSProperties = {
  fontSize: FontSize.xs,
  color: Theme.text.muted,
  padding: Spacing.sm,
};
