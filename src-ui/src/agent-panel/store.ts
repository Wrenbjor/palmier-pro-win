// Agent-panel external store (E8-S8) — mirrors the media-panel's self-contained store.
//
// To avoid touching the shared pnpm lockfile (owned by the app-shell worker — scope
// guard) this is a tiny self-contained store exposing the same shape a Zustand store
// would (`getState`/`setState`/`subscribe`) plus a React `useAgentStore` hook on
// `useSyncExternalStore`. If Zustand is later added project-wide, swap for `create(...)`
// with no change to consumers.
//
// It holds everything the reference's `AgentService` keeps as @Observable state:
// sessions + the current session id, the live `messages` of the current session, the
// `draft` + attached `mentions`, `isStreaming`, `streamError`, the selected `model`,
// the `BackendStatus`, and panel UI state (collapsed, history-open).

import { useSyncExternalStore } from "react";
import { deriveTitle, isNonEmptySession } from "./logic";
import {
  NEW_CHAT_TITLE,
  type AgentMention,
  type AgentMessage,
  type AgentModelId,
  type AgentStreamError,
  type BackendStatus,
  type ChatSession,
} from "./types";

export interface AgentPanelState {
  // --- backend / models ---
  backend: BackendStatus;
  /** Persisted model preference (`"agentModel"`); null = use first available. */
  preferredModel: AgentModelId | null;

  // --- sessions / tabs ---
  sessions: ChatSession[];
  currentSessionId: string;

  // --- live conversation (the current session's messages, edited in place) ---
  messages: AgentMessage[];

  // --- composer ---
  draft: string;
  mentions: AgentMention[];

  // --- streaming ---
  isStreaming: boolean;
  streamError: AgentStreamError | null;

  // --- panel UI ---
  collapsed: boolean;
  historyOpen: boolean;
}

export interface AgentPanelStore {
  getState: () => AgentPanelState;
  setState: (partial: Partial<AgentPanelState>) => void;
  subscribe: (listener: () => void) => () => void;

  // backend / model
  setBackend: (backend: BackendStatus) => void;
  setPreferredModel: (model: AgentModelId) => void;

  // composer
  setDraft: (draft: string) => void;
  setMentions: (mentions: AgentMention[]) => void;

  // streaming
  setStreaming: (on: boolean) => void;
  setStreamError: (err: AgentStreamError | null) => void;

  // messages (live)
  setMessages: (messages: AgentMessage[]) => void;
  appendMessage: (message: AgentMessage) => void;
  updateMessage: (id: string, update: (m: AgentMessage) => AgentMessage) => void;
  removeMessage: (id: string) => void;

  // sessions / tabs
  newChat: () => string;
  selectSession: (id: string) => void;
  closeTab: (id: string) => void;
  deleteSession: (id: string) => void;
  /** Copy live `messages` into the current session, bump updatedAt, derive title. */
  syncMessagesIntoCurrentSession: () => void;

  // panel UI
  setCollapsed: (collapsed: boolean) => void;
  toggleCollapsed: () => void;
  setHistoryOpen: (open: boolean) => void;
}

let idCounter = 0;
function newSessionId(): string {
  idCounter += 1;
  return `session-${Date.now().toString(36)}-${idCounter}`;
}

function freshSession(): ChatSession {
  return {
    id: newSessionId(),
    title: NEW_CHAT_TITLE,
    updatedAt: new Date().toISOString(),
    messages: [],
    isOpen: true,
  };
}

export function createAgentPanelStore(
  initial?: Partial<AgentPanelState>,
): AgentPanelStore {
  // Always have at least one open current session.
  let sessions = initial?.sessions ?? [freshSession()];
  if (sessions.length === 0) sessions = [freshSession()];
  const current =
    initial?.currentSessionId ??
    sessions.find((s) => s.isOpen)?.id ??
    sessions[0].id;
  const currentSession = sessions.find((s) => s.id === current) ?? sessions[0];

  let state: AgentPanelState = {
    backend: initial?.backend ?? {
      hasApiKey: false,
      isSignedIn: false,
      isPaid: false,
      hasCredits: false,
    },
    preferredModel: initial?.preferredModel ?? null,
    sessions,
    currentSessionId: currentSession.id,
    messages: initial?.messages ?? currentSession.messages.slice(),
    draft: initial?.draft ?? "",
    mentions: initial?.mentions ?? [],
    isStreaming: initial?.isStreaming ?? false,
    streamError: initial?.streamError ?? null,
    collapsed: initial?.collapsed ?? false,
    historyOpen: initial?.historyOpen ?? false,
  };

  const listeners = new Set<() => void>();
  const emit = () => listeners.forEach((l) => l());
  const setState = (partial: Partial<AgentPanelState>) => {
    state = { ...state, ...partial };
    emit();
  };

  const syncIntoCurrent = () => {
    const sessions = state.sessions.map((s) =>
      s.id === state.currentSessionId
        ? {
            ...s,
            messages: state.messages.slice(),
            updatedAt: new Date().toISOString(),
            title: deriveTitle(s.title, state.messages),
          }
        : s,
    );
    state = { ...state, sessions };
  };

  return {
    getState: () => state,
    setState,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    setBackend: (backend) => setState({ backend }),
    setPreferredModel: (preferredModel) => setState({ preferredModel }),

    setDraft: (draft) => setState({ draft }),
    setMentions: (mentions) => setState({ mentions }),

    setStreaming: (isStreaming) => setState({ isStreaming }),
    setStreamError: (streamError) => setState({ streamError }),

    setMessages: (messages) => setState({ messages }),
    appendMessage: (message) => setState({ messages: [...state.messages, message] }),
    updateMessage: (id, update) =>
      setState({
        messages: state.messages.map((m) => (m.id === id ? update(m) : m)),
      }),
    removeMessage: (id) =>
      setState({ messages: state.messages.filter((m) => m.id !== id) }),

    newChat: () => {
      syncIntoCurrent();
      const session = freshSession();
      setState({
        sessions: [...state.sessions, session],
        currentSessionId: session.id,
        messages: [],
        draft: "",
        mentions: [],
        streamError: null,
        historyOpen: false,
      });
      return session.id;
    },

    selectSession: (id) => {
      if (id === state.currentSessionId) {
        setState({ historyOpen: false });
        return;
      }
      // Persist the outgoing session, then load the incoming one. The caller is
      // expected to have cancelled any in-flight stream first (controller does this).
      syncIntoCurrent();
      const target = state.sessions.find((s) => s.id === id);
      if (!target) return;
      const sessions = state.sessions.map((s) =>
        s.id === id ? { ...s, isOpen: true } : s,
      );
      setState({
        sessions,
        currentSessionId: id,
        messages: target.messages.slice(),
        draft: "",
        mentions: [],
        streamError: null,
        historyOpen: false,
      });
    },

    closeTab: (id) => {
      // Closing the current tab first syncs it; closing the last open tab → new chat.
      const wasCurrent = id === state.currentSessionId;
      if (wasCurrent) syncIntoCurrent();
      let sessions = state.sessions.map((s) =>
        s.id === id ? { ...s, isOpen: false } : s,
      );
      const open = sessions.filter((s) => s.isOpen);
      if (open.length === 0) {
        const session = freshSession();
        sessions = [...sessions, session];
        setState({
          sessions,
          currentSessionId: session.id,
          messages: [],
          draft: "",
          mentions: [],
          streamError: null,
        });
        return;
      }
      if (wasCurrent) {
        const next = open[open.length - 1];
        setState({
          sessions,
          currentSessionId: next.id,
          messages: next.messages.slice(),
          draft: "",
          mentions: [],
          streamError: null,
        });
      } else {
        setState({ sessions });
      }
    },

    deleteSession: (id) => {
      let sessions = state.sessions.filter((s) => s.id !== id);
      if (id === state.currentSessionId) {
        const open = sessions.filter((s) => s.isOpen);
        const next = open[open.length - 1];
        if (next) {
          setState({
            sessions,
            currentSessionId: next.id,
            messages: next.messages.slice(),
            draft: "",
            mentions: [],
            streamError: null,
          });
          return;
        }
        const session = freshSession();
        sessions = [...sessions, session];
        setState({
          sessions,
          currentSessionId: session.id,
          messages: [],
          draft: "",
          mentions: [],
          streamError: null,
        });
        return;
      }
      setState({ sessions });
    },

    syncMessagesIntoCurrentSession: () => {
      syncIntoCurrent();
      emit();
    },

    setCollapsed: (collapsed) => setState({ collapsed }),
    toggleCollapsed: () => setState({ collapsed: !state.collapsed }),
    setHistoryOpen: (historyOpen) => setState({ historyOpen }),
  };
}

/** The sessions that have a tab (isOpen). History shows the rest. */
export function openSessions(sessions: ChatSession[]): ChatSession[] {
  return sessions.filter((s) => s.isOpen);
}

/** All non-empty sessions, newest first — the history list (agent-panel.md line 151). */
export function historySessions(sessions: ChatSession[]): ChatSession[] {
  return sessions
    .filter((s) => isNonEmptySession(s.messages))
    .slice()
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

/** Subscribe a React component to a slice of the store. */
export function useAgentStore<T>(
  store: AgentPanelStore,
  selector: (s: AgentPanelState) => T,
): T {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.getState()),
    () => selector(store.getState()),
  );
}
