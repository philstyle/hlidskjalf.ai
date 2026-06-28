import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore } from "./settings";

// Strip ANSI escape codes from terminal output
function stripAnsi(str: string): string {
  return str.replace(
    // eslint-disable-next-line no-control-regex
    /[\u001b\u009b][[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nq-uy=><~]/g,
    "",
  );
}

interface AttachResponse {
  data: string;
  seq: number;
  cols: number;
  rows: number;
}

async function fetchRecentLines(sessionId: string, count: number): Promise<string[]> {
  const resp = await invoke<AttachResponse>("attach_session", {
    sessionId,
  });
  const stripped = stripAnsi(resp.data);
  const lines = stripped
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  return lines.slice(-count);
}

async function callHaiku(
  apiKey: string,
  lines: string[],
): Promise<string> {
  const context = lines.join("\n");

  const resp = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
      "content-type": "application/json",
      "anthropic-dangerous-direct-browser-access": "true",
    },
    body: JSON.stringify({
      model: "claude-haiku-4-5-20251001",
      max_tokens: 40,
      system: `You generate concise session status lines (5-10 words) for a coding session sidebar.

From the terminal output, identify what the USER last asked Claude to do and what Claude is doing in response. Focus on the user's goal, not on specific files or tools.

Always respond with ONLY the summary — no quotes, no punctuation at end, no explanation. Examples:
'Adding branch selection to new session modal'
'Reviewing app against Phase 1 spec'
'Debugging websocket reconnect logic'
'Waiting for user input'`,
      messages: [
        {
          role: "user",
          content: `Summarize the current task in 5-10 words:\n\n${context}`,
        },
      ],
    }),
  });

  if (!resp.ok) {
    throw new Error(`Anthropic API error: ${resp.status}`);
  }

  const data = await resp.json();
  return data.content?.[0]?.text?.trim() ?? "";
}

const THROTTLE_MS = 60_000;
const MIN_LINES = 5;

// Simple hash of lines for change detection — skip API call if output unchanged
function hashLines(lines: string[]): string {
  return lines.join("\n");
}

interface SummariesState {
  summaries: Record<string, string>;
  lastFetched: Record<string, number>;
  lastHash: Record<string, string>;
  loading: Record<string, boolean>;
  fetchSummary: (cardId: string, sessionId: string) => Promise<void>;
}

export const useSummariesStore = create<SummariesState>((set, get) => ({
  summaries: {},
  lastFetched: {},
  lastHash: {},
  loading: {},

  fetchSummary: async (cardId, sessionId) => {
    const { lastFetched, lastHash, loading } = get();

    // Throttle: skip if fetched too recently
    if (lastFetched[cardId] && Date.now() - lastFetched[cardId] < THROTTLE_MS) return;
    // Skip if already in flight
    if (loading[cardId]) return;

    const { settings } = useSettingsStore.getState();
    if (settings.ai_summaries !== "on") return;

    set((s) => ({ loading: { ...s.loading, [cardId]: true } }));

    try {
      // Fetch lines first for change detection (both paths need them)
      const lines = await fetchRecentLines(sessionId, 50);
      if (lines.length < MIN_LINES) return;

      // Change detection: skip API call if terminal output hasn't changed
      const hash = hashLines(lines);
      if (lastHash[cardId] && lastHash[cardId] === hash) {
        // Output unchanged — update timestamp to reset throttle but skip API call
        set((s) => ({ lastFetched: { ...s.lastFetched, [cardId]: Date.now() } }));
        return;
      }

      let summary: string;

      if (settings.anthropic_api_key) {
        summary = await callHaiku(settings.anthropic_api_key, lines);
      } else {
        summary = await invoke<string>("generate_summary_local", { cardId });
      }

      if (summary) {
        set((s) => ({
          summaries: { ...s.summaries, [cardId]: summary },
          lastFetched: { ...s.lastFetched, [cardId]: Date.now() },
          lastHash: { ...s.lastHash, [cardId]: hash },
        }));
        // Persist to DB for NexusLink mobile access
        invoke("update_card_summary", { cardId, summary }).catch(() => {});
      }
    } catch (e) {
      console.warn("[summaries] Failed to fetch summary:", e);
    } finally {
      set((s) => ({ loading: { ...s.loading, [cardId]: false } }));
    }
  },
}));
