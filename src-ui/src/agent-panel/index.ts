// Public API of the agent-panel module (E8-S8).
//
// The app shell mounts `<AgentPanel />` — self-contained (creates its own store +
// controller from the fixtures if none injected). Until the real Tauri agent commands
// land (the streaming run loop is palmier-agent E8-S4; the Tauri bridge is later), a
// `MockAgentStream` drives a fake assistant turn (text + a tool_use + tool_result)
// through the `AgentPanelController` command seam so the panel animates without a
// backend. Every integration point is marked `// TODO(integration): Tauri agent commands`.

// --- The mountable panel (app shell mounts this) ------------------------------
export { AgentPanel } from "./AgentPanel";
export type { AgentPanelProps } from "./AgentPanel";

// --- Sub-components (exported for reuse / tests) ------------------------------
export { TabBar } from "./TabBar";
export type { TabBarProps } from "./TabBar";
export { MessageList } from "./MessageList";
export type { MessageListProps } from "./MessageList";
export { ContentBlockView } from "./MessageBlocks";
export { EmptyState } from "./EmptyState";
export type { EmptyStateProps } from "./EmptyState";
export { InputArea } from "./InputArea";
export type { InputAreaProps } from "./InputArea";
export { ModelPicker } from "./ModelPicker";
export type { ModelPickerProps } from "./ModelPicker";

// --- Store (Zustand-shaped, self-contained) ----------------------------------
export {
  createAgentPanelStore,
  useAgentStore,
  openSessions,
  historySessions,
} from "./store";
export type { AgentPanelStore, AgentPanelState } from "./store";

// --- Command seam (Tauri agent commands replace the mock at integration) -----
export { AgentPanelController, localId } from "./controller";

// --- Mock stream (swapped for the real Tauri agent event stream) -------------
export {
  runMockTurn,
  mockToolTurn,
  mockClosingTurn,
  MOCK_TOOL_RESULT_TEXT,
} from "./mock-stream";
export type {
  AgentStreamEvent,
  AgentStopReason,
  MockTurn,
  MockStreamHandle,
} from "./mock-stream";

// --- Pure logic (model availability / gating / mentions / titles) — tested ---
export {
  availableModels,
  effectiveModel,
  canStream,
  canSend,
  selectedBackend,
  SIGN_IN_HINT,
  makeDisplayName,
  attachMentionToken,
  draftContainsToken,
  disambiguateMentions,
  referencedMentions,
  pruneDetachedMentions,
  mentionQueryAt,
  filterMentionCandidates,
  applyMentionPick,
  deriveTitle,
  firstUserMessageText,
  isNonEmptySession,
  appendTextDelta,
  isEmptyAssistantTurn,
  formatToolInput,
  isPinnedToBottom,
} from "./logic";

// --- Theme constants ---------------------------------------------------------
export { Theme, Spacing, Interaction, rgba } from "./theme";

// --- Fixtures (swapped for agent_status / load_sessions at integration) ------
export {
  makeFixtureMentions,
  makeFixtureBackend,
  makeFixtureSessions,
} from "./fixture";

// --- Types -------------------------------------------------------------------
export type {
  AgentModelId,
  AgentModel,
  BackendStatus,
  AgentRole,
  ToolResultBlock,
  AgentContentBlock,
  AgentMessage,
  MentionKind,
  AgentMention,
  ChatSession,
  AgentStreamError,
  StarterPrompt,
} from "./types";
export {
  AGENT_MODELS,
  DEFAULT_MODEL,
  MODEL_PREF_KEY,
  NEW_CHAT_TITLE,
  STARTER_PROMPTS,
} from "./types";

// --- Parity checks (tsc-covered; runnable via _run-parity.mts) ---------------
export { runAgentParityChecks } from "./parity.checks";
