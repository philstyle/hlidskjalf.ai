// Shared mutable state — single source of truth for cross-module globals

export const state = {
  connectionState: "connecting", // "connected" | "unreachable" | "offline"
  connectionLatency: 0, // ms from last health check
  lastSessions: null,
  sessionPollTimer: null,
  healthPollTimer: null,
  activeLaneFilter: null, // null = "All" — show every lane
  sseSource: null, // EventSource instance when SSE is active
  sseRetryTimer: null, // setTimeout ID for SSE reconnection
  cachedLanes: null, // [{ id, name, color, ... }] from /lanes — fetched once
  bootstrapState: null, // { state: "idle"|"running"|"complete"|"failed", last_lines: [...], completed_at: ... }
  wakeStatus: null, // { enabled, poll_interval_secs, last_poll, next_poll, queued_agent_total, queued_person_total, consecutive_errors, backoff_active, ... }
};
