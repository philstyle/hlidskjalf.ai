// API calls, SSE, and polling

import { timedFetch, getToken, clearToken } from "./utils.js";
import { state } from "./state.js";

export async function pair(key) {
  const res = await timedFetch(`/pair?key=${encodeURIComponent(key)}`);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new Error(
      body?.error?.code === "invalid_key"
        ? "Invalid pairing key. Please scan the QR code again."
        : `Pairing failed (${res.status})`
    );
  }
  const data = await res.json();
  return data.token;
}

export async function fetchSessions() {
  const token = getToken();
  const res = await timedFetch("/sessions", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) {
    throw new Error("auth");
  }
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  return res.json();
}

export async function fetchLanes() {
  const token = getToken();
  const res = await timedFetch("/lanes", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function patchCard(cardId, body) {
  const token = getToken();
  const res = await timedFetch(`/cards/${cardId}`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const data = await res.json().catch(() => null);
    throw new Error(data?.error?.message || `HTTP ${res.status}`);
  }
  return res.json();
}

export async function killSession(sessionId) {
  const token = getToken();
  const res = await timedFetch(`/sessions/${sessionId}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) {
    const data = await res.json().catch(() => null);
    throw new Error(data?.error?.message || `HTTP ${res.status}`);
  }
  return res.json();
}

export async function fetchInbox() {
  const token = getToken();
  const res = await timedFetch("/inbox", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function updateInboxItem(filename, action, actionIndex) {
  const token = getToken();
  const body = { action };
  if (actionIndex !== undefined) body.action_index = actionIndex;
  const res = await timedFetch(`/inbox/${encodeURIComponent(filename)}`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
}

export async function fetchDispatchPrs() {
  const token = getToken();
  const res = await timedFetch("/dispatch", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) return [];
  return res.json();
}

export async function fetchBootstrapStatus() {
  const token = getToken();
  const res = await timedFetch("/bootstrap/status", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function triggerBootstrap() {
  const token = getToken();
  const res = await timedFetch("/bootstrap", {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  return { status: res.status, data: await res.json() };
}

export async function checkHealth() {
  const start = performance.now();
  try {
    const res = await timedFetch("/health");
    state.connectionLatency = Math.round(performance.now() - start);
    if (res.ok) {
      state.connectionState = "connected";
    } else {
      state.connectionState = "unreachable";
    }
  } catch {
    state.connectionLatency = 9999;
    state.connectionState = navigator.onLine ? "unreachable" : "offline";
  }
}

const SESSION_POLL_MS = 5000;
const HEALTH_POLL_MS = 10000;
const SSE_RETRY_MS = 5000;

// --- SSE ---

function connectSSE(viewManager, showBoard, renderScanScreen) {
  const token = getToken();
  if (!token) return;

  const es = new EventSource(`/events?token=${encodeURIComponent(token)}`);
  state.sseSource = es;

  es.onopen = () => {
    // SSE connected — stop session polling (health polling continues)
    if (state.sessionPollTimer) {
      clearTimeout(state.sessionPollTimer);
      state.sessionPollTimer = null;
    }
    // Fetch initial wake status
    timedFetch("/wake/status", { headers: { Authorization: `Bearer ${getToken()}` } })
      .then(r => r.ok ? r.json() : null)
      .then(data => { if (data) { state.wakeStatus = data; } })
      .catch(() => {});
  };

  es.onmessage = () => {
    // Default message handler — shouldn't fire since we use named events
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  };

  // Listen for specific event types from the backend
  for (const evt of ["session:started", "session:exited", "card:updated", "board:refresh"]) {
    es.addEventListener(evt, () => {
      refreshFromSSE(viewManager, showBoard, renderScanScreen);
    });
  }

  es.addEventListener("bootstrap:state", (e) => {
    try {
      state.bootstrapState = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("bootstrap-state-update", { detail: state.bootstrapState }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("hook:tool", (e) => {
    try {
      const data = JSON.parse(e.data);
      if (state.lastSessions && data.card_id) {
        const card = state.lastSessions.find(s => s.card_id === data.card_id);
        if (card) card.current_activity = data.activity || null;
      }
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  for (const evt of ["hook:session-start", "hook:session-end", "hook:alert", "hook:compact"]) {
    es.addEventListener(evt, () => {
      refreshFromSSE(viewManager, showBoard, renderScanScreen);
    });
  }

  es.addEventListener("wake:status_changed", (e) => {
    try {
      const data = JSON.parse(e.data);
      state.wakeStatus = data;
      document.dispatchEvent(new CustomEvent("wake-status-update", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:poll_completed", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("wake-poll-completed", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:dispatch_detected", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("wake-dispatch-detected", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:error", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("wake-error", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:channel_muted", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("wake-channel-muted", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:relay_message_detected", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("relay-message-detected", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:relay_tap_sent", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("relay-tap-sent", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.addEventListener("wake:pending_high_water", (e) => {
    try {
      const data = JSON.parse(e.data);
      document.dispatchEvent(new CustomEvent("relay-pending-high-water", { detail: data }));
    } catch {}
    refreshFromSSE(viewManager, showBoard, renderScanScreen);
  });

  es.onerror = () => {
    // SSE disconnected — close and fall back to polling
    disconnectSSE();
    startSessionPolling(viewManager, showBoard, renderScanScreen);
    // Retry SSE after delay
    state.sseRetryTimer = setTimeout(() => {
      connectSSE(viewManager, showBoard, renderScanScreen);
    }, SSE_RETRY_MS);
  };
}

let sseDebounceTimer = null;

function refreshFromSSE(viewManager, showBoard, renderScanScreen) {
  // Debounce rapid SSE events — coalesce into a single fetch
  if (sseDebounceTimer) return;
  sseDebounceTimer = setTimeout(async () => {
    sseDebounceTimer = null;
    try {
      const sessions = await fetchSessions();
      state.lastSessions = sessions;
      if (viewManager.currentViewName === "board") showBoard(sessions);
    } catch (e) {
      if (e.message === "auth") {
        stopPolling();
        disconnectSSE();
        clearToken();
        renderScanScreen("Session expired — scan QR to reconnect.");
      }
    }
  }, 300);
}

function disconnectSSE() {
  if (state.sseSource) {
    state.sseSource.close();
    state.sseSource = null;
  }
  if (state.sseRetryTimer) {
    clearTimeout(state.sseRetryTimer);
    state.sseRetryTimer = null;
  }
}

// --- Polling ---

function startSessionPolling(viewManager, showBoard, renderScanScreen) {
  if (state.sessionPollTimer) return; // Already polling

  async function pollSessions() {
    // Don't poll if SSE is active
    if (state.sseSource && state.sseSource.readyState === EventSource.OPEN) return;

    try {
      const sessions = await fetchSessions();
      state.lastSessions = sessions;
      if (viewManager.currentViewName === "board") showBoard(sessions);
    } catch (e) {
      if (e.message === "auth") {
        stopPolling();
        disconnectSSE();
        clearToken();
        renderScanScreen("Session expired — scan QR to reconnect.");
        return;
      }
      if (state.lastSessions && viewManager.currentViewName === "board") {
        showBoard(state.lastSessions);
      }
    }
    state.sessionPollTimer = setTimeout(pollSessions, SESSION_POLL_MS);
  }

  pollSessions();
}

export function startPolling(viewManager, showBoard, renderScanScreen) {
  stopPolling();

  // Try SSE first, with polling as fallback
  connectSSE(viewManager, showBoard, renderScanScreen);
  startSessionPolling(viewManager, showBoard, renderScanScreen);

  // Health polling always runs (updates signal bars)
  async function pollHealth() {
    await checkHealth();
    if (state.lastSessions && viewManager.currentViewName === "board") {
      showBoard(state.lastSessions);
    }
    state.healthPollTimer = setTimeout(pollHealth, HEALTH_POLL_MS);
  }

  pollHealth();
}

export function stopPolling() {
  if (state.sessionPollTimer) {
    clearTimeout(state.sessionPollTimer);
    state.sessionPollTimer = null;
  }
  if (state.healthPollTimer) {
    clearTimeout(state.healthPollTimer);
    state.healthPollTimer = null;
  }
  if (sseDebounceTimer) {
    clearTimeout(sseDebounceTimer);
    sseDebounceTimer = null;
  }
  disconnectSSE();
}
