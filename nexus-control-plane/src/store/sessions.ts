import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCardsStore } from "./cards";

export interface SessionInfo {
  sessionId: string;
  cardId: string;
  isAlive: boolean;
  isIdle: boolean;
  idleSince: number | null; // Date.now() timestamp
  preview: string;
  startedAt: string | null;
  lastOutputAt: number | null; // Date.now() timestamp of last output
  // Live status from Claude Code (via statusline sideband)
  model?: string;
  contextPercent?: number;
  permissionMode?: string;
  cost?: number;
  // Live state from Claude Code JSONL watcher
  claudeState: string | null;
  claudeTool: string | null;
  agentCount: number;
  lastTurnDurationMs: number | null;
  currentTurnStarted: string | null;
}

interface ActiveSessionInfo {
  card_id: string;
  session_id: string;
  started_at: string;
}

interface SessionStatus {
  card_id: string;
  session_id: string;
  model_display?: string;
  context_percent?: number;
  permission_mode?: string;
  cost_usd?: number;
}

interface SessionsState {
  sessions: Record<string, SessionInfo>; // keyed by cardId
  setSession: (cardId: string, sessionId: string, startedAt?: string | null) => void;
  updatePreview: (cardId: string, preview: string) => void;
  updateStatus: (cardId: string, status: Partial<Pick<SessionInfo, "model" | "contextPercent" | "permissionMode" | "cost">>) => void;
  markDead: (sessionId: string) => void;
  updateClaudeState: (cardId: string, state: string, tool: string | null, agentCount: number, lastTurnDurationMs: number | null, currentTurnStarted: string | null) => void;
  markIdle: (cardId: string) => void;
  removeSession: (cardId: string) => void;
  loadActiveSessions: () => Promise<void>;
}

export const useSessionsStore = create<SessionsState>((set, get) => ({
  sessions: {},
  setSession: (cardId, sessionId, startedAt = null) =>
    set((s) => {
      const existing = s.sessions[cardId];
      return {
        sessions: {
          ...s.sessions,
          [cardId]: {
            // Preserve live status fields if re-attaching to same session
            ...(existing?.sessionId === sessionId ? existing : {}),
            sessionId,
            cardId,
            isAlive: true,
            isIdle: false,
            idleSince: null,
            preview: existing?.preview ?? "",
            startedAt,
            lastOutputAt: existing?.lastOutputAt ?? null,
            claudeState: null,
            claudeTool: null,
            agentCount: 0,
            lastTurnDurationMs: null,
            currentTurnStarted: null,
          },
        },
      };
    }),
  updatePreview: (cardId, preview) =>
    set((s) => {
      const existing = s.sessions[cardId];
      if (!existing) return s;
      // If JSONL watcher is active, let it be the source of truth for idle state
      const idleOverride = existing.claudeState
        ? {}
        : { isIdle: false, idleSince: null };
      return {
        sessions: { ...s.sessions, [cardId]: { ...existing, preview, lastOutputAt: Date.now(), ...idleOverride } },
      };
    }),
  updateStatus: (cardId, status) =>
    set((s) => {
      const existing = s.sessions[cardId];
      if (!existing) return s;
      return {
        sessions: { ...s.sessions, [cardId]: { ...existing, ...status, lastOutputAt: Date.now() } },
      };
    }),
  markDead: (sessionId) =>
    set((s) => {
      const updated = { ...s.sessions };
      for (const key of Object.keys(updated)) {
        if (updated[key].sessionId === sessionId) {
          updated[key] = {
            ...updated[key],
            isAlive: false,
            claudeState: null,
            claudeTool: null,
            agentCount: 0,
            lastTurnDurationMs: null,
            currentTurnStarted: null,
          };
        }
      }
      return { sessions: updated };
    }),
  updateClaudeState: (cardId, state, tool, agentCount, lastTurnDurationMs, currentTurnStarted) =>
    set((s) => {
      const existing = s.sessions[cardId];
      if (!existing) return s;
      const isIdle = state === "idle";
      return {
        sessions: {
          ...s.sessions,
          [cardId]: {
            ...existing,
            claudeState: state,
            claudeTool: tool,
            agentCount,
            lastTurnDurationMs,
            currentTurnStarted,
            isIdle: isIdle ? true : false,
            idleSince: isIdle ? (existing.idleSince ?? Date.now()) : null,
          },
        },
      };
    }),
  markIdle: (cardId) =>
    set((s) => {
      const existing = s.sessions[cardId];
      if (!existing) return s;
      return {
        sessions: { ...s.sessions, [cardId]: { ...existing, isIdle: true, idleSince: existing.idleSince ?? Date.now() } },
      };
    }),
  removeSession: (cardId) =>
    set((s) => {
      const { [cardId]: _, ...rest } = s.sessions;
      return { sessions: rest };
    }),
  loadActiveSessions: async () => {
    const active = await invoke<ActiveSessionInfo[]>("list_active_sessions");
    const { sessions, setSession } = get();
    for (const s of active) {
      if (!sessions[s.card_id]) {
        setSession(s.card_id, s.session_id, s.started_at);
      }
    }
  },
}));

let listenersInitialized = false;

export async function initSessionListeners() {
  if (listenersInitialized) return;
  listenersInitialized = true;

  // Register ALL listeners FIRST so no events are missed,
  // then load active sessions to discover already-running PTYs.
  await listen<{ card_id: string; preview: string }>(
    "session:preview",
    (event) => {
      // Use getState() for latest actions to avoid stale closure
      useSessionsStore.getState().updatePreview(event.payload.card_id, event.payload.preview);
    },
  );

  await listen<string>("session:exit", (event) => {
    useSessionsStore.getState().markDead(event.payload);
  });

  // Session started externally (e.g., from phone via NexusLink)
  await listen<{ session_id: string; card_id: string; started_at: string }>(
    "session:started",
    (event) => {
      const { session_id, card_id, started_at } = event.payload;
      useSessionsStore.getState().setSession(card_id, session_id, started_at);
    },
  );

  // Session went idle (Claude waiting for input)
  await listen<{ session_id: string; card_id: string }>(
    "session:idle",
    (event) => {
      useSessionsStore.getState().markIdle(event.payload.card_id);
    },
  );

  // Claude session state from JSONL watcher
  await listen<{ card_id: string; state: { type: string; tool?: string; count?: number }; tool?: string; agent_count: number; last_turn_duration_ms?: number; current_turn_started?: string }>(
    "claude:state-changed",
    (event) => {
      const p = event.payload;
      useSessionsStore.getState().updateClaudeState(
        p.card_id,
        p.state.type,
        p.tool ?? p.state.tool ?? null,
        p.agent_count,
        p.last_turn_duration_ms ?? null,
        p.current_turn_started ?? null,
      );
    },
  );

  // Card created externally (e.g., from phone via NexusLink)
  await listen("card:created", () => {
    useCardsStore.getState().fetchCards();
  });

  // Live session status from Claude Code (via statusline sideband)
  await listen<SessionStatus>("session:status", (event) => {
    const { card_id, model_display, context_percent, permission_mode, cost_usd } =
      event.payload;
    useSessionsStore.getState().updateStatus(card_id, {
      model: model_display,
      contextPercent: context_percent,
      permissionMode: permission_mode,
      cost: cost_usd,
    });
  });

  // Now that all listeners are active, discover already-running sessions
  useSessionsStore.getState().loadActiveSessions();
}
