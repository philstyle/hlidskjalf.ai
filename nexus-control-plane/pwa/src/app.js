// NexusLink PWA — Session Board + Terminal Streaming

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";

const TOKEN_KEY = "nexuslink_token";
const SERVER_KEY = "nexuslink_server";
const VIEW_MODE_KEY = "ncc_pwa_view_mode";
const SESSION_POLL_MS = 5000;
const HEALTH_POLL_MS = 10000;
const FETCH_TIMEOUT_MS = 5000;
// GitHub-proxied endpoints (/gh/repos) shell out to `gh`, which can take
// well over 5s for accounts with many repos — give them a longer leash.
const GH_FETCH_TIMEOUT_MS = 30000;
const WS_RECONNECT_MS = 500;

const app = document.getElementById("app");
globalThis.__nccDashboardBundleMarkers = {
  updateDashboard: true,
  fetchDashboardScreen: true,
};

// --- Utility ---

function timedFetch(url, opts = {}, timeoutMs = FETCH_TIMEOUT_MS) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  return fetch(url, { ...opts, signal: controller.signal }).finally(() =>
    clearTimeout(timeout)
  );
}

function getToken() {
  return localStorage.getItem(TOKEN_KEY);
}

function setToken(token) {
  localStorage.setItem(TOKEN_KEY, token);
  localStorage.setItem(SERVER_KEY, window.location.origin);
}

function clearToken() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(SERVER_KEY);
}

// --- NCC branding (name + accent color, like an iTerm tab) ---
// Cosmetic; sourced from the ncc_display_name / ncc_accent_color settings. Defaults
// match the server catalog so the chrome looks right before /settings returns.
let nccName = "Nexus Command Center";
let nccAccent = "#3B82F6";
// How the "Claude" action launches Claude in a session — runtime-configurable via the
// claude_launch_command setting. Safe fallback until /settings loads (then the configured
// default, which includes the per-workspace autoMemoryDirectory, takes over).
let claudeLaunchCommand = "claude --dangerously-skip-permissions";
let _brandingLoaded = false;

function applyBranding() {
  document.title = nccName;                                              // distinguishes browser tabs
  document.documentElement.style.setProperty("--ncc-accent", nccAccent); // tints top bar + bottom strip via CSS
  document.querySelectorAll(".ncc-name").forEach((el) => { el.textContent = nccName; });
  // Persistent bottom accent strip (created once) — a color cue at the bottom of every view.
  if (document.body) {
    let strip = document.getElementById("ncc-bottom-strip");
    if (!strip) {
      strip = document.createElement("div");
      strip.id = "ncc-bottom-strip";
      document.body.appendChild(strip);
    }
  }
}

async function loadBranding() {
  try {
    const res = await timedFetch("/settings", { headers: { Authorization: "Bearer " + getToken() } });
    if (!res.ok) return;
    const data = await res.json();
    const map = {};
    (data.settings || []).forEach((s) => { map[s.key] = s.value; });
    nccName = (map.ncc_display_name && String(map.ncc_display_name).trim()) || "Nexus Command Center";
    nccAccent = (map.ncc_accent_color && String(map.ncc_accent_color).trim()) || "#3B82F6";
    if (map.claude_launch_command && String(map.claude_launch_command).trim()) {
      claudeLaunchCommand = String(map.claude_launch_command);
    }
    _brandingLoaded = true;
    applyBranding();
  } catch (_) { /* leave defaults */ }
}
applyBranding();   // paint defaults immediately at module load

function escapeHtml(str) {
  const el = document.createElement("span");
  el.textContent = str;
  return el.innerHTML;
}

function escapeAttr(str) {
  return escapeHtml(String(str ?? "")).replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function stripAnsi(str) {
  return String(str || "").replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, "");
}

// --- State ---

let connectionState = "connecting"; // "connected" | "unreachable" | "offline"
let lastSessions = null;
let sessionPollTimer = null;
let healthPollTimer = null;

// Wizard state
let wizardStep = 0; // 0=closed, 1=name/lane, 2=source
let wizardName = "", wizardLaneId = "", wizardNotes = "";
let wizardSourceType = "github";
let wizardLocalPath = "", wizardSelectedRepo = null;
let wizardLanes = [], wizardRepos = [], wizardRepoSearch = "";
let wizardGhAuth = null, wizardOrg = "";
let wizardError = null, wizardCreating = false;
let wizardReposLoading = false;
let wizardLaunchClaude = false;
let wizardRelayEnabled = true;

// Terminal state
function initialViewMode() {
  const saved = localStorage.getItem(VIEW_MODE_KEY);
  return (saved === "board" || saved === "split" || saved === "dashboard") ? saved : "board";
}

let currentView = initialViewMode(); // "board" | "terminal" | "split" | "dashboard"
let currentSessionId = null;
let currentCardName = null;
let termInstance = null;
let fitAddon = null;
let ws = null;
let bufferSeq = 0;
let termStatus = "connecting"; // "connecting" | "live" | "disconnected" | "ended"
let resizeObserver = null;
let desktopCols = null;
let desktopRows = null;
let voiceMode = false;
let cameFromDashboard = false;
const dashboardSnapshots = new Map();
const dashboardScreenCache = new Map();

// Split layout state
let activeCardId = null;          // Card ID selected in sidebar
let sidebarWidthPx = 280;         // Current sidebar pixel width
const sidebarCollapsedLanes = new Set(); // Lane names currently collapsed

// Per-lane sort mode: "default" (sort_order) or "recent" (most recent activity first)
const laneSortModes = JSON.parse(localStorage.getItem("ncc_lane_sort_modes") || "{}");
function getLaneSortMode(lane) { return laneSortModes[lane] || "default"; }
function toggleLaneSortMode(lane) {
  laneSortModes[lane] = getLaneSortMode(lane) === "default" ? "recent" : "default";
  localStorage.setItem("ncc_lane_sort_modes", JSON.stringify(laneSortModes));
}

// Inbox state
let inboxPollTimer = null;        // Timer ID for 30s inbox badge poll
let inboxItems = [];              // Cached inbox items for badge count + panel render
let inboxPanelOpen = false;       // Whether inbox panel is currently displayed in split-panel
let inboxActiveTab = "all";       // Current tab filter in inbox panel
let savedSessionId = null;        // Saved session ID before opening inbox panel
let savedCardName = null;         // Saved card name before opening inbox panel
let savedCardId = null;           // Saved card ID before opening inbox panel
let settingsReturnView = null;
let wizardReturnView = null;
let fileBrowserReturnView = null;

let uiModalOpen = false; // Suppresses polling re-renders when context menu, edit modal, or delete confirm is open

function findCardData(cardId) {
  if (!lastSessions) return null;
  const s = lastSessions.find(c => c.card_id === cardId);
  if (!s) return null;
  return { card_id: s.card_id, card_name: s.card_name, session_id: s.session_id, is_alive: s.is_alive, workspace_path: s.workspace_path, lane_id: s.lane_id, lane_name: s.lane_name, notes: s.notes, relay_pending_count: s.relay_pending_count, relay_mode: s.relay_mode, relay_enabled: s.relay_enabled };
}

function setStoredViewMode(view) {
  if (view === "board" || view === "split" || view === "dashboard") {
    localStorage.setItem(VIEW_MODE_KEY, view);
  }
}

function viewSwitcherHtml(activeView) {
  const views = [
    ["board", "Board", "Kanban board — cards arranged in lanes (Queued, Active, Waiting…)"],
    ["split", "Split", "Sidebar session list beside one full terminal"],
    ["dashboard", "Grid", "Every live session at once, each a shrunk live terminal"],
  ];
  return `
    <div class="view-switcher" role="tablist" aria-label="View mode">
      ${views.map(([view, label, tip]) => `
        <button class="view-switch-btn${activeView === view ? " active" : ""}" data-view-mode="${view}" role="tab" aria-selected="${activeView === view ? "true" : "false"}" title="${tip}">${label}</button>
      `).join("")}
    </div>
  `;
}

function attachViewSwitcherHandlers() {
  document.querySelectorAll("[data-view-mode]").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      switchViewMode(btn.dataset.viewMode);
    });
  });
}

function refreshCurrentSessionView(sessions) {
  if (currentView === "board") renderBoard(sessions);
  else if (currentView === "split") updateSidebar(sessions);
  else if (currentView === "dashboard") updateDashboard(sessions);
}

function renderPreferredSessionView(sessions) {
  if (currentView === "dashboard") renderDashboard(sessions);
  else if (currentView === "split") renderSplitLayout(sessions);
  else renderBoard(sessions);
}

function switchViewMode(view) {
  if (view !== "board" && view !== "split" && view !== "dashboard") return;
  cameFromDashboard = false;
  setStoredViewMode(view);

  if (view !== "split") {
    cleanupTerminal();
    activeCardId = null;
    currentSessionId = null;
    currentCardName = null;
  }

  currentView = view;
  if (view === "dashboard") {
    window.location.hash = "dashboard";
    if (lastSessions) renderDashboard(lastSessions);
    return;
  }

  if (view === "board") {
    window.location.hash = "board";
    if (lastSessions) renderBoard(lastSessions);
    return;
  }

  window.location.hash = "split";
  if (lastSessions) renderSplitLayout(lastSessions);
}

function returnToDashboard() {
  cleanupTerminal();
  cameFromDashboard = false;
  activeCardId = null;
  currentSessionId = null;
  currentCardName = null;
  currentView = "dashboard";
  setStoredViewMode("dashboard");
  window.location.hash = "dashboard";
  if (lastSessions) renderDashboard(lastSessions);
}

// --- Inbox poll functions ---

function updateInboxBadge() {
  const badge = document.getElementById("inbox-badge");
  if (!badge) return;
  const count = inboxItems.length;
  badge.textContent = count;
  badge.style.display = count > 0 ? "" : "none";
}

function updateWakeIndicator() {
  const dot = document.querySelector(".wake-dot");
  const label = document.getElementById("wake-label");
  const badge = document.getElementById("wake-badge");
  if (!dot || !label) return;

  const ws = window._nccWakeStatus || null;
  if (!ws) {
    dot.className = "wake-dot wake-dot-off";
    label.textContent = "Wake: Off";
    if (badge) badge.style.display = "none";
    return;
  }

  if (!ws.enabled) {
    dot.className = "wake-dot wake-dot-off";
    label.textContent = "Wake: Off";
  } else if (ws.consecutive_errors >= 5) {
    dot.className = "wake-dot wake-dot-error";
    label.textContent = "Wake: Error";
  } else if (ws.backoff_active) {
    dot.className = "wake-dot wake-dot-backoff";
    label.textContent = "Wake: Active (backoff)";
  } else {
    dot.className = "wake-dot wake-dot-active";
    label.textContent = "Wake: Active";
  }

  const total = (ws.queued_agent_total || 0) + (ws.queued_person_total || 0);
  if (badge) {
    if (total > 0) {
      badge.textContent = total;
      badge.style.display = "";
    } else {
      badge.style.display = "none";
    }
  }
}

function fetchUsage() {
  timedFetch("/usage", { headers: { Authorization: "Bearer " + getToken() } })
    .then(r => r.ok ? r.json() : null)
    .then(data => { if (data) renderUsage(data); })
    .catch(() => {});
}

function renderUsage(data) {
  const el = document.getElementById("sidebar-usage");
  if (!el) return;

  let html = "";

  if (data.five_hour && data.five_hour.used_percentage != null) {
    const used = Math.round(data.five_hour.used_percentage);
    const color = used < 50 ? "#22c55e" : used < 75 ? "#eab308" : "#ef4444";
    html += `<div class="usage-row">
      <span class="usage-label">5h</span>
      <div class="usage-track"><div class="usage-fill" style="width:${used}%;background:${color}"></div></div>
      <span class="usage-pct">${used}%</span>
    </div>`;
  }

  if (data.seven_day && data.seven_day.used_percentage != null) {
    const used = Math.round(data.seven_day.used_percentage);
    const expected = Math.round(data.seven_day.expected_percentage || 0);
    const headroom = data.seven_day.headroom || 0;
    const color = headroom < 0 ? "#ef4444" : used < 50 ? "#22c55e" : used < 75 ? "#eab308" : "#f97316";
    html += `<div class="usage-row">
      <span class="usage-label">7d</span>
      <div class="usage-track">
        <div class="usage-fill" style="width:${used}%;background:${color}"></div>
        <div class="usage-pace" style="left:${expected}%" title="Pace: ${expected}%"></div>
      </div>
      <span class="usage-pct">${used}%</span>
    </div>`;
  }

  el.innerHTML = html;
}

function toggleWakePopover() {
  const existing = document.getElementById("wake-popover");
  if (existing) {
    existing.remove();
    return;
  }

  const popover = document.createElement("div");
  popover.id = "wake-popover";
  popover.className = "wake-popover";

  const ws = window._nccWakeStatus || null;
  const enabled = ws && ws.enabled;
  const lastPoll = ws && ws.last_poll ? ws.last_poll : "—";
  const nextPoll = ws && ws.next_poll ? ws.next_poll : "—";
  const errors = ws ? (ws.consecutive_errors || 0) : 0;
  const backoff = ws && ws.backoff_active;

  popover.innerHTML = `
    <div class="wake-popover-title">Agent Wake</div>
    <div class="wake-popover-row">
      <span>Status:</span>
      <span>${enabled ? (backoff ? "Active (backoff)" : "Active") : "Off"}</span>
    </div>
    <div class="wake-popover-row">
      <span>Last poll:</span><span>${lastPoll}</span>
    </div>
    <div class="wake-popover-row">
      <span>Next poll:</span><span>${nextPoll}</span>
    </div>
    <div class="wake-popover-row">
      <span>Errors:</span><span>${errors}</span>
    </div>
    <button class="wake-toggle-btn" id="wake-toggle-btn">${enabled ? "Disable Wake" : "Enable Wake"}</button>
  `;

  const anchor = document.getElementById("wake-label");
  if (anchor) {
    const rect = anchor.getBoundingClientRect();
    popover.style.bottom = `${window.innerHeight - rect.top + 8}px`;
    popover.style.left = `${rect.left}px`;
  } else {
    popover.style.bottom = "60px";
    popover.style.left = "16px";
  }

  document.body.appendChild(popover);

  const toggleBtn = document.getElementById("wake-toggle-btn");
  if (toggleBtn) {
    toggleBtn.addEventListener("click", async () => {
      popover.remove();
      const endpoint = enabled ? "/wake/disable" : "/wake/enable";
      try {
        const res = await timedFetch(endpoint, {
          method: "POST",
          headers: { Authorization: "Bearer " + getToken() },
        });
        if (res.ok) {
          const data = await res.json();
          window._nccWakeStatus = data;
          updateWakeIndicator();
        }
      } catch (_) {}
    });
  }

  // Close on outside click
  setTimeout(() => {
    document.addEventListener("click", function closePopover(e) {
      if (!popover.contains(e.target)) {
        popover.remove();
        document.removeEventListener("click", closePopover);
      }
    });
  }, 0);
}

function stopInboxPoll() {
  if (inboxPollTimer) {
    clearTimeout(inboxPollTimer);
    inboxPollTimer = null;
  }
}

async function pollInbox() {
  try {
    const res = await timedFetch("/inbox", {
      headers: { Authorization: "Bearer " + getToken() },
    });
    if (res.ok) {
      const data = await res.json();
      inboxItems = Array.isArray(data) ? data : (data.items || []);
      updateInboxBadge();
    }
  } catch (_) {
    // network error — keep stale count
  }
  inboxPollTimer = setTimeout(pollInbox, 30000);
}

function startInboxPoll() {
  stopInboxPoll();
  pollInbox();
}

// --- Inbox helpers (pure functions, adapted from inbox.js) ---

function formatDueDateInbox(due) {
  if (!due) return "";
  const today = new Date();
  const dueDate = new Date(due + "T00:00:00");
  const diffDays = Math.ceil((dueDate.getTime() - today.getTime()) / 86400000);
  if (diffDays < 0) return "overdue";
  if (diffDays === 0) return "today";
  if (diffDays === 1) return "tomorrow";
  if (diffDays <= 7) return dueDate.toLocaleDateString("en-US", { weekday: "short" });
  return due.slice(5);
}

function renderMarkdown(text) {
  if (!text) return "";
  let html = escapeHtml(text);
  // Code blocks (```)
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, '<pre class="md-code"><code>$2</code></pre>');
  // Inline code
  html = html.replace(/`([^`]+)`/g, '<code class="md-inline-code">$1</code>');
  // Headers
  html = html.replace(/^### (.+)$/gm, '<h4 class="md-h">$1</h4>');
  html = html.replace(/^## (.+)$/gm, '<h3 class="md-h">$1</h3>');
  // Bold
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  // Tables (simple — pipe-delimited)
  html = html.replace(/((?:^\|.+\|$\n?)+)/gm, (match) => {
    const rows = match.trim().split("\n").filter(r => r.trim());
    if (rows.length < 2) return match;
    const toRow = (r, tag) => "<tr>" + r.split("|").filter((_,i,a) => i > 0 && i < a.length - 1).map(c => `<${tag}>${c.trim()}</${tag}>`).join("") + "</tr>";
    const header = toRow(rows[0], "th");
    const body = rows.slice(2).map(r => toRow(r, "td")).join("");
    return `<table class="md-table">${header}${body}</table>`;
  });
  // Unordered lists
  html = html.replace(/^- (.+)$/gm, '<li>$1</li>');
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul class="md-list">$1</ul>');
  // Line breaks (but not inside pre/table)
  html = html.replace(/\n/g, '<br>');
  // Clean up double breaks after block elements
  html = html.replace(/<\/pre><br>/g, '</pre>');
  html = html.replace(/<\/table><br>/g, '</table>');
  html = html.replace(/<\/ul><br>/g, '</ul>');
  html = html.replace(/<\/h[34]><br>/g, m => m.replace('<br>', ''));
  return html;
}

function splitInboxItemHtml(item) {
  const priorityColors = {
    P0: "var(--nx-red)",
    P1: "var(--nx-yellow)",
    P2: "var(--nx-accent)",
    P3: "var(--nx-muted)",
  };
  const pColor = priorityColors[item.priority] || priorityColors.P3;

  const dueDateStr = formatDueDateInbox(item.due_date);
  const isOverdue = dueDateStr === "overdue";
  const isDueToday = dueDateStr === "today";
  const dueClass = isOverdue ? "inbox-due-overdue" : isDueToday ? "inbox-due-today" : "inbox-due";

  const totalActions = (item.action_items || []).length;
  const checkedActions = (item.action_items || []).filter((a) => a.checked).length;
  const allDone = totalActions > 0 && checkedActions === totalActions;

  const actionsHtml = (item.action_items || []).map((a, i) => `
    <button class="inbox-checkbox" data-filename="${escapeHtml(item.filename)}" data-action-idx="${i}">
      <span class="inbox-check-icon ${a.checked ? "checked" : ""}">${a.checked ? "&#9745;" : "&#9744;"}</span>
      <span class="inbox-check-text ${a.checked ? "checked" : ""}">${escapeHtml(a.text)}</span>
    </button>
  `).join("");

  return `
    <div class="inbox-item ${allDone ? "inbox-item-done" : ""}">
      <button class="inbox-item-row">
        <span class="inbox-priority" style="color: ${pColor}; background: ${pColor}15">${escapeHtml(item.priority || "P3")}</span>
        <span class="inbox-subject ${allDone ? "line-through" : ""}">${escapeHtml(item.subject || item.summary || "")}</span>
        ${item.waiting_on ? '<span class="inbox-waiting">waiting</span>' : ''}
        ${dueDateStr ? `<span class="${dueClass}">${dueDateStr}</span>` : ''}
        ${totalActions > 0 ? `<span class="inbox-progress ${allDone ? "inbox-progress-done" : ""}">${checkedActions}/${totalActions}</span>` : ''}
        <span class="inbox-chevron">&#x276F;</span>
      </button>
      <div class="inbox-detail">
        ${item.from ? `<div class="inbox-meta">From: ${escapeHtml(item.from)} &middot; ${escapeHtml(item.category || "")} &middot; ${escapeHtml(item.date || "")}</div>` : ''}
        ${item.workstream ? `<div class="inbox-meta">Workstream: ${escapeHtml(item.workstream)}</div>` : ''}
        ${item.body ? `<div class="inbox-body">${renderMarkdown(item.body)}</div>` : ''}
        ${item.summary && !item.body ? `<p class="inbox-summary">${escapeHtml(item.summary)}</p>` : ''}
        ${actionsHtml ? `<div class="inbox-actions-list">${actionsHtml}</div>` : ''}
        <button class="inbox-dismiss ${allDone ? "inbox-dismiss-done" : ""}" data-filename="${escapeHtml(item.filename || "")}">
          ${allDone ? "Done" : "Dismiss"}
        </button>
      </div>
    </div>
  `;
}

// --- Inbox panel functions ---

function renderInboxPanelContent() {
  const panel = document.querySelector(".split-panel");
  if (!panel) return;

  const tab = inboxActiveTab;
  let filteredItems = inboxItems;
  if (tab === "inbound") filteredItems = inboxItems.filter(i => i.type === "inbound" || i.direction === "inbound");
  else if (tab === "dispatch") filteredItems = inboxItems.filter(i => i.type === "dispatch" || i.direction === "dispatch");
  else if (tab === "personal") filteredItems = inboxItems.filter(i => i.type === "personal");

  const overdueCount = inboxItems.filter(i => formatDueDateInbox(i.due_date) === "overdue").length;
  const totalActions = inboxItems.reduce((sum, i) => sum + (i.action_items || []).length, 0);
  const checkedActions = inboxItems.reduce((sum, i) => sum + (i.action_items || []).filter(a => a.checked).length, 0);
  const openActions = totalActions - checkedActions;

  const tabsHtml = [
    { id: "all", label: "All" },
    { id: "inbound", label: "Inbound" },
    { id: "dispatch", label: "Dispatch" },
    { id: "personal", label: "Personal" },
  ].map(t => `<button class="inbox-tab ${t.id === tab ? "active" : ""}" data-tab="${t.id}">${t.label}</button>`).join("");

  const itemsHtml = filteredItems.length > 0
    ? filteredItems.map(item => splitInboxItemHtml(item)).join("")
    : '<div class="inbox-empty">&#10003; Inbox clear</div>';

  panel.innerHTML = `
    <div class="inbox-panel">
      <div class="inbox-panel-header">
        <button class="inbox-back-btn">&larr; Back</button>
        <h2 class="inbox-panel-title">Mission Control</h2>
        <span class="inbox-panel-count">${inboxItems.length}</span>
      </div>
      ${overdueCount > 0 || openActions > 0 ? `
      <div class="inbox-stats">
        ${overdueCount > 0 ? `<span class="inbox-stat inbox-stat-overdue">&#9888; ${overdueCount} overdue</span>` : ""}
        ${openActions > 0 ? `<span class="inbox-stat">${openActions} open action${openActions !== 1 ? "s" : ""}</span>` : ""}
      </div>
      ` : ""}
      <div class="inbox-tabs">${tabsHtml}</div>
      <div class="inbox-list">${itemsHtml}</div>
    </div>
  `;

  // Back button
  panel.querySelector(".inbox-back-btn").addEventListener("click", closeInboxPanel);

  // Tab buttons
  panel.querySelectorAll(".inbox-tab").forEach(btn => {
    btn.addEventListener("click", () => {
      inboxActiveTab = btn.dataset.tab;
      renderInboxPanelContent();
    });
  });

  // Expand/collapse rows
  panel.querySelectorAll(".inbox-item-row").forEach(btn => {
    btn.addEventListener("click", () => {
      const itemEl = btn.closest(".inbox-item");
      const detail = itemEl && itemEl.querySelector(".inbox-detail");
      if (detail) {
        detail.classList.toggle("open");
        const chevron = btn.querySelector(".inbox-chevron");
        if (chevron) chevron.style.transform = detail.classList.contains("open") ? "rotate(90deg)" : "";
      }
    });
  });

  // Checkboxes — optimistic toggle then PATCH
  panel.querySelectorAll(".inbox-checkbox").forEach(btn => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const filename = btn.dataset.filename;
      const idx = parseInt(btn.dataset.actionIdx, 10);
      const item = inboxItems.find(i => i.filename === filename);
      if (!item || !item.action_items) return;
      item.action_items[idx].checked = !item.action_items[idx].checked;
      renderInboxPanelContent();
      stopInboxPoll();
      startInboxPoll();
      try {
        await timedFetch(`/inbox/${encodeURIComponent(filename)}`, {
          method: "PATCH",
          headers: { Authorization: "Bearer " + getToken(), "Content-Type": "application/json" },
          body: JSON.stringify({ action: "toggle_action", action_index: idx }),
        });
      } catch (_) {}
    });
  });

  // Dismiss buttons — optimistic remove then PATCH
  panel.querySelectorAll(".inbox-dismiss").forEach(btn => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const filename = btn.dataset.filename;
      inboxItems = inboxItems.filter(i => i.filename !== filename);
      updateInboxBadge();
      renderInboxPanelContent();
      stopInboxPoll();
      startInboxPoll();
      try {
        await timedFetch(`/inbox/${encodeURIComponent(filename)}`, {
          method: "PATCH",
          headers: { Authorization: "Bearer " + getToken(), "Content-Type": "application/json" },
          body: JSON.stringify({ action: "dismiss" }),
        });
      } catch (_) {}
    });
  });
}

async function openInboxPanel() {
  savedSessionId = currentSessionId;
  savedCardName = currentCardName;
  savedCardId = activeCardId;

  inboxPanelOpen = true;
  inboxActiveTab = "all";

  cleanupTerminal();

  const panel = document.querySelector(".split-panel");
  if (!panel) return;
  panel.innerHTML = '<div class="split-empty">Loading inbox\u2026</div>';

  try {
    const res = await timedFetch("/inbox", {
      headers: { Authorization: "Bearer " + getToken() },
    });
    if (res.ok) {
      const data = await res.json();
      inboxItems = Array.isArray(data) ? data : (data.items || []);
    }
  } catch (_) {}

  try {
    const res = await timedFetch("/dispatch", {
      headers: { Authorization: "Bearer " + getToken() },
    });
    if (res.ok) {
      const data = await res.json();
      const dispatchItems = Array.isArray(data) ? data : (data.items || []);
      for (const d of dispatchItems) {
        if (!inboxItems.find(i => i.filename === d.filename)) {
          d.type = "dispatch";
          d.subject = d.subject || d.title || "";
          d.from = d.from || (d.author && d.author.login) || "";
          inboxItems.push(d);
        }
      }
    }
  } catch (_) {}

  updateInboxBadge();
  renderInboxPanelContent();

  // Reset poll timer from now to avoid stale-fetch race
  stopInboxPoll();
  startInboxPoll();
}

function closeInboxPanel() {
  inboxPanelOpen = false;
  if (savedSessionId && savedCardId) {
    openSplitTerminal(savedSessionId, savedCardName, savedCardId);
  } else {
    const panel = document.querySelector(".split-panel");
    if (panel) panel.innerHTML = '<div class="split-empty">Select a session</div>';
  }
}

// Onboarding auth state
let ghAuthStatus = null;
let claudeAuthStatus = null;
let ghBannerDismissed = localStorage.getItem("ncc_gh_banner_dismissed") === "1";
let claudeBannerDismissed = localStorage.getItem("ncc_claude_banner_dismissed") === "1";

// Bootstrap state
let bootstrapState = null; // { state: "idle"|"running"|"complete"|"failed", last_lines: [...], completed_at: ... }

// --- Screens ---

function renderScanScreen(message) {
  app.innerHTML = `
    <div class="screen">
      <div class="logo">NX</div>
      <h1>NexusLink</h1>
      <p class="muted">${message || "Enter your access token to connect."}</p>
      <div style="margin-top:1.5rem;width:100%;max-width:360px">
        <input id="token-input" type="text" placeholder="Paste bootstrap token"
          style="width:100%;padding:0.75rem;border-radius:8px;border:1px solid #333;background:#1a1a2e;color:#e0e0e0;font-size:0.95rem;box-sizing:border-box" />
        <button id="token-submit"
          style="margin-top:0.75rem;width:100%;padding:0.75rem;border-radius:8px;border:none;background:#3B82F6;color:white;font-size:0.95rem;cursor:pointer">
          Connect
        </button>
      </div>
    </div>
  `;
  document.getElementById("token-submit").addEventListener("click", async () => {
    const token = document.getElementById("token-input").value.trim();
    if (!token) return;
    setToken(token);
    renderLoading();
    try {
      await fetchSessions();
      connectionState = "connected";
      startPolling();
    } catch (e) {
      clearToken();
      renderScanScreen("Invalid token — please try again.");
    }
  });
  document.getElementById("token-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") document.getElementById("token-submit").click();
  });
}

function renderErrorScreen(message) {
  app.innerHTML = `
    <div class="screen">
      <div class="logo">NX</div>
      <h1>NexusLink</h1>
      <p class="error-text">${message}</p>
    </div>
  `;
}

function renderLoading() {
  app.innerHTML = `
    <div class="screen">
      <div class="logo">NX</div>
      <p class="muted">Loading sessions...</p>
    </div>
  `;
}

function renderBoard(sessions) {
  stopInboxPoll();

  if (window.innerWidth >= 768) {
    renderSplitLayout(sessions);
    return;
  }

  app.classList.remove("split-active", "dashboard-active");
  currentView = "board";

  // Group by lane name, track color per lane, skip Archived
  const byLane = {};
  const laneColors = {};
  const seenOrder = [];
  for (const s of sessions) {
    if (s.lane_name === "Archived") continue;
    if (!byLane[s.lane_name]) {
      byLane[s.lane_name] = [];
      laneColors[s.lane_name] = s.lane_color || "#6B7280";
      seenOrder.push(s.lane_name);
    }
    byLane[s.lane_name].push(s);
  }

  // Build lane sections in encounter order (server returns sorted by sort_order)
  let lanesHtml = "";
  for (const lane of seenOrder) {
    const cards = byLane[lane];
    if (!cards || cards.length === 0) continue;
    const color = laneColors[lane];
    const cardsHtml = cards.map((c) => cardHtml(c)).join("");
    lanesHtml += `
      <div class="lane">
        <div class="lane-header">
          <span class="lane-badge" style="background: ${color}20; color: ${color}">${escapeHtml(lane)}</span>
          <span class="lane-count">${cards.length}</span>
        </div>
        ${cardsHtml}
      </div>
    `;
  }

  const fabHtml = `<button class="fab" id="new-btn">+</button>`;

  let bannersHtml = "";
  if (ghAuthStatus && !ghAuthStatus.authenticated && !ghBannerDismissed) {
    bannersHtml += `<div class="onboarding-banner" id="banner-gh">&#9888; GitHub not authenticated. Open a terminal and run: <code>gh auth login</code><button class="onboarding-dismiss" data-banner="gh">&#215;</button></div>`;
  }
  const hasAliveSessions = sessions && sessions.some(s => s.is_alive);
  if (claudeAuthStatus && !claudeAuthStatus.authenticated && !claudeBannerDismissed && !hasAliveSessions) {
    bannersHtml += `<div class="onboarding-banner" id="banner-claude">&#9888; Claude not authenticated. Open a terminal and run: <code>claude auth login</code><button class="onboarding-dismiss" data-banner="claude">&#215;</button></div>`;
  }

  const bootstrapHtml = bootstrapBadgeHtml();

  if (!lanesHtml) {
    app.innerHTML = `
      ${statusBarHtml()}
      ${bannersHtml}
      ${bootstrapHtml}
      <div class="screen">
        <p class="muted">No sessions yet.</p>
      </div>
      ${fabHtml}
    `;
    document.getElementById("new-btn").addEventListener("click", openWizard);
    document.querySelectorAll(".onboarding-dismiss").forEach((btn) => {
      btn.addEventListener("click", () => {
        if (btn.dataset.banner === "gh") { ghBannerDismissed = true; localStorage.setItem("ncc_gh_banner_dismissed", "1"); }
        if (btn.dataset.banner === "claude") { claudeBannerDismissed = true; localStorage.setItem("ncc_claude_banner_dismissed", "1"); }
        btn.closest(".onboarding-banner").remove();
      });
    });
    const settingsBtnEmpty = document.getElementById("settings-btn");
    if (settingsBtnEmpty) settingsBtnEmpty.addEventListener("click", renderSettings);
    const runBtnEmpty = document.getElementById("bootstrap-run-btn");
    if (runBtnEmpty) runBtnEmpty.addEventListener("click", handleBootstrapRun);
    attachViewSwitcherHandlers();
    return;
  }

  app.innerHTML = `
    ${statusBarHtml()}
    ${bannersHtml}
    ${bootstrapHtml}
    <div class="board">${lanesHtml}</div>
    ${fabHtml}
  `;

  // Attach click handlers — alive cards open terminal, dormant cards start session first
  document.querySelectorAll(".card-clickable").forEach((el) => {
    el.addEventListener("click", () => {
      const sid = el.dataset.sessionId;
      const name = el.dataset.cardName;
      if (sid) openTerminal(sid, name);
    });
  });
  document.querySelectorAll(".card-dormant").forEach((el) => {
    el.addEventListener("click", (e) => {
      // Don't trigger if action buttons were clicked
      if (e.target.closest(".claude-btn") || e.target.closest(".card-restart-btn") || e.target.closest(".card-file-btn")) return;
      const cardId = el.dataset.cardId;
      const name = el.dataset.cardName;
      if (cardId) startSessionAndOpen(cardId, name);
    });
  });

  // Claude buttons on dormant cards
  document.querySelectorAll(".claude-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const cardId = btn.dataset.cardId;
      const name = btn.dataset.cardName;
      if (cardId) startSessionAndOpen(cardId, name, claudeLaunchCommand);
    });
  });

  // Onboarding banner dismiss
  document.querySelectorAll(".onboarding-dismiss").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.banner === "gh") { ghBannerDismissed = true; localStorage.setItem("ncc_gh_banner_dismissed", "1"); }
      if (btn.dataset.banner === "claude") { claudeBannerDismissed = true; localStorage.setItem("ncc_claude_banner_dismissed", "1"); }
      btn.closest(".onboarding-banner").remove();
    });
  });

  // Stop buttons on alive cards
  document.querySelectorAll(".card-stop-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const cardId = btn.dataset.cardId;
      const token = getToken();
      try {
        const res = await timedFetch(`/cards/${cardId}/session`, {
          method: "DELETE",
          headers: { Authorization: `Bearer ${token}` },
        });
        if (res.ok) {
          const sessions = await fetchSessions();
          lastSessions = sessions;
          renderBoard(sessions);
        }
      } catch (err) { /* ignore — next poll will update */ }
    });
  });

  // Restart buttons on dormant cards
  document.querySelectorAll(".card-restart-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const cardId = btn.dataset.cardId;
      const name = btn.dataset.cardName;
      if (cardId) startSessionAndOpen(cardId, name);
    });
  });

  // File browser buttons
  document.querySelectorAll(".card-file-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      renderFileBrowser(btn.dataset.cardId);
    });
  });

  // Context menu on board cards
  document.querySelectorAll(".card").forEach((el) => {
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const cardData = findCardData(el.dataset.cardId);
      if (cardData) showContextMenu(e.clientX, e.clientY, cardData);
    });
  });

  // Settings button
  const settingsBtn = document.getElementById("settings-btn");
  if (settingsBtn) settingsBtn.addEventListener("click", renderSettings);
  attachViewSwitcherHandlers();

  // Bootstrap run button
  const runBtn = document.getElementById("bootstrap-run-btn");
  if (runBtn) runBtn.addEventListener("click", handleBootstrapRun);

  // FAB
  document.getElementById("new-btn").addEventListener("click", openWizard);
}

// Track when each session first became idle (keyed by card_id)
const idleSinceMap = {};

function formatElapsed(ms) {
  const mins = Math.floor(ms / 60000);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ${mins % 60}m`;
}

function cardHtml(card) {
  const dot = card.is_alive ? (card.is_idle ? "dot-idle" : "dot-alive") : "dot-dead";

  // Track idle timestamps
  if (card.is_idle) {
    if (!idleSinceMap[card.card_id]) idleSinceMap[card.card_id] = Date.now();
  } else {
    delete idleSinceMap[card.card_id];
  }

  // Show "Idle Xm" when idle, nothing when active (green dot is enough)
  let duration = "";
  if (card.is_alive && card.is_idle && idleSinceMap[card.card_id]) {
    duration = `<div class="card-duration card-duration-idle">Idle ${formatElapsed(Date.now() - idleSinceMap[card.card_id])}</div>`;
  }

  // Preview image for alive sessions with captured canvas, otherwise text preview
  const token = getToken();
  let previewHtml = "";
  if (card.is_alive && card.has_preview_image && card.session_id) {
    // Round cache-buster to 30s intervals so browser serves from cache across consecutive 5s polls
    const cacheBuster = Math.floor(Date.now() / 30000) * 30000;
    const imgUrl = `/sessions/${card.session_id}/preview?token=${encodeURIComponent(token)}&t=${cacheBuster}`;
    previewHtml = `<img class="card-preview-img" src="${imgUrl}" loading="lazy" alt="" />`;
  } else if (card.preview) {
    previewHtml = `<pre class="card-preview">${escapeHtml(card.preview)}</pre>`;
  }

  // All cards are clickable — alive ones open terminal, dormant ones start a session first
  const cls = card.is_alive && card.session_id ? "card card-clickable" : "card card-dormant";
  const dataAttrs = ` data-card-id="${escapeHtml(card.card_id)}" data-card-name="${escapeHtml(card.card_name)}"` +
    (card.session_id ? ` data-session-id="${escapeHtml(card.session_id)}"` : "");
  const isAlive = card.is_alive && card.session_id;
  const claudeBtn = !isAlive
    ? `<button class="claude-btn" data-card-id="${escapeHtml(card.card_id)}" data-card-name="${escapeHtml(card.card_name)}">Claude</button>`
    : "";
  const stopBtn = isAlive
    ? `<button class="card-stop-btn" data-card-id="${escapeHtml(card.card_id)}">&#9632;</button>`
    : "";
  const restartBtn = !isAlive
    ? `<button class="card-restart-btn" data-card-id="${escapeHtml(card.card_id)}" data-card-name="${escapeHtml(card.card_name)}">Restart</button>`
    : "";
  const fileBrowserBtn = (card.session_id || !card.is_alive)
    ? `<button class="card-file-btn" data-card-id="${escapeHtml(card.card_id)}" title="Browse files">&#128193;</button>`
    : "";

  return `
    <div class="${cls}"${dataAttrs}>
      <div class="card-header">
        <span class="${dot}"></span>
        <span class="card-name">${escapeHtml(card.card_name)}</span>
        ${fileBrowserBtn}${claudeBtn}${restartBtn}${stopBtn}
      </div>
      ${duration}
      ${previewHtml}
    </div>
  `;
}

function dashboardActivityInfo(card) {
  const state = card.claude_state || "";
  if (!card.is_alive) return { dot: "dot-dead", label: "Dormant", cls: "dash-state-dead" };
  if (card.is_idle) return { dot: "dot-idle", label: "Idle", cls: "dash-state-idle" };
  if (state) return { dot: "dot-alive", label: state.replace(/_/g, " "), cls: "dash-state-active" };
  return { dot: "dot-alive", label: "Active", cls: "dash-state-active" };
}

function dashboardContextHtml(card) {
  if (card.context_remaining == null) {
    return `<div class="dash-context dash-context-empty"><span>Context</span><strong>--</strong></div>`;
  }
  const remaining = Math.max(0, Math.min(100, Math.round(card.context_remaining)));
  const used = 100 - remaining;
  const colorClass = remaining < 15 ? "ctx-red" : remaining < 30 ? "ctx-orange" : remaining < 50 ? "ctx-yellow" : "ctx-green";
  return `
    <div class="dash-context" title="${remaining}% context remaining">
      <span>Context</span>
      <strong>${remaining}%</strong>
      <div class="dash-context-bar"><div class="dash-context-fill ${colorClass}" style="width:${used}%"></div></div>
    </div>
  `;
}

function dashboardPreviewHtml(card) {
  const token = getToken();
  if (card.is_alive && card.has_preview_image && card.session_id) {
    const cacheBuster = Math.floor(Date.now() / 30000) * 30000;
    const imgUrl = `/sessions/${card.session_id}/preview?token=${encodeURIComponent(token)}&t=${cacheBuster}`;
    return `<img class="dash-preview-img" src="${imgUrl}" loading="lazy" alt="" />`;
  }
  if (card.preview) return `<pre class="dash-preview">${escapeHtml(stripAnsi(card.preview))}</pre>`;
  return `<pre class="dash-preview dash-preview-empty">No preview yet.</pre>`;
}

function dashboardFallbackPreviewText(card) {
  const text = stripAnsi(card.preview || "");
  return text || "No preview yet.";
}

function dashboardCardById(cardId) {
  return Array.from(document.querySelectorAll(".dash-card"))
    .find((el) => el.dataset.cardId === String(cardId));
}

function shouldFlashDashboardCard(card) {
  const key = card.card_id || card.session_id || card.card_name;
  const snapshot = encodeURIComponent(JSON.stringify({
    preview: card.preview || "",
    state: card.claude_state || "",
    alive: !!card.is_alive,
    idle: !!card.is_idle,
  }));
  const previous = dashboardSnapshots.get(key);
  dashboardSnapshots.set(key, snapshot);
  return previous != null && previous !== snapshot;
}

function dashboardCardHtml(card) {
  const activity = dashboardActivityInfo(card);
  const laneColor = card.lane_color || "#6B7280";
  const flashCls = shouldFlashDashboardCard(card) ? " dash-flash" : "";
  const dormantCls = card.is_alive ? "" : " dash-card-dormant";
  return `
    <article class="dash-card${flashCls}${dormantCls}" data-card-id="${escapeHtml(card.card_id)}">
      <div class="dash-card-head">
        <span class="${activity.dot}"></span>
        <div class="dash-card-title">
          <h3>${escapeHtml(card.card_name)}</h3>
          <span class="dash-lane" style="background:${laneColor}20;color:${laneColor}">${escapeHtml(card.lane_name || "No lane")}</span>
        </div>
        <button class="dash-menu-btn" data-card-id="${escapeHtml(card.card_id)}" aria-label="Card actions">&#8942;</button>
      </div>
      <div class="dash-meta">
        <span class="dash-state ${activity.cls}">${escapeHtml(activity.label)}</span>
        ${dashboardContextHtml(card)}
      </div>
      <div class="dash-preview-wrap">${dashboardPreviewHtml(card)}</div>
    </article>
  `;
}

async function fetchDashboardScreen(sessionId) {
  const token = getToken();
  const res = await timedFetch(`/sessions/${encodeURIComponent(sessionId)}/screen`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

function ensureDashboardPreviewPre(wrap, card) {
  let pre = wrap.querySelector(".dash-preview");
  if (!pre || wrap.querySelector(".dash-preview-img")) {
    wrap.innerHTML = `<pre class="dash-preview"></pre>`;
    pre = wrap.querySelector(".dash-preview");
  }
  pre.classList.toggle("dash-preview-empty", !card.preview && !dashboardScreenCache.has(card.session_id));
  pre.dataset.sessionId = card.session_id || "";
  return pre;
}

// Scale the monospace screen so the real cols×rows grid fits the frame both ways, centered
// (letterboxed by the frame's dark bg) — preserves the terminal's true layout instead of
// wrapping/clipping at a fixed font size.
function sizeTerminalPreview(pre, wrap, cols, rows) {
  if (!pre || !wrap || !cols || !rows) return;
  const w = wrap.clientWidth, h = wrap.clientHeight;
  if (!w || !h) return;
  const CHAR_RATIO = 0.62, LINE = 1.08;   // monospace advance ≈ 0.62em; line-height 1.08
  const fs = Math.max(3, Math.min(w / (cols * CHAR_RATIO), h / (rows * LINE)));
  pre.style.fontSize = fs.toFixed(2) + "px";
  pre.style.lineHeight = String(LINE);
  pre.classList.add("dash-preview-term");
}

function clearTerminalPreviewSizing(pre) {
  if (!pre) return;
  pre.style.fontSize = "";
  pre.style.lineHeight = "";
  pre.classList.remove("dash-preview-term");
}

function updateDashboardPreview(card, wrap) {
  if (!wrap) return;

  if (card.is_alive && card.has_preview_image && card.session_id) {
    wrap.innerHTML = dashboardPreviewHtml(card);
    return;
  }

  const pre = ensureDashboardPreviewPre(wrap, card);
  const cached = card.session_id ? dashboardScreenCache.get(card.session_id) : null;
  if (cached && cached.screen) {
    pre.textContent = cached.screen;
    pre.classList.remove("dash-preview-empty");
    sizeTerminalPreview(pre, wrap, cached.cols, cached.rows);
  } else {
    pre.textContent = dashboardFallbackPreviewText(card);
    pre.classList.toggle("dash-preview-empty", !pre.textContent || pre.textContent === "No preview yet.");
    clearTerminalPreviewSizing(pre);
  }

  if (!card.is_alive || !card.session_id) return;

  fetchDashboardScreen(card.session_id)
    .then((data) => {
      const screen = String(data?.screen || "");
      if (!screen.trim()) return;
      const cols = Number(data?.cols) || 80, rows = Number(data?.rows) || 24;
      dashboardScreenCache.set(card.session_id, { screen, cols, rows });
      const current = dashboardCardById(card.card_id);
      const currentPre = current?.querySelector(".dash-preview");
      if (!currentPre || currentPre.dataset.sessionId !== String(card.session_id)) return;
      const currentWrap = currentPre.closest(".dash-preview-wrap");
      currentPre.textContent = screen;
      currentPre.classList.remove("dash-preview-empty");
      sizeTerminalPreview(currentPre, currentWrap, cols, rows);
    })
    .catch(() => {
      // Keep the card preview fallback on auth, 404, or transient screen fetch failures.
    });
}

function updateDashboardCardNode(el, card) {
  const activity = dashboardActivityInfo(card);
  const flash = shouldFlashDashboardCard(card);
  el.classList.toggle("dash-card-dormant", !card.is_alive);
  if (flash) {
    el.classList.remove("dash-flash");
    void el.offsetWidth;
    el.classList.add("dash-flash");
  }

  const dot = el.querySelector(".dash-card-head > span");
  if (dot) dot.className = activity.dot;

  const title = el.querySelector(".dash-card-title h3");
  if (title) title.textContent = card.card_name || "";

  const lane = el.querySelector(".dash-lane");
  if (lane) {
    const laneColor = card.lane_color || "#6B7280";
    lane.textContent = card.lane_name || "No lane";
    lane.style.background = `${laneColor}20`;
    lane.style.color = laneColor;
  }

  const meta = el.querySelector(".dash-meta");
  if (meta) {
    meta.innerHTML = `
      <span class="dash-state ${activity.cls}">${escapeHtml(activity.label)}</span>
      ${dashboardContextHtml(card)}
    `;
  }

  updateDashboardPreview(card, el.querySelector(".dash-preview-wrap"));
}

function attachDashboardCardHandlers(el) {
  el.addEventListener("click", (e) => {
    if (e.target.closest(".dash-menu-btn")) return;
    const card = (lastSessions || []).find((s) => s.card_id === el.dataset.cardId);
    if (card) openDashboardCard(card);
  });
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const cardData = findCardData(el.dataset.cardId);
    if (cardData) showContextMenu(e.clientX, e.clientY, cardData);
  });
  const btn = el.querySelector(".dash-menu-btn");
  if (btn) {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const cardData = findCardData(btn.dataset.cardId);
      if (cardData) showContextMenu(e.clientX, e.clientY, cardData);
    });
  }
}

function updateDashboard(sessions) {
  if (!document.querySelector(".dashboard-shell")) {
    renderDashboard(sessions);
    return;
  }

  const visibleSessions = (sessions || []).filter((s) => s.lane_name !== "Archived");
  const { dotClass, label } = connectionStatusInfo();
  const headerDot = document.querySelector(".dashboard-title .status-dot");
  const headerText = document.querySelector(".dashboard-title p");
  if (headerDot) headerDot.className = `status-dot ${dotClass}`;
  if (headerText) headerText.textContent = `${label} · ${visibleSessions.length} cards`;

  let grid = document.querySelector(".dashboard-grid");
  const empty = document.querySelector(".dashboard-empty");

  if (!visibleSessions.length) {
    if (grid) {
      const main = document.createElement("main");
      main.className = "dashboard-empty";
      main.innerHTML = `<p class="muted">No sessions yet.</p><button class="btn-primary" id="dash-new-btn">New Session</button>`;
      grid.replaceWith(main);
      const newBtn = document.getElementById("dash-new-btn");
      if (newBtn) newBtn.addEventListener("click", openWizard);
    }
    return;
  }

  if (!grid) {
    grid = document.createElement("main");
    grid.className = "dashboard-grid";
    if (empty) empty.replaceWith(grid);
    else document.querySelector(".dashboard-shell")?.insertBefore(grid, document.querySelector(".dash-actions"));
  }

  const visibleIds = new Set(visibleSessions.map((card) => String(card.card_id)));
  Array.from(grid.querySelectorAll(".dash-card")).forEach((el) => {
    if (!visibleIds.has(el.dataset.cardId)) el.remove();
  });

  visibleSessions.forEach((card) => {
    let el = dashboardCardById(card.card_id);
    if (!el) {
      grid.insertAdjacentHTML("beforeend", dashboardCardHtml(card));
      el = dashboardCardById(card.card_id);
      if (el) {
        attachDashboardCardHandlers(el);
        updateDashboardPreview(card, el.querySelector(".dash-preview-wrap"));
      }
      return;
    }
    updateDashboardCardNode(el, card);
  });
}

function renderDashboard(sessions) {
  stopInboxPoll();
  app.classList.remove("split-active");
  app.classList.add("dashboard-active");
  currentView = "dashboard";

  const visibleSessions = (sessions || []).filter((s) => s.lane_name !== "Archived");
  const { dotClass, label } = connectionStatusInfo();
  app.innerHTML = `
    <div class="dashboard-shell">
      <header class="dashboard-header">
        <div class="dashboard-title">
          <span class="status-dot ${dotClass}"></span>
          <div>
            <h1 class="ncc-name">${escapeHtml(nccName)}</h1>
            <p>${escapeHtml(label)} · ${visibleSessions.length} cards</p>
          </div>
        </div>
        ${viewSwitcherHtml("dashboard")}
        <button class="settings-btn" id="settings-btn" aria-label="Settings" title="Settings & ‘How this works’ legend">&#9881;</button>
      </header>
      ${visibleSessions.length
        ? `<main class="dashboard-grid">${visibleSessions.map((card) => dashboardCardHtml(card)).join("")}</main>`
        : `<main class="dashboard-empty"><p class="muted">No sessions yet.</p><button class="btn-primary" id="dash-new-btn">New Session</button></main>`}
      ${dashboardActionsPanelHtml(visibleSessions)}
    </div>
  `;

  attachViewSwitcherHandlers();
  attachDashboardHandlers(visibleSessions);
  visibleSessions.forEach((card) => {
    const el = dashboardCardById(card.card_id);
    if (el) updateDashboardPreview(card, el.querySelector(".dash-preview-wrap"));
  });

  const settingsBtn = document.getElementById("settings-btn");
  if (settingsBtn) settingsBtn.addEventListener("click", renderSettings);
  const newBtn = document.getElementById("dash-new-btn");
  if (newBtn) newBtn.addEventListener("click", openWizard);
}

function dashboardActionsPanelHtml(sessions) {
  const options = sessions.map((card) => `
    <option value="${escapeHtml(card.card_id)}">${escapeHtml(card.card_name)}</option>
  `).join("");
  return `
    <details class="dash-actions" open>
      <summary>Experimental Actions</summary>
      <div class="dash-actions-body">
        <label class="dash-action-label" for="dash-action-session">Session</label>
        <select class="dash-action-input" id="dash-action-session">${options}</select>
        <textarea class="dash-action-input dash-action-message" id="dash-action-message" rows="3" placeholder="Message to send"></textarea>
        <div class="dash-action-row">
          <button class="dash-action-btn" id="dash-send-btn">Send</button>
          <button class="dash-action-btn" id="dash-clear-btn">/clear</button>
        </div>
        <div class="dash-action-row">
          <button class="dash-action-btn" id="dash-wake-on-btn">Wake On</button>
          <button class="dash-action-btn" id="dash-wake-off-btn">Wake Off</button>
        </div>
        <div class="dash-action-row">
          <select class="dash-action-input" id="dash-relay-mode">
            <option value="auto">Relay auto</option>
            <option value="manual">Relay manual</option>
          </select>
          <button class="dash-action-btn" id="dash-relay-btn">Set</button>
        </div>
        <div class="dash-action-row">
          <button class="dash-action-btn" id="dash-winddown-on-btn">Winddown On</button>
          <button class="dash-action-btn" id="dash-winddown-off-btn">Winddown Off</button>
        </div>
        <div class="dash-config-row">
          <input class="dash-action-input" id="dash-wd-stage1" type="number" min="1" max="100" value="35" title="Stage 1 percent" />
          <input class="dash-action-input" id="dash-wd-stage2" type="number" min="1" max="100" value="30" title="Stage 2 percent" />
          <input class="dash-action-input" id="dash-wd-clear" type="number" min="1" max="100" value="20" title="Force clear percent" />
          <button class="dash-action-btn" id="dash-winddown-config-btn">Config</button>
        </div>
        <div class="dash-action-status" id="dash-action-status" aria-live="polite"></div>
      </div>
    </details>
  `;
}

function selectedDashboardCard() {
  const select = document.getElementById("dash-action-session");
  const cardId = select?.value;
  return (lastSessions || []).find((card) => card.card_id === cardId) || null;
}

function setDashActionStatus(message, isError = false) {
  const status = document.getElementById("dash-action-status");
  if (!status) return;
  status.textContent = message;
  status.classList.toggle("error", !!isError);
}

async function postDashboardAction(endpoint, body) {
  const token = getToken();
  const opts = {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  };
  if (body !== undefined) {
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  const res = await timedFetch(endpoint, opts);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(data?.error?.message || data?.error || data?.message || `HTTP ${res.status}`);
  }
  return data;
}

async function refreshDashboardSessions() {
  const sessions = await fetchSessions();
  lastSessions = sessions;
  if (currentView === "dashboard") updateDashboard(sessions);
  return sessions;
}

function attachDashboardHandlers(sessions) {
  document.querySelectorAll(".dash-card").forEach((el) => {
    attachDashboardCardHandlers(el);
  });

  const sendBtn = document.getElementById("dash-send-btn");
  if (sendBtn) sendBtn.addEventListener("click", async () => {
    const card = selectedDashboardCard();
    const message = document.getElementById("dash-action-message")?.value || "";
    if (!card || !message.trim()) return setDashActionStatus("Choose a session and enter a message.", true);
    try {
      await postDashboardAction(`/cards/${card.card_id}/send`, { message, force: true });
      await refreshDashboardSessions();
      setDashActionStatus("Message sent.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const clearBtn = document.getElementById("dash-clear-btn");
  if (clearBtn) clearBtn.addEventListener("click", async () => {
    const card = selectedDashboardCard();
    if (!card) return setDashActionStatus("Choose a session.", true);
    try {
      await postDashboardAction(`/cards/${card.card_id}/send`, { message: "/clear", force: true });
      await refreshDashboardSessions();
      setDashActionStatus("Clear sent.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const wakeOnBtn = document.getElementById("dash-wake-on-btn");
  if (wakeOnBtn) wakeOnBtn.addEventListener("click", async () => {
    try {
      window._nccWakeStatus = await postDashboardAction("/wake/enable");
      setDashActionStatus("Wake enabled.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const wakeOffBtn = document.getElementById("dash-wake-off-btn");
  if (wakeOffBtn) wakeOffBtn.addEventListener("click", async () => {
    try {
      window._nccWakeStatus = await postDashboardAction("/wake/disable");
      setDashActionStatus("Wake disabled.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const relayBtn = document.getElementById("dash-relay-btn");
  if (relayBtn) relayBtn.addEventListener("click", async () => {
    const card = selectedDashboardCard();
    const mode = document.getElementById("dash-relay-mode")?.value || "manual";
    if (!card?.workspace_path) return setDashActionStatus("Selected session has no workspace path.", true);
    try {
      await postDashboardAction(`/wake/relay/agents/${encodeURIComponent(card.workspace_path)}/mode`, { mode });
      await refreshDashboardSessions();
      setDashActionStatus(`Relay set to ${mode}.`);
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const winddownOnBtn = document.getElementById("dash-winddown-on-btn");
  if (winddownOnBtn) winddownOnBtn.addEventListener("click", async () => {
    try {
      await postDashboardAction("/winddown/enable");
      setDashActionStatus("Winddown enabled.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const winddownOffBtn = document.getElementById("dash-winddown-off-btn");
  if (winddownOffBtn) winddownOffBtn.addEventListener("click", async () => {
    try {
      await postDashboardAction("/winddown/disable");
      setDashActionStatus("Winddown disabled.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });

  const winddownConfigBtn = document.getElementById("dash-winddown-config-btn");
  if (winddownConfigBtn) winddownConfigBtn.addEventListener("click", async () => {
    const num = (id, fallback) => Number(document.getElementById(id)?.value || fallback);
    const body = {
      stage1_pct: num("dash-wd-stage1", 35),
      stage2_pct: num("dash-wd-stage2", 30),
      force_clear_pct: num("dash-wd-clear", 20),
      idle_before_stage1_secs: 600,
      stage1_to_stage2_secs: 300,
      max_clears_per_hour: 2,
    };
    try {
      await postDashboardAction("/winddown/config", body);
      setDashActionStatus("Winddown config applied.");
    } catch (err) {
      setDashActionStatus(err.message, true);
    }
  });
}

function openDashboardCard(card) {
  cameFromDashboard = true;
  currentView = "split";
  activeCardId = null;
  currentSessionId = null;
  currentCardName = null;
  renderSplitLayout(lastSessions || []);

  if (card.is_alive && card.session_id) {
    openSplitTerminal(card.session_id, card.card_name, card.card_id);
  } else {
    startSessionAndOpenSplit(card.card_id, card.card_name);
  }
}

function statusBarHtml() {
  let dotClass, label;
  if (connectionState === "connected") {
    dotClass = "status-connected";
    label = "Connected";
  } else if (connectionState === "unreachable") {
    dotClass = "status-unreachable";
    label = "Mac unreachable — check Tailscale";
  } else {
    dotClass = "status-offline";
    label = "No network connection";
  }
  return `
    <div class="status-bar">
      <span class="ncc-name" title="NCC name">${escapeHtml(nccName)}</span>
      <span class="status-dot ${dotClass}"></span>
      <span class="status-label">${label}</span>
      ${viewSwitcherHtml(currentView === "dashboard" ? "dashboard" : "board")}
      <button class="settings-btn" id="settings-btn" aria-label="Settings">&#9881;</button>
    </div>
  `;
}

function bootstrapBadgeHtml() {
  const st = bootstrapState ? bootstrapState.state : "idle";
  const showBtn = (st === "idle" || st === "failed");
  const btnHtml = showBtn
    ? `<button class="bootstrap-btn" id="bootstrap-run-btn">Run Bootstrap</button>`
    : "";
  return `<div class="bootstrap-section"><span class="bootstrap-badge bootstrap-${escapeHtml(st)}">Bootstrap: ${escapeHtml(st)}</span>${btnHtml}</div>`;
}

async function handleBootstrapRun() {
  try {
    const result = await triggerBootstrap();
    if (result.status === 202 || result.status === 409) {
      bootstrapState = { state: "running" };
      if (lastSessions) {
        refreshCurrentSessionView(lastSessions);
      }
      // Poll bootstrap status until complete/failed
      const pollBootstrap = async () => {
        try {
          const bs = await fetchBootstrapStatus();
          bootstrapState = bs;
          const bsSection = document.querySelector(".bootstrap-section");
          if (bsSection) {
            bsSection.outerHTML = bootstrapBadgeHtml();
            const runBtn = document.getElementById("bootstrap-run-btn");
            if (runBtn) runBtn.addEventListener("click", handleBootstrapRun);
          }
          if (bs.state === "running") {
            setTimeout(pollBootstrap, 2000);
          }
        } catch (_) {
          setTimeout(pollBootstrap, 2000);
        }
      };
      setTimeout(pollBootstrap, 2000);
    }
  } catch (e) {
    // ignore — next poll will reflect state
  }
}

// --- Split Layout Helpers ---

function emptyPanelHtml(sessions) {
  const hasSessions = sessions && sessions.length > 0;
  if (hasSessions) return '<div class="split-empty">Select a session</div>';

  // First-time user onboarding
  const ghOk = ghAuthStatus?.authenticated;
  const claudeOk = claudeAuthStatus?.loggedIn || claudeAuthStatus?.authenticated;
  const bsOk = bootstrapState?.state === "complete";

  return `<div class="onboarding-panel">
    <h2>Welcome to Nexus Command Center</h2>
    <p class="onboarding-subtitle">Your cloud workspace for managing Claude Code sessions.</p>

    <div class="onboarding-steps">
      <div class="onboarding-step ${ghOk ? 'step-done' : 'step-todo'}">
        <span class="step-icon">${ghOk ? '&#9745;' : '&#9744;'}</span>
        <div>
          <strong>1. Set up GitHub</strong>
          <p>Click <strong>+ New</strong> in the sidebar, create a session with any name, then open it.<br>
          In the terminal, run: <code>gh auth login</code><br>
          Choose <strong>GitHub.com</strong> → <strong>HTTPS</strong> → <strong>Login with a web browser</strong>.<br>
          Copy the one-time code, open the URL it gives you, and paste it in your browser.</p>
        </div>
      </div>

      <div class="onboarding-step ${claudeOk ? 'step-done' : 'step-todo'}">
        <span class="step-icon">${claudeOk ? '&#9745;' : '&#9744;'}</span>
        <div>
          <strong>2. Set up Claude Code</strong>
          <p>In the terminal, just run: <code>claude</code><br>
          If not logged in, it will prompt you to authenticate — follow the link it gives you and sign in with your Anthropic account.<br>
          This uses your personal Claude credits. After auth, type <code>/exit</code> to leave Claude, or keep using it.</p>
        </div>
      </div>

      <div class="onboarding-step ${bsOk ? 'step-done' : 'step-todo'}">
        <span class="step-icon">${bsOk ? '&#9745;' : '&#9744;'}</span>
        <div>
          <strong>3. Run Bootstrap</strong>
          <p>Click <strong>Run Bootstrap</strong> in the sidebar to install org tools:
          skills (dispatch, conductor, lens, etc.), repos (priorities, announcements),
          and configure your git identity. This makes every session org-aware.</p>
        </div>
      </div>
    </div>

    <div class="onboarding-ready">
      <p>After setup, create sessions to start working. Each session is a full terminal
      with Claude Code, connected to a workspace directory.</p>
    </div>
  </div>`;
}

function connectionStatusInfo() {
  let dotClass, label;
  if (connectionState === "connected") {
    dotClass = "status-connected";
    label = "Connected";
  } else if (connectionState === "unreachable") {
    dotClass = "status-unreachable";
    label = "Mac unreachable — check Tailscale";
  } else {
    dotClass = "status-offline";
    label = "No network connection";
  }
  return { dotClass, label };
}

function isOrchestrator(card) {
  return card.notes && card.notes.includes("#orchestrator");
}

function sidebarEntryHtml(card) {
  const cs = card.claude_state;
  const orch = isOrchestrator(card);
  const dot = !card.is_alive ? "dot-dead"
    : card.is_idle ? "dot-idle"
    : (cs === "thinking" || cs === "working" || cs === "running_command" || cs === "reading_code" || cs === "writing_code" || cs === "spawning_agents") ? "dot-alive"
    : cs === "waiting_for_approval" ? "dot-idle"
    : "dot-alive";
  const activeCls = (card.card_id === activeCardId) ? " sidebar-entry-active" : "";
  const orchCls = orch ? " sidebar-entry-orchestrator" : "";
  const stateLabel = cs && card.is_alive && !card.is_idle ? cs.replace(/_/g, " ") : "";
  const activityLabel = card.current_activity || stateLabel;
  const orchBadge = orch ? `<span class="sidebar-orch-badge">orch</span>` : "";
  const idleBadge = card.is_alive && card.is_idle ? `<span class="sidebar-idle-badge">Idle</span>`
    : activityLabel ? `<span class="sidebar-state-badge">${activityLabel}</span>`
    : "";
  const relayCount = card.relay_pending_count ?? 0;
  const relayBadge = relayCount > 0 ? `<span class="relay-badge">${relayCount}</span>` : "";
  const relayMode = card.relay_mode ?? null;
  const relayEnabled = card.relay_enabled ?? false;
  const relayIcon = relayEnabled && relayMode
    ? `<span class="relay-mode-icon" data-workspace="${escapeHtml(card.workspace_path || "")}" data-mode="${relayMode}" title="Relay ${relayMode === "auto" ? "Auto (\uD83D\uDCE1) \u2014 messages are delivered to this agent automatically when it's idle. Click to switch to Manual." : "Manual (\u23F8) \u2014 messages are held until you deliver them. Click to switch to Auto."}">${relayMode === "auto" ? "\u{1F4E1}" : "\u23F8"}</span>`
    : "";
  const sid = card.session_id || "";
  const resumeBtn = !card.is_alive ? `<button class="sidebar-resume-btn" data-card-id="${escapeHtml(card.card_id)}" data-card-name="${escapeHtml(card.card_name)}" title="Resume session">&#8635;</button>` : "";

  // Context bar — filled proportional to usage, color-coded
  let ctxBar = "";
  if (card.is_alive && card.context_remaining != null) {
    const used = Math.max(0, Math.min(100, 100 - Math.round(card.context_remaining)));
    const colorClass = used < 50 ? "ctx-green" : used < 70 ? "ctx-yellow" : used < 85 ? "ctx-orange" : "ctx-red";
    ctxBar = `<div class="sidebar-ctx-bar"><div class="sidebar-ctx-fill ${colorClass}" style="width:${used}%" title="Context: ${used}% used"></div></div>`;
  }

  return `
    <div class="sidebar-session-entry${activeCls}${orchCls}" data-card-id="${escapeHtml(card.card_id)}" data-session-id="${escapeHtml(sid)}" data-card-name="${escapeHtml(card.card_name)}" data-alive="${card.is_alive ? "1" : "0"}">
      <span class="${dot}"></span>
      ${orchBadge}
      ${relayIcon}
      <span class="sidebar-entry-name">${escapeHtml(card.card_name)}</span>
      ${idleBadge}
      ${relayBadge}
      ${resumeBtn}
      ${ctxBar}
    </div>
  `;
}

function sortSessions(sessions, mode) {
  const sorted = [...sessions];
  // Orchestrators always first
  sorted.sort((a, b) => (isOrchestrator(b) ? 1 : 0) - (isOrchestrator(a) ? 1 : 0));
  if (mode === "recent") {
    // Active > idle > dead, then by activity recency
    sorted.sort((a, b) => {
      // Orchestrators stay on top
      const orchA = isOrchestrator(a) ? 1 : 0;
      const orchB = isOrchestrator(b) ? 1 : 0;
      if (orchA !== orchB) return orchB - orchA;
      // Active sessions first
      const aliveA = a.is_alive ? (a.is_idle ? 1 : 2) : 0;
      const aliveB = b.is_alive ? (b.is_idle ? 1 : 2) : 0;
      if (aliveA !== aliveB) return aliveB - aliveA;
      // Then by started_at descending (most recent first)
      const timeA = a.started_at || "";
      const timeB = b.started_at || "";
      return timeB.localeCompare(timeA);
    });
  }
  return sorted;
}

function sidebarLaneHtml(laneName, laneColor, sessions) {
  const collapsed = sidebarCollapsedLanes.has(laneName);
  const chevronCls = collapsed ? "sidebar-lane-chevron collapsed" : "sidebar-lane-chevron";
  const sortMode = getLaneSortMode(laneName);
  const sorted = sortSessions(sessions, sortMode);
  const sortIcon = sortMode === "recent" ? "&#8645;" : "&#8693;";
  const sortTitle = sortMode === "recent" ? "Sorted by recent activity (click for default)" : "Default sort order (click for recent activity)";
  const sortCls = sortMode === "recent" ? "lane-sort-btn active" : "lane-sort-btn";
  const entriesHtml = collapsed ? "" : sorted.map(s => sidebarEntryHtml(s)).join("");
  return `
    <div class="sidebar-lane" data-lane="${escapeHtml(laneName)}">
      <div class="sidebar-lane-header" data-lane="${escapeHtml(laneName)}">
        <span class="${chevronCls}">&#9654;</span>
        <span class="sidebar-lane-name" style="background: ${laneColor}20; color: ${laneColor}">${escapeHtml(laneName)}</span>
        <span class="sidebar-lane-count">${sessions.length}</span>
        <button class="${sortCls}" data-lane-sort="${escapeHtml(laneName)}" title="${sortTitle}">${sortIcon}</button>
      </div>
      ${entriesHtml}
    </div>
  `;
}

function buildSidebarLanesHtml(sessions) {
  const byLane = {};
  const laneColors = {};
  const seenOrder = [];
  for (const s of sessions) {
    if (s.lane_name === "Archived") continue;
    if (!byLane[s.lane_name]) {
      byLane[s.lane_name] = [];
      laneColors[s.lane_name] = s.lane_color || "#6B7280";
      seenOrder.push(s.lane_name);
    }
    byLane[s.lane_name].push(s);
  }
  let lanesHtml = "";
  for (const lane of seenOrder) {
    const cards = byLane[lane];
    if (!cards || cards.length === 0) continue;
    lanesHtml += sidebarLaneHtml(lane, laneColors[lane], cards);
  }
  return { lanesHtml, seenOrder, byLane, laneColors };
}

// --- Split Layout ---

function attachSidebarHandlers() {
  document.querySelectorAll(".sidebar-session-entry").forEach(el => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const cardId = el.dataset.cardId;
      const sid = el.dataset.sessionId;
      const name = el.dataset.cardName;
      const alive = el.dataset.alive === "1";
      if (alive && sid) {
        openSplitTerminal(sid, name, cardId);
      } else if (cardId) {
        startSessionAndOpenSplit(cardId, name);
      }
    });
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const cardData = findCardData(el.dataset.cardId);
      if (cardData) showContextMenu(e.clientX, e.clientY, cardData);
    });
  });

  // Resume buttons on dead sessions
  document.querySelectorAll(".sidebar-resume-btn").forEach(btn => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      e.preventDefault();
      const cardId = btn.dataset.cardId;
      const cardName = btn.dataset.cardName;
      btn.disabled = true;
      btn.textContent = "…";
      const token = getToken();
      try {
        const panel = document.querySelector(".split-panel");
        if (panel) panel.innerHTML = `<div class="split-empty">Resuming session...</div>`;
        const res = await timedFetch(`/cards/${cardId}/session`, {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
          body: JSON.stringify({ resume: true, cols: Math.floor((panel?.clientWidth || 800) / 9), rows: Math.floor((panel?.clientHeight || 600) / 17) }),
        });
        if (res.ok) {
          const data = await res.json();
          if (data.session_id) openSplitTerminal(data.session_id, cardName, cardId);
        }
      } catch (_) {}
      // Button disappears on next sidebar refresh when session becomes alive
    });
  });

  document.querySelectorAll(".relay-mode-icon").forEach(icon => {
    icon.addEventListener("click", async (e) => {
      e.stopPropagation();
      e.preventDefault();
      const workspace = icon.dataset.workspace;
      const currentMode = icon.dataset.mode;
      if (!workspace) return;
      const newMode = currentMode === "auto" ? "manual" : "auto";
      const token = getToken();
      try {
        await timedFetch(`/wake/relay/agents/${encodeURIComponent(workspace)}/mode`, {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
          body: JSON.stringify({ mode: newMode }),
        });
        const sessions = await fetchSessions();
        lastSessions = sessions;
        updateSidebar(sessions);
      } catch (err) { /* next poll catches it */ }
    });
  });

  document.querySelectorAll(".sidebar-lane-header").forEach(el => {
    el.addEventListener("click", (e) => {
      if (e.target.closest(".lane-sort-btn")) return; // don't collapse on sort click
      const lane = el.dataset.lane;
      if (sidebarCollapsedLanes.has(lane)) {
        sidebarCollapsedLanes.delete(lane);
      } else {
        sidebarCollapsedLanes.add(lane);
      }
      updateSidebar(lastSessions);
    });
  });

  // Per-lane sort toggle
  document.querySelectorAll(".lane-sort-btn").forEach(btn => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const lane = btn.dataset.laneSort;
      if (lane) {
        toggleLaneSortMode(lane);
        updateSidebar(lastSessions);
      }
    });
  });

  const newBtn = document.getElementById("sidebar-new-btn");
  if (newBtn) newBtn.addEventListener("click", openWizard);

  const settingsBtn = document.getElementById("sidebar-settings-btn");
  if (settingsBtn) settingsBtn.addEventListener("click", renderSettings);

  const inboxBtn = document.getElementById("sidebar-inbox-btn");
  if (inboxBtn) inboxBtn.onclick = openInboxPanel;

  const wakeLabel = document.getElementById("wake-label");
  if (wakeLabel) {
    wakeLabel.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleWakePopover();
    });
  }
}

// --- Context Menu ---

function closeContextMenu() {
  const existing = document.getElementById("ctx-menu");
  if (existing) existing.remove();
  uiModalOpen = false;
}

function showContextMenu(x, y, card) {
  closeContextMenu();
  uiModalOpen = true;

  const menu = document.createElement("div");
  menu.id = "ctx-menu";

  // Build lane map from lastSessions, deduplicating by lane_id
  const laneMap = new Map();
  if (lastSessions) {
    for (const s of lastSessions) {
      if (s.lane_id && !laneMap.has(s.lane_id)) {
        laneMap.set(s.lane_id, { name: s.lane_name, color: s.lane_color || "#888" });
      }
    }
  }

  // Helper to create menu item
  function makeItem(label, cls, onClick) {
    const el = document.createElement("div");
    el.className = "ctx-item" + (cls ? " " + cls : "");
    el.innerHTML = label;
    el.addEventListener("mousedown", (e) => { e.stopPropagation(); });
    el.addEventListener("click", (e) => { e.stopPropagation(); onClick(e); });
    return el;
  }

  function makeSeparator() {
    const el = document.createElement("div");
    el.className = "ctx-separator";
    return el;
  }

  // Open terminal
  menu.appendChild(makeItem("Open terminal", "", () => {
    closeContextMenu();
    if (card.is_alive && card.session_id) {
      if (currentView === "split") openSplitTerminal(card.session_id, card.card_name, card.card_id);
      else openTerminal(card.session_id, card.card_name);
    } else {
      if (currentView === "split") startSessionAndOpenSplit(card.card_id, card.card_name);
      else startSessionAndOpen(card.card_id, card.card_name);
    }
  }));

  // Edit card
  menu.appendChild(makeItem("Edit card\u2026", "", () => {
    closeContextMenu();
    showEditModal(card);
  }));

  // Move to lane submenu
  const otherLanes = [...laneMap.entries()].filter(([id]) => id !== card.lane_id);
  if (otherLanes.length > 0) {
    const submenuContainer = document.createElement("div");
    submenuContainer.className = "ctx-item ctx-submenu-container";
    submenuContainer.innerHTML = "Move to lane \u25B6";

    const submenu = document.createElement("div");
    submenu.className = "ctx-submenu";
    submenu.style.cssText = "position:absolute;left:100%;top:0;background:var(--nx-surface);border:1px solid var(--nx-border);border-radius:8px;padding:4px 0;min-width:160px;box-shadow:0 8px 24px rgba(0,0,0,0.5);z-index:10000;display:none;";

    for (const [laneId, laneInfo] of otherLanes) {
      const item = document.createElement("div");
      item.className = "ctx-item";
      item.innerHTML = `<span class="ctx-lane-dot" style="background:${escapeHtml(laneInfo.color)}"></span>${escapeHtml(laneInfo.name)}`;
      item.addEventListener("mousedown", (e) => e.stopPropagation());
      item.addEventListener("click", async (e) => {
        e.stopPropagation();
        closeContextMenu();
        const token = getToken();
        try {
          await timedFetch(`/cards/${card.card_id}`, {
            method: "PUT",
            headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
            body: JSON.stringify({ name: card.card_name, notes: card.notes || null, lane_id: laneId }),
          });
          const sessions = await fetchSessions();
          lastSessions = sessions;
          refreshCurrentSessionView(sessions);
        } catch (err) { /* next poll will catch it */ }
      });
      submenu.appendChild(item);
    }

    submenuContainer.appendChild(submenu);
    submenuContainer.addEventListener("mouseenter", () => { submenu.style.display = "block"; });
    submenuContainer.addEventListener("mouseleave", () => { submenu.style.display = "none"; });
    menu.appendChild(submenuContainer);
  }

  // Copy workspace path
  menu.appendChild(makeItem("Copy workspace path", "", () => {
    closeContextMenu();
    navigator.clipboard.writeText(card.workspace_path || "").catch(() => {});
  }));

  // Kill session (only if alive)
  if (card.is_alive) {
    menu.appendChild(makeItem("Kill session", "", async () => {
      closeContextMenu();
      const token = getToken();
      try {
        await timedFetch(`/cards/${card.card_id}/session`, {
          method: "DELETE",
          headers: { Authorization: `Bearer ${token}` },
        });
        const sessions = await fetchSessions();
        lastSessions = sessions;
        refreshCurrentSessionView(sessions);
      } catch (err) { /* ignore */ }
    }));
  }

  // Relay section
  if (card.workspace_path) {
    menu.appendChild(makeSeparator());

    const relayMode = card.relay_mode ?? null;
    const relayEnabled = card.relay_enabled ?? false;

    // Mode: Auto
    menu.appendChild(makeItem(
      `Relay: Auto${relayMode === "auto" ? ' <span class="ctx-item-check">\u2713</span>' : ""}`,
      relayEnabled ? "" : "ctx-item-disabled",
      async () => {
        closeContextMenu();
        if (!relayEnabled) return;
        const token = getToken();
        try {
          await timedFetch(`/wake/relay/agents/${encodeURIComponent(card.workspace_path)}/mode`, {
            method: "POST",
            headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
            body: JSON.stringify({ mode: "auto" }),
          });
          const sessions = await fetchSessions();
          lastSessions = sessions;
          refreshCurrentSessionView(sessions);
        } catch {}
      }
    ));

    // Mode: Manual
    menu.appendChild(makeItem(
      `Relay: Manual${relayMode === "manual" ? ' <span class="ctx-item-check">\u2713</span>' : ""}`,
      relayEnabled ? "" : "ctx-item-disabled",
      async () => {
        closeContextMenu();
        if (!relayEnabled) return;
        const token = getToken();
        try {
          await timedFetch(`/wake/relay/agents/${encodeURIComponent(card.workspace_path)}/mode`, {
            method: "POST",
            headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
            body: JSON.stringify({ mode: "manual" }),
          });
          const sessions = await fetchSessions();
          lastSessions = sessions;
          refreshCurrentSessionView(sessions);
        } catch {}
      }
    ));

    // Enable Relay
    menu.appendChild(makeItem(
      `Enable Relay${relayEnabled ? ' <span class="ctx-item-check">\u2713</span>' : ""}`,
      "",
      async () => {
        closeContextMenu();
        const token = getToken();
        try {
          await timedFetch(`/cards/${card.card_id}`, {
            method: "PATCH",
            headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
            body: JSON.stringify({ relay_enabled: !relayEnabled }),
          });
          const sessions = await fetchSessions();
          lastSessions = sessions;
          refreshCurrentSessionView(sessions);
        } catch {}
      }
    ));

    // Re-register Relay
    if (relayEnabled) {
      menu.appendChild(makeItem("Re-register Relay", "", async () => {
        closeContextMenu();
        const token = getToken();
        try {
          await timedFetch(`/wake/relay/reregister/${card.card_id}`, {
            method: "POST",
            headers: { Authorization: `Bearer ${token}` },
          });
          const sessions = await fetchSessions();
          lastSessions = sessions;
          refreshCurrentSessionView(sessions);
        } catch (e) {
          console.error("[relay] re-register failed:", e);
        }
      }));
    }

    // Clear Pending (only if count > 0)
    const pendingCount = card.relay_pending_count ?? 0;
    if (pendingCount > 0) {
      menu.appendChild(makeItem(
        `Clear Pending (${pendingCount})`,
        "",
        () => {
          closeContextMenu();
          showClearPendingConfirm(card, pendingCount);
        }
      ));
    }
  }

  menu.appendChild(makeSeparator());

  // Delete card
  menu.appendChild(makeItem("Delete card\u2026", "ctx-item-danger", () => {
    closeContextMenu();
    showDeleteConfirm(card);
  }));

  menu.style.cssText = `position:fixed;z-index:9999;left:${x}px;top:${y}px;`;
  document.body.appendChild(menu);

  // Clamp to viewport
  requestAnimationFrame(() => {
    const rect = menu.getBoundingClientRect();
    if (rect.right > window.innerWidth) menu.style.left = (x - rect.width) + "px";
    if (rect.bottom > window.innerHeight) menu.style.top = (y - rect.height) + "px";
  });

  // Close on outside click
  const onMouseDown = (e) => {
    if (!menu.contains(e.target)) {
      closeContextMenu();
      document.removeEventListener("mousedown", onMouseDown);
    }
  };
  setTimeout(() => document.addEventListener("mousedown", onMouseDown), 0);
}

// --- Edit Modal ---

function showEditModal(card) {
  uiModalOpen = true;

  const overlay = document.createElement("div");
  overlay.className = "modal-overlay";
  overlay.innerHTML = `
    <div class="modal-box">
      <div class="modal-title">Edit Card</div>
      <div class="field-group">
        <label class="field-label" for="edit-card-name">Name</label>
        <input class="field-input" id="edit-card-name" type="text" value="${escapeHtml(card.card_name)}" autocomplete="off" />
      </div>
      <div class="field-group" style="margin-top:12px">
        <label class="field-label" for="edit-card-notes">Notes</label>
        <textarea class="field-input" id="edit-card-notes" rows="3" style="resize:vertical">${escapeHtml(card.notes || "")}</textarea>
      </div>
      <div class="modal-error" id="edit-card-error" style="display:none"></div>
      <div class="modal-actions">
        <button class="btn-secondary" id="edit-card-cancel">Cancel</button>
        <button class="btn-primary" id="edit-card-save">Save</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const nameEl = overlay.querySelector("#edit-card-name");
  const notesEl = overlay.querySelector("#edit-card-notes");
  const errorEl = overlay.querySelector("#edit-card-error");
  const cancelBtn = overlay.querySelector("#edit-card-cancel");
  const saveBtn = overlay.querySelector("#edit-card-save");

  nameEl.focus();
  nameEl.select();

  cancelBtn.addEventListener("click", () => {
    overlay.remove();
    uiModalOpen = false;
  });

  saveBtn.addEventListener("click", async () => {
    const name = nameEl.value.trim();
    if (!name) {
      errorEl.textContent = "Name is required.";
      errorEl.style.display = "";
      return;
    }
    saveBtn.disabled = true;
    errorEl.style.display = "none";
    const token = getToken();
    try {
      const res = await timedFetch(`/cards/${card.card_id}`, {
        method: "PUT",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({ name, notes: notesEl.value || null, lane_id: card.lane_id }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        errorEl.textContent = data?.error?.message || data.message || `Error ${res.status}`;
        errorEl.style.display = "";
        saveBtn.disabled = false;
        return;
      }
      overlay.remove();
      uiModalOpen = false;
      const sessions = await fetchSessions();
      lastSessions = sessions;
      refreshCurrentSessionView(sessions);
    } catch (err) {
      errorEl.textContent = "Network error. Try again.";
      errorEl.style.display = "";
      saveBtn.disabled = false;
    }
  });
}

// --- Delete Confirm ---

function showDeleteConfirm(card) {
  uiModalOpen = true;

  const overlay = document.createElement("div");
  overlay.className = "modal-overlay";
  overlay.innerHTML = `
    <div class="modal-box">
      <div class="modal-title">Delete &ldquo;${escapeHtml(card.card_name)}&rdquo;?</div>
      <div style="font-size:13px;color:var(--nx-muted,#888);margin-bottom:8px">This will kill the session (if running) and permanently delete the card.</div>
      <div class="modal-error" id="del-card-error" style="display:none"></div>
      <div class="modal-actions">
        <button class="btn-secondary" id="del-card-cancel">Cancel</button>
        <button class="btn-danger" id="del-card-confirm">Delete</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const errorEl = overlay.querySelector("#del-card-error");
  const cancelBtn = overlay.querySelector("#del-card-cancel");
  const confirmBtn = overlay.querySelector("#del-card-confirm");

  cancelBtn.addEventListener("click", () => {
    overlay.remove();
    uiModalOpen = false;
  });

  confirmBtn.addEventListener("click", async () => {
    confirmBtn.disabled = true;
    errorEl.style.display = "none";
    const token = getToken();
    try {
      const res = await timedFetch(`/cards/${card.card_id}`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        errorEl.textContent = data?.error?.message || data.message || `Error ${res.status}`;
        errorEl.style.display = "";
        confirmBtn.disabled = false;
        return;
      }
      overlay.remove();
      uiModalOpen = false;
      const sessions = await fetchSessions();
      lastSessions = sessions;
      refreshCurrentSessionView(sessions);
    } catch (err) {
      errorEl.textContent = "Network error. Try again.";
      errorEl.style.display = "";
      confirmBtn.disabled = false;
    }
  });
}

function showClearPendingConfirm(card, count) {
  uiModalOpen = true;

  const overlay = document.createElement("div");
  overlay.className = "modal-overlay";
  overlay.innerHTML = `
    <div class="modal-box">
      <div class="modal-title">Clear ${count} pending relay messages?</div>
      <div style="font-size:13px;color:var(--nx-muted,#888);margin-bottom:8px">This will discard all pending relay messages for &ldquo;${escapeHtml(card.card_name)}&rdquo;.</div>
      <div class="modal-error" id="clear-relay-error" style="display:none"></div>
      <div class="modal-actions">
        <button class="btn-secondary" id="clear-relay-cancel">Cancel</button>
        <button class="btn-danger" id="clear-relay-confirm">Clear</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const errorEl = overlay.querySelector("#clear-relay-error");
  const cancelBtn = overlay.querySelector("#clear-relay-cancel");
  const confirmBtn = overlay.querySelector("#clear-relay-confirm");

  cancelBtn.addEventListener("click", () => {
    overlay.remove();
    uiModalOpen = false;
  });

  confirmBtn.addEventListener("click", async () => {
    confirmBtn.disabled = true;
    errorEl.style.display = "none";
    const token = getToken();
    try {
      const res = await timedFetch(`/wake/relay/pending/${encodeURIComponent(card.workspace_path)}`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        errorEl.textContent = data?.error?.message || `Error ${res.status}`;
        errorEl.style.display = "";
        confirmBtn.disabled = false;
        return;
      }
      overlay.remove();
      uiModalOpen = false;
      const sessions = await fetchSessions();
      lastSessions = sessions;
      refreshCurrentSessionView(sessions);
    } catch (err) {
      errorEl.textContent = "Network error. Try again.";
      errorEl.style.display = "";
      confirmBtn.disabled = false;
    }
  });
}

async function uploadFiles(files, cardId) {
  if (!files || files.length === 0 || !cardId) return;
  const token = getToken();
  const form = new FormData();
  for (const file of files) form.append("file", file);
  try {
    const res = await fetch(`/cards/${cardId}/upload`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
      body: form,
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      const msg = err?.error?.message || `HTTP ${res.status}`;
      showUploadStatus(`Upload failed: ${msg}`, true);
      return;
    }
    const data = await res.json();
    showUploadStatus(`Uploaded ${data.files?.length || 0} file(s)`);
  } catch (e) {
    showUploadStatus(`Upload failed: ${e.message}`, true);
  }
}

function showUploadStatus(message, isError) {
  const header = document.querySelector(".split-panel .terminal-header")
    || document.querySelector(".terminal-header");
  if (!header) return;
  const existing = header.querySelector(".upload-status");
  if (existing) existing.remove();
  const span = document.createElement("span");
  span.className = "upload-status" + (isError ? " error" : "");
  span.textContent = message;
  header.appendChild(span);
  setTimeout(() => span.remove(), isError ? 5000 : 3000);
}

function renderSplitLayout(sessions) {
  const { lanesHtml } = buildSidebarLanesHtml(sessions);
  const { dotClass, label: connLabel } = connectionStatusInfo();

  let bannersHtml = "";
  if (ghAuthStatus && !ghAuthStatus.authenticated && !ghBannerDismissed) {
    bannersHtml += `<div class="onboarding-banner" id="banner-gh">&#9888; GitHub not authenticated. Open a terminal and run: <code>gh auth login</code><button class="onboarding-dismiss" data-banner="gh">&#215;</button></div>`;
  }
  const hasAliveSplitSessions = sessions && sessions.some(s => s.is_alive);
  if (claudeAuthStatus && !claudeAuthStatus.authenticated && !claudeBannerDismissed && !hasAliveSplitSessions) {
    bannersHtml += `<div class="onboarding-banner" id="banner-claude">&#9888; Claude not authenticated. Open a terminal and run: <code>claude auth login</code><button class="onboarding-dismiss" data-banner="claude">&#215;</button></div>`;
  }

  app.classList.add("split-active");
  app.classList.remove("dashboard-active");
  currentView = "split";

  app.innerHTML = `
    ${bannersHtml}
    <div class="split-layout">
      <div class="split-sidebar" style="width: ${sidebarWidthPx}px">
        <div class="sidebar-header">
          <span class="sidebar-header-title ncc-name" title="NCC name">${escapeHtml(nccName)}</span>
          <button id="sidebar-new-btn">+ New</button>
          <button id="sidebar-inbox-btn" aria-label="Inbox" class="sidebar-inbox-btn">&#9993;<span class="inbox-badge" id="inbox-badge" style="display:none"></span></button>
        </div>
        ${viewSwitcherHtml("split")}
        ${bootstrapBadgeHtml()}
        <div class="sidebar-session-list">${lanesHtml}</div>
        <div class="sidebar-usage" id="sidebar-usage"></div>
        <div class="sidebar-footer">
          <span class="status-dot ${dotClass}"></span>
          <span class="status-label">${connLabel}</span>
          <span class="wake-indicator" style="cursor:pointer" title="Agent Wake — when ON, agents register on the relay and an idle session gets tapped on the shoulder the moment a message arrives. Click to toggle.">
            <span class="wake-dot wake-dot-off"></span>
            <span class="wake-label" id="wake-label">Wake: Off</span>
            <span class="wake-badge" id="wake-badge" style="display:none"></span>
          </span>
          <button id="sidebar-settings-btn" aria-label="Settings" title="Settings & ‘How this works’ legend">&#9881;</button>
        </div>
      </div>
      <div class="split-divider"></div>
      <div class="split-panel">
        ${activeCardId ? "" : emptyPanelHtml(sessions)}
      </div>
    </div>
  `;

  // Attach onboarding banner dismiss handlers
  document.querySelectorAll(".onboarding-dismiss").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.banner === "gh") ghBannerDismissed = true;
      if (btn.dataset.banner === "claude") claudeBannerDismissed = true;
      btn.closest(".onboarding-banner").remove();
    });
  });

  attachSidebarHandlers();
  attachViewSwitcherHandlers();

  // Bootstrap run button
  const splitRunBtn = document.getElementById("bootstrap-run-btn");
  if (splitRunBtn) splitRunBtn.addEventListener("click", handleBootstrapRun);

  // Divider drag (adapted from SplitLayout.tsx)
  const divider = document.querySelector(".split-divider");
  divider.addEventListener("mousedown", () => {
    document.body.classList.add("no-select");
    const sidebar = document.querySelector(".split-sidebar");

    let rafId = null;
    const onMouseMove = (e) => {
      if (rafId) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        const maxW = Math.min(480, window.innerWidth * 0.4);
        const newWidth = Math.min(maxW, Math.max(160, e.clientX));
        sidebar.style.width = newWidth + "px";
        sidebarWidthPx = newWidth;
      });
    };
    const onMouseUp = () => {
      document.body.classList.remove("no-select");
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("blur", onMouseUp);
      if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
    };
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    window.addEventListener("blur", onMouseUp);
  });

  // Drag-and-drop file upload
  const splitPanel = document.querySelector(".split-panel");
  if (splitPanel) {
    let dragDepth = 0;
    let dragCardId = null;

    splitPanel.addEventListener("dragenter", (e) => {
      e.preventDefault();
      dragDepth++;
      if (dragDepth === 1) {
        splitPanel.classList.add("drag-over");
        dragCardId = activeCardId; // snapshot at drag start
      }
    });

    splitPanel.addEventListener("dragleave", (e) => {
      e.preventDefault();
      dragDepth--;
      if (dragDepth === 0) {
        splitPanel.classList.remove("drag-over");
        dragCardId = null;
      }
    });

    splitPanel.addEventListener("dragover", (e) => {
      e.preventDefault(); // required for drop to fire
    });

    splitPanel.addEventListener("drop", (e) => {
      e.preventDefault();
      dragDepth = 0;
      splitPanel.classList.remove("drag-over");
      if (e.dataTransfer.files.length && dragCardId) {
        uploadFiles(e.dataTransfer.files, dragCardId);
      }
      dragCardId = null;
    });
  }

  // If active session, mount terminal
  if (activeCardId && currentSessionId) {
    mountTerminalInPanel();
  }

  startInboxPoll();
}

function updateSidebar(sessions) {
  const listEl = document.querySelector(".sidebar-session-list");
  if (!listEl) return;

  const { lanesHtml } = buildSidebarLanesHtml(sessions);
  listEl.innerHTML = lanesHtml;

  attachSidebarHandlers();

  // Update bootstrap badge in-place
  const bsSection = document.querySelector(".bootstrap-section");
  if (bsSection) {
    bsSection.outerHTML = bootstrapBadgeHtml();
    const runBtn = document.getElementById("bootstrap-run-btn");
    if (runBtn) runBtn.addEventListener("click", handleBootstrapRun);
  }

  const footerDot = document.querySelector(".sidebar-footer .status-dot");
  const footerLabel = document.querySelector(".sidebar-footer .status-label");
  if (footerDot && footerLabel) {
    const { dotClass, label } = connectionStatusInfo();
    footerDot.className = `status-dot ${dotClass}`;
    footerLabel.textContent = label;
  }

  updateWakeIndicator();
  updateInboxBadge();
}

function mountTerminalInPanel() {
  const panel = document.querySelector(".split-panel");
  if (!panel) return;

  // Dispose any prior terminal before remounting. renderSplitLayout() wipes the
  // panel DOM and calls this WITHOUT cleanupTerminal(), so without this the old
  // xterm instance is orphaned, not disposed — its requestAnimationFrame render
  // loop keeps the instance (and its ~10k-line scrollback backing store) alive,
  // leaking ~10-30MB per remount. Over a long split-view session with many card
  // switches / view refreshes this accumulates to GBs. The live WebSocket stays
  // open and simply writes into the freshly-created instance below, so there is
  // no behavior change — only the leaked predecessor is reclaimed.
  if (termInstance) {
    termInstance.dispose();
    termInstance = null;
    fitAddon = null;
  }

  termStatus = "connecting";

  panel.innerHTML = `
    <div class="terminal-header">
      ${cameFromDashboard ? `<button class="dashboard-back-btn" id="dashboard-back-btn">&larr; Dashboard</button>` : ""}
      <span class="terminal-title">${escapeHtml(currentCardName || "Terminal")}</span>
      <button class="upload-btn" id="split-upload-btn" aria-label="Upload files">&#11014;</button>
      <span class="terminal-status">
        <span class="status-dot status-unreachable"></span>
        <span class="status-label">Connecting...</span>
      </span>
    </div>
    <div class="terminal-container" id="terminal-container"></div>
  `;

  const dashboardBackBtn = document.getElementById("dashboard-back-btn");
  if (dashboardBackBtn) dashboardBackBtn.addEventListener("click", returnToDashboard);

  // Upload button + hidden file input (guarded singleton)
  const uploadBtn = document.getElementById("split-upload-btn");
  if (uploadBtn) {
    if (!document.getElementById("upload-input")) {
      const inp = document.createElement("input");
      inp.type = "file";
      inp.id = "upload-input";
      inp.multiple = true;
      inp.style.display = "none";
      document.body.appendChild(inp);
      inp.addEventListener("change", () => {
        if (inp.files.length && activeCardId) {
          uploadFiles(inp.files, activeCardId);
        }
        inp.value = "";
      });
    }
    uploadBtn.addEventListener("click", () => {
      document.getElementById("upload-input").click();
    });
  }

  termInstance = new Terminal({
    disableStdin: false,
    cursorBlink: true,
    fontSize: 12,
    scrollback: 10000,
    fontFamily: '"SF Mono", "Menlo", "Monaco", monospace',
    theme: {
      background: "#0a0a0a",
      foreground: "#e0e0e0",
      cursor: "#e0e0e0",
    },
  });
  fitAddon = new FitAddon();
  termInstance.loadAddon(fitAddon);
  termInstance.loadAddon(new WebLinksAddon());

  const container = document.getElementById("terminal-container");
  termInstance.open(container);

  termInstance.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "input", data }));
    }
  });

  // ResizeObserver with rAF throttle (mitigates drag resize storm)
  if (resizeObserver) resizeObserver.disconnect();
  let resizeRafId = null;
  resizeObserver = new ResizeObserver(() => {
    if (resizeRafId) return;
    resizeRafId = requestAnimationFrame(() => {
      resizeRafId = null;
      if (container.clientWidth > 0 && container.clientHeight > 0 && fitAddon) {
        fitAddon.fit();
        const dims = fitAddon.proposeDimensions();
        if (dims && dims.cols > 0 && dims.rows > 0) {
          sendResizeMsg(dims.cols, dims.rows);
        }
      }
    });
  });
  resizeObserver.observe(container);
}

function openSplitTerminal(sessionId, cardName, cardId) {
  cleanupTerminal();

  activeCardId = cardId;
  currentSessionId = sessionId;
  currentCardName = cardName;
  window.location.hash = `session/${sessionId}`;

  // Update sidebar highlight
  document.querySelectorAll(".sidebar-session-entry").forEach(el => {
    el.classList.toggle("sidebar-entry-active", el.dataset.cardId === cardId);
  });

  mountTerminalInPanel();
  connectWs(sessionId);
  // NOTE: Do NOT call stopPolling() — polling continues in split mode
}

async function startSessionAndOpenSplit(cardId, cardName) {
  // Immediately show loading state in panel + highlight entry
  activeCardId = cardId;
  document.querySelectorAll(".sidebar-session-entry").forEach(el => {
    el.classList.toggle("sidebar-entry-active", el.dataset.cardId === cardId);
  });
  const panel = document.querySelector(".split-panel");
  if (panel) {
    panel.innerHTML = `<div class="split-empty">Starting session...</div>`;
  }

  const token = getToken();
  try {
    const res = await fetch(`/cards/${cardId}/session`, {
      method: "POST",
      headers: {
        "Authorization": `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ cols: Math.floor((document.querySelector(".split-panel")?.clientWidth || 800) / 9), rows: Math.floor((document.querySelector(".split-panel")?.clientHeight || 600) / 17) }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      alert(err?.error?.message || "Failed to start session");
      return;
    }
    const data = await res.json();
    if (data.session_id) {
      openSplitTerminal(data.session_id, cardName, cardId);
    }
  } catch (e) {
    alert("Failed to start session: " + e.message);
  }
}

// --- Settings View ---

function renderSettingControl(setting) {
  const disabled = !setting.writable || setting.env_wins;
  const disabledAttr = disabled ? "disabled" : "";
  if (setting.secret) {
    const placeholder = setting.is_set ? "Stored secret" : "Enter secret";
    return `<input class="settings-input" id="setting-${escapeAttr(setting.key)}" type="password" autocomplete="off" placeholder="${placeholder}" ${disabledAttr} />`;
  }
  if (setting.value_type === "bool") {
    const value = String(setting.value || "").toLowerCase();
    return `
      <select class="settings-input" id="setting-${escapeAttr(setting.key)}" ${disabledAttr}>
        <option value="true" ${value === "true" ? "selected" : ""}>Enabled</option>
        <option value="false" ${value === "false" ? "selected" : ""}>Disabled</option>
      </select>
    `;
  }
  if (setting.value_type === "color") {
    const val = /^#[0-9a-fA-F]{6}$/.test(setting.value || "") ? setting.value : "#3B82F6";
    return `<input class="settings-input settings-input-color" id="setting-${escapeAttr(setting.key)}" type="color" value="${escapeAttr(val)}" ${disabledAttr} />`;
  }
  // Long / multi-line text settings (relay tap message, Claude launch command) get a textarea.
  if (setting.key === "relay_tap_template" || setting.key === "claude_launch_command") {
    return `<textarea class="settings-input settings-input-multiline" id="setting-${escapeAttr(setting.key)}" rows="4" ${disabledAttr}>${escapeHtml(setting.value || "")}</textarea>`;
  }
  const type = setting.value_type === "u64" ? "number" : "text";
  const inputMode = setting.value_type === "u64" ? "inputmode=\"numeric\" min=\"0\"" : "";
  return `<input class="settings-input" id="setting-${escapeAttr(setting.key)}" type="${type}" ${inputMode} value="${escapeAttr(setting.value || "")}" ${disabledAttr} />`;
}

function renderSettingRow(setting) {
  const disabled = !setting.writable || setting.env_wins;
  const status = setting.env_wins
    ? "Environment override"
    : setting.kind === "restart"
      ? "Restart required"
      : "Live";
  const secretState = setting.secret
    ? `<span class="settings-secret-state">${setting.is_set ? "Set" : "Not set"}</span>`
    : "";
  return `
    <div class="settings-row" data-setting-key="${escapeAttr(setting.key)}" data-setting-secret="${setting.secret ? "true" : "false"}">
      <div class="settings-row-head">
        <label for="setting-${escapeAttr(setting.key)}">${escapeHtml(setting.label || setting.key)}</label>
        <span class="settings-badge ${setting.kind === "restart" ? "is-restart" : "is-live"}">${status}</span>
      </div>
      <div class="settings-row-control">
        ${renderSettingControl(setting)}
        <button class="settings-save-btn" data-setting-save="${escapeAttr(setting.key)}" ${disabled ? "disabled" : ""}>Save</button>
      </div>
      <div class="settings-row-foot">
        <code>${escapeHtml(setting.key)}</code>
        ${secretState}
        <span class="settings-row-status" id="setting-status-${escapeAttr(setting.key)}"></span>
      </div>
    </div>
  `;
}

function renderSettingsSections(settings) {
  const groups = new Map();
  for (const setting of settings) {
    const section = setting.section || "Settings";
    if (!groups.has(section)) groups.set(section, []);
    groups.get(section).push(setting);
  }
  return Array.from(groups.entries()).map(([section, rows]) => `
    <section class="settings-section">
      <h2>${escapeHtml(section)}</h2>
      <div class="settings-section-rows">
        ${rows.map(renderSettingRow).join("")}
      </div>
    </section>
  `).join("");
}

async function renderSettings() {
  settingsReturnView = (currentView === "dashboard" || currentView === "split") ? currentView : "board";
  currentView = "settings";
  stopPolling();
  const token = getToken();
  const headers = { Authorization: `Bearer ${token}` };

  let settings = [];
  let settingsError = null;
  try {
    const res = await timedFetch("/settings", { headers });
    if (!res.ok) throw new Error("Failed to load settings");
    const data = await res.json();
    settings = Array.isArray(data.settings) ? data.settings : [];
  } catch (e) {
    settingsError = e.message || "Failed to load settings";
  }

  app.classList.remove("split-active", "dashboard-active");
  app.innerHTML = `
    <div class="settings-view">
      <div class="terminal-header">
        <button class="back-btn" id="settings-back">&larr;</button>
        <span class="terminal-title">Settings</span>
      </div>
      <div class="settings-body">
        <details class="settings-legend" style="margin-bottom:18px;border:1px solid #232a3a;border-radius:10px;background:rgba(255,255,255,.02);padding:0 14px;" open>
          <summary style="cursor:pointer;padding:12px 0;font-weight:600;">How this works — quick legend</summary>
          <div style="padding:0 2px 14px;line-height:1.7;font-size:13.5px;color:#aeb6c6;">
            <p style="margin:6px 0;"><b>Views</b> (top tabs): <b>Board</b> = lanes / kanban · <b>Split</b> = session list + one terminal · <b>Grid</b> = every live session at once.</p>
            <p style="margin:6px 0;"><b>Agent Wake</b> (the “Wake: …” pill, bottom-left of Split view): when <b>On</b>, agents register on the relay and an <i>idle</i> session gets tapped on the shoulder the moment a message arrives. Defaults to <b>On</b>. Click the pill to toggle.</p>
            <p style="margin:6px 0;"><b>Relay mode</b> (icon on each card): <b>📡 Auto</b> = messages are delivered to the agent automatically · <b>⏸ Manual</b> = messages are held until you deliver them. Click the icon to switch.</p>
            <p style="margin:6px 0;"><b>Status dots</b>: green = connected / active · grey = off · amber = retrying.</p>
            <p style="margin:6px 0;"><b>⚙ gear</b> = this Settings page · <b>⋮</b> on a card = per-card actions · the <b>Experimental Actions</b> panel sends a message or sets wake / relay mode for one session.</p>
          </div>
        </details>
        ${settingsError
          ? `<div class="settings-load-error">${escapeHtml(settingsError)}</div>`
          : renderSettingsSections(settings)}
      </div>
    </div>
  `;

  document.getElementById("settings-back").addEventListener("click", () => {
    currentView = settingsReturnView || initialViewMode();
    if (lastSessions) renderPreferredSessionView(lastSessions);
    startPolling();
  });

  document.querySelectorAll("[data-setting-save]").forEach((button) => {
    button.addEventListener("click", async () => {
      const key = button.getAttribute("data-setting-save");
      const row = document.querySelector(`[data-setting-key="${CSS.escape(key)}"]`);
      const input = document.getElementById(`setting-${key}`);
      const status = document.getElementById(`setting-status-${key}`);
      const isSecret = row?.getAttribute("data-setting-secret") === "true";
      if (!input || !status) return;
      if (isSecret && !input.value.trim()) {
        status.textContent = "Enter a new secret";
        status.className = "settings-row-status is-error";
        return;
      }
      button.disabled = true;
      status.textContent = "Saving...";
      status.className = "settings-row-status";
      try {
        const res = await timedFetch(`/settings/${encodeURIComponent(key)}`, {
          method: "PUT",
          headers: { ...headers, "Content-Type": "application/json" },
          body: JSON.stringify({ value: input.value }),
        });
        if (!res.ok) {
          const error = await res.json().catch(() => ({}));
          throw new Error(error?.error?.message || "Failed to save setting");
        }
        if (isSecret) input.value = "";
        status.textContent = "Saved";
        status.className = "settings-row-status is-saved";
        setTimeout(() => { status.textContent = ""; }, 2200);
        // Live-apply NCC identity / launch-command changes without a reload.
        if (key === "ncc_display_name" || key === "ncc_accent_color" || key === "claude_launch_command") loadBranding();
      } catch (e) {
        status.textContent = e.message || "Failed to save";
        status.className = "settings-row-status is-error";
      } finally {
        button.disabled = false;
      }
    });
  });
}

// --- File Browser View ---

async function renderFileBrowser(cardId) {
  fileBrowserReturnView = (currentView === "dashboard" || currentView === "split") ? currentView : "board";
  currentView = "filebrowser";
  stopPolling();
  const token = getToken();
  const headers = { Authorization: `Bearer ${token}` };

  app.classList.remove("split-active", "dashboard-active");
  app.innerHTML = `
    <div class="filebrowser-view">
      <div class="terminal-header">
        <button class="back-btn" id="fb-back">&larr;</button>
        <span class="terminal-title">Files</span>
      </div>
      <div class="filebrowser-body" id="fb-body">
        <div class="filebrowser-tree" id="fb-tree"><p class="muted" style="padding:1rem">Loading...</p></div>
        <div class="filebrowser-content" id="fb-content"><p class="muted" style="padding:1rem">Select a file to view</p></div>
      </div>
    </div>
  `;

  document.getElementById("fb-back").addEventListener("click", () => {
    currentView = fileBrowserReturnView || initialViewMode();
    if (lastSessions) renderPreferredSessionView(lastSessions);
    startPolling();
  });

  await loadDirectory(cardId, "", document.getElementById("fb-tree"), headers);
}

async function loadDirectory(cardId, path, container, headers) {
  try {
    const url = path
      ? `/cards/${cardId}/files?path=${encodeURIComponent(path)}`
      : `/cards/${cardId}/files`;
    const res = await timedFetch(url, { headers });
    if (!res.ok) { container.innerHTML = `<p class="muted">Failed to load</p>`; return; }
    const entries = await res.json();
    container.innerHTML = "";
    for (const entry of entries) {
      const el = document.createElement("div");
      el.className = `filebrowser-entry ${entry.is_dir ? "is-dir" : "is-file"}`;
      el.textContent = entry.name;
      if (entry.is_dir) {
        let expanded = false;
        let childContainer = null;
        el.addEventListener("click", async () => {
          if (!expanded) {
            childContainer = document.createElement("div");
            childContainer.className = "filebrowser-children";
            el.after(childContainer);
            await loadDirectory(cardId, entry.path, childContainer, headers);
            expanded = true;
          } else {
            if (childContainer) childContainer.remove();
            childContainer = null;
            expanded = false;
          }
        });
      } else {
        el.addEventListener("click", async () => {
          const contentPane = document.getElementById("fb-content");
          const body = document.getElementById("fb-body");
          if (!contentPane) return;
          contentPane.innerHTML = `<p class="muted">Loading...</p>`;
          if (body) body.classList.add("showing-content");
          try {
            const fRes = await timedFetch(
              `/cards/${cardId}/files/content?path=${encodeURIComponent(entry.path)}`,
              { headers }
            );
            if (fRes.status === 422) {
              contentPane.innerHTML = `<p class="muted">Binary file — open in terminal to view.</p>`;
            } else if (fRes.status === 413) {
              contentPane.innerHTML = `<p class="muted">File too large to display (&gt;512KB).</p>`;
            } else if (fRes.ok) {
              const text = await fRes.text();
              contentPane.innerHTML = `<pre class="filebrowser-code">${escapeHtml(text)}</pre>`;
            } else {
              contentPane.innerHTML = `<p class="muted">Failed to load file.</p>`;
            }
          } catch {
            contentPane.innerHTML = `<p class="muted">Failed to load file.</p>`;
          }
        });
      }
      container.appendChild(el);
    }
  } catch {
    container.innerHTML = `<p class="muted">Failed to load directory.</p>`;
  }
}

// --- Wizard ---

async function openWizard() {
  wizardReturnView = (currentView === "dashboard" || currentView === "split") ? currentView : "board";
  stopPolling();
  wizardStep = 1;
  wizardError = null;
  wizardCreating = false;
  wizardName = "";
  wizardNotes = "";
  wizardSourceType = "github";
  wizardLocalPath = "";
  wizardSelectedRepo = null;
  wizardRepoSearch = "";
  wizardRepos = [];
  wizardReposLoading = false;

  const token = getToken();
  const headers = { Authorization: `Bearer ${token}` };

  // Fetch lanes + settings in parallel
  try {
    const [lanesRes, defaultLaneRes, orgRes] = await Promise.all([
      timedFetch("/lanes", { headers }),
      timedFetch("/settings/default_lane_id", { headers }).catch(() => null),
      timedFetch("/settings/github_org", { headers }).catch(() => null),
    ]);

    if (!lanesRes.ok) throw new Error("Failed to load lanes");
    wizardLanes = await lanesRes.json();

    // Default lane
    if (defaultLaneRes && defaultLaneRes.ok) {
      const d = await defaultLaneRes.json();
      wizardLaneId = d.value;
    }
    // Fallback: first lane
    if (!wizardLaneId && wizardLanes.length > 0) {
      wizardLaneId = wizardLanes[0].id;
    }

    // GitHub org
    if (orgRes && orgRes.ok) {
      const o = await orgRes.json();
      wizardOrg = o.value || "";
    }
  } catch (e) {
    wizardStep = 0;
    currentView = wizardReturnView || initialViewMode();
    if (lastSessions) renderPreferredSessionView(lastSessions);
    startPolling();
    return;
  }

  renderWizard();
}

function closeWizard() {
  resetWizard();
  currentView = wizardReturnView || initialViewMode();
  if (lastSessions) renderPreferredSessionView(lastSessions);
  startPolling();
}

function resetWizard() {
  wizardStep = 0;
  wizardName = "";
  wizardLaneId = "";
  wizardNotes = "";
  wizardSourceType = "github";
  wizardLocalPath = "";
  wizardSelectedRepo = null;
  wizardRepoSearch = "";
  wizardLanes = [];
  wizardRepos = [];
  wizardReposLoading = false;
  wizardGhAuth = null;
  wizardOrg = "";
  wizardError = null;
  wizardCreating = false;
  wizardLaunchClaude = false;
  wizardRelayEnabled = true;
}

function renderWizard() {
  app.classList.remove("split-active", "dashboard-active");
  let bodyHtml = "";
  let footerHtml = "";

  if (wizardStep === 1) {
    const laneOptions = wizardLanes
      .map(
        (l) =>
          `<option value="${escapeHtml(l.id)}"${l.id === wizardLaneId ? " selected" : ""}>${escapeHtml(l.name)}</option>`
      )
      .join("");

    bodyHtml = `
      <div class="field-group">
        <label class="field-label" for="wiz-name">Name</label>
        <input class="field-input" id="wiz-name" type="text" placeholder="e.g. auth-refactor" value="${escapeHtml(wizardName)}" autocomplete="off" autocapitalize="off" />
      </div>
      <div class="field-group">
        <label class="field-label" for="wiz-lane">Lane</label>
        <select class="field-select" id="wiz-lane">${laneOptions}</select>
      </div>
      <div class="field-group">
        <label class="field-label" for="wiz-notes">Notes (optional)</label>
        <textarea class="field-textarea" id="wiz-notes" rows="2" placeholder="What are you working on?">${escapeHtml(wizardNotes)}</textarea>
      </div>
    `;
    footerHtml = `
      <button class="btn-secondary" id="wiz-cancel">Cancel</button>
      <button class="btn-primary" id="wiz-next">Next</button>
    `;
  } else if (wizardStep === 2) {
    const ghActive = wizardSourceType === "github" ? " active" : "";
    const localActive = wizardSourceType === "local" ? " active" : "";

    let sourceBody = "";
    if (wizardSourceType === "github") {
      let repoListHtml = "";
      if (wizardReposLoading) {
        repoListHtml = `<div class="repo-loading">Loading repos...</div>`;
      } else if (wizardRepos.length > 0) {
        const filtered = wizardRepoSearch
          ? wizardRepos.filter((r) =>
              r.name.toLowerCase().includes(wizardRepoSearch.toLowerCase())
            )
          : wizardRepos;
        repoListHtml = `<div class="repo-list">${filtered
          .map(
            (r) => `
          <div class="repo-item${wizardSelectedRepo === r.full_name ? " selected" : ""}" data-repo="${escapeHtml(r.full_name)}">
            <div class="repo-item-name">${escapeHtml(r.name)}${r.is_private ? " <span style='color:var(--nx-dim)'>private</span>" : ""}</div>
            ${r.description ? `<div class="repo-item-desc">${escapeHtml(r.description)}</div>` : ""}
          </div>`
          )
          .join("")}</div>`;
      } else if (!wizardOrg) {
        repoListHtml = `<div class="repo-loading">Set a GitHub org in desktop settings first.</div>`;
      }

      sourceBody = `
        ${wizardOrg ? `<input class="repo-search" id="wiz-repo-search" type="text" placeholder="Search repos..." value="${escapeHtml(wizardRepoSearch)}" autocomplete="off" autocapitalize="off" />` : ""}
        ${repoListHtml}
      `;
    } else {
      sourceBody = `
        <div class="folder-picker" id="folder-picker">
          <div class="folder-breadcrumb" id="folder-breadcrumb"></div>
          <div class="folder-list" id="folder-list">
            <div class="folder-loading">Loading\u2026</div>
          </div>
          <div class="folder-selected" id="folder-selected">${wizardLocalPath ? escapeHtml(wizardLocalPath) : "No folder selected"}</div>
        </div>
      `;
    }

    bodyHtml = `
      <div class="source-tabs">
        <button class="source-tab${ghActive}" data-source="github">GitHub</button>
        <button class="source-tab${localActive}" data-source="local">Local Path</button>
      </div>
      ${sourceBody}
      <div class="field-group" style="margin-top:12px;border-top:1px solid rgba(255,255,255,0.05);padding-top:12px">
        <label class="toggle-row" style="display:flex;align-items:center;gap:10px;cursor:pointer">
          <input type="checkbox" id="wiz-claude" ${wizardLaunchClaude ? "checked" : ""} style="accent-color:#f97316" />
          <div>
            <div style="font-size:13px;color:var(--nx-text,#e0e0e0)">Start with Claude</div>
            <div style="font-size:10px;color:var(--nx-dim,#666)">Launches claude --dangerously-skip-permissions</div>
          </div>
        </label>
      </div>
      <div class="field-group" style="margin-top:8px">
        <label class="toggle-row" style="display:flex;align-items:center;gap:10px;cursor:pointer">
          <input type="checkbox" id="wiz-relay" ${wizardRelayEnabled ? "checked" : ""} style="accent-color:var(--nx-accent)" />
          <div>
            <div style="font-size:13px;color:var(--nx-text,#e0e0e0)">Enable Relay messaging</div>
            <div style="font-size:10px;color:var(--nx-dim,#666)">Agent will receive relay messages when idle</div>
          </div>
        </label>
      </div>
    `;

    const createDisabled = wizardCreating ? " disabled" : "";
    const createLabel = wizardCreating
      ? `<span class="spinner"></span>Creating...`
      : "Create";

    footerHtml = `
      <button class="btn-secondary" id="wiz-back"${wizardCreating ? " disabled" : ""}>Back</button>
      <button class="btn-primary" id="wiz-create"${createDisabled}>${createLabel}</button>
    `;
  }

  const errorHtml = wizardError
    ? `<div class="wizard-error">${escapeHtml(wizardError)}</div>`
    : "";

  const stepLabel = wizardStep === 1 ? "Step 1 of 2" : "Step 2 of 2";

  app.innerHTML = `
    <div class="wizard">
      <div class="wizard-header">
        <span class="wizard-title">New Session</span>
        <span class="wizard-step-label">${stepLabel}</span>
      </div>
      <div class="wizard-body">
        ${errorHtml}
        ${bodyHtml}
      </div>
      <div class="wizard-footer">
        ${footerHtml}
      </div>
    </div>
  `;

  // Wire events
  if (wizardStep === 1) {
    document.getElementById("wiz-cancel").addEventListener("click", closeWizard);
    document.getElementById("wiz-next").addEventListener("click", wizardGoStep2);
  } else if (wizardStep === 2) {
    document.getElementById("wiz-back").addEventListener("click", () => {
      if (wizardCreating) return;
      wizardStep = 1;
      wizardError = null;
      renderWizard();
    });
    document.getElementById("wiz-create").addEventListener("click", handleCreate);

    // Source tabs
    document.querySelectorAll(".source-tab").forEach((tab) => {
      tab.addEventListener("click", () => {
        if (wizardCreating) return;
        const src = tab.dataset.source;
        if (src !== wizardSourceType) {
          wizardSourceType = src;
          wizardError = null;
          renderWizard();
        }
      });
    });

    // Repo search
    const searchEl = document.getElementById("wiz-repo-search");
    if (searchEl) {
      searchEl.addEventListener("input", (e) => {
        wizardRepoSearch = e.target.value;
        renderWizard();
        // Re-focus and restore cursor
        const newSearch = document.getElementById("wiz-repo-search");
        if (newSearch) {
          newSearch.focus();
          newSearch.setSelectionRange(wizardRepoSearch.length, wizardRepoSearch.length);
        }
      });
    }

    // Repo items
    document.querySelectorAll(".repo-item").forEach((el) => {
      el.addEventListener("click", () => {
        if (wizardCreating) return;
        wizardSelectedRepo = el.dataset.repo;
        wizardError = null;
        renderWizard();
      });
    });

    // Folder picker (local source)
    if (wizardSourceType === "local" && document.getElementById("folder-picker")) {
      loadWorkspaceDirs("");
    }

    // Claude toggle
    const claudeEl = document.getElementById("wiz-claude");
    if (claudeEl) {
      claudeEl.addEventListener("change", (e) => {
        wizardLaunchClaude = e.target.checked;
      });
    }

    // Relay toggle
    const relayEl = document.getElementById("wiz-relay");
    if (relayEl) {
      relayEl.addEventListener("change", (e) => {
        wizardRelayEnabled = e.target.checked;
      });
    }
  }
}

function wizardGoStep2() {
  // Capture inputs from DOM
  const nameEl = document.getElementById("wiz-name");
  const laneEl = document.getElementById("wiz-lane");
  const notesEl = document.getElementById("wiz-notes");
  if (nameEl) wizardName = nameEl.value;
  if (laneEl) wizardLaneId = laneEl.value;
  if (notesEl) wizardNotes = notesEl.value;

  // Validate
  if (!wizardName.trim()) {
    wizardError = "Name is required.";
    renderWizard();
    return;
  }

  wizardError = null;
  wizardStep = 2;
  renderWizard();

  // Load repos if GitHub source and org is set
  if (wizardSourceType === "github" && wizardOrg && wizardRepos.length === 0) {
    loadRepos();
  }
}

async function loadRepos() {
  wizardReposLoading = true;
  renderWizard();

  const token = getToken();
  try {
    const res = await timedFetch(
      `/gh/repos?org=${encodeURIComponent(wizardOrg)}`,
      { headers: { Authorization: `Bearer ${token}` } },
      GH_FETCH_TIMEOUT_MS
    );
    if (!res.ok) {
      const body = await res.json().catch(() => null);
      throw new Error(body?.error?.message || `HTTP ${res.status}`);
    }
    wizardRepos = await res.json();
  } catch (e) {
    wizardError = `Failed to load repos: ${e.message}`;
  }
  wizardReposLoading = false;
  renderWizard();
}

// Folder browser state
let folderBrowseRoot = null;
let folderBrowsePath = ""; // current path relative to workspace_root

async function loadWorkspaceDirs(path) {
  folderBrowsePath = path;
  const listEl = document.getElementById("folder-list");
  const breadcrumbEl = document.getElementById("folder-breadcrumb");
  const selectedEl = document.getElementById("folder-selected");
  if (!listEl) return;

  listEl.innerHTML = '<div class="folder-loading">Loading\u2026</div>';

  const token = getToken();
  try {
    const res = await timedFetch(`/workspaces/browse?path=${encodeURIComponent(path)}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    const data = await res.json().catch(() => ({}));

    if (!res.ok) {
      if (data?.error?.code === "workspace_not_configured") {
        listEl.innerHTML = '<div class="folder-error">Configure workspace_root in Settings first.</div>';
      } else {
        listEl.innerHTML = `<div class="folder-error">Error: ${escapeHtml(data?.error?.message || data.message || String(res.status))}</div>`;
      }
      return;
    }

    folderBrowseRoot = data.root || null;

    // Render breadcrumb
    if (breadcrumbEl) {
      const segments = path ? path.split("/").filter(Boolean) : [];
      let breadHtml = '<span data-path="" style="cursor:pointer;color:var(--nx-accent,#f97316)">workspace</span>';
      let acc = "";
      for (const seg of segments) {
        acc = acc ? acc + "/" + seg : seg;
        const captured = acc;
        breadHtml += ` / <span data-path="${escapeHtml(captured)}" style="cursor:pointer;color:var(--nx-accent,#f97316)">${escapeHtml(seg)}</span>`;
      }
      breadcrumbEl.innerHTML = breadHtml;
      breadcrumbEl.querySelectorAll("span[data-path]").forEach(el => {
        el.addEventListener("click", () => loadWorkspaceDirs(el.dataset.path));
      });
    }

    // Render directory list
    const dirs = data.dirs || [];
    if (dirs.length === 0) {
      listEl.innerHTML = '<div class="folder-loading" style="color:var(--nx-muted,#888)">No subdirectories</div>';
    } else {
      listEl.innerHTML = dirs.map(d =>
        `<div class="folder-item" data-path="${escapeHtml(d.path)}" data-name="${escapeHtml(d.name)}">&#128193; ${escapeHtml(d.name)}</div>`
      ).join("");
      listEl.querySelectorAll(".folder-item").forEach(el => {
        el.addEventListener("click", () => loadWorkspaceDirs(el.dataset.path));
      });
    }

    // "Use this folder" button
    const useBtn = document.createElement("div");
    useBtn.className = "folder-item";
    useBtn.style.cssText = "color:var(--nx-accent,#f97316);border-top:1px solid var(--nx-border,rgba(255,255,255,0.1));margin-top:4px;";
    useBtn.textContent = "\u2713 Use this folder";
    useBtn.addEventListener("click", () => {
      if (folderBrowseRoot) {
        wizardLocalPath = path ? folderBrowseRoot + "/" + path : folderBrowseRoot;
      } else {
        wizardLocalPath = path || "";
      }
      if (selectedEl) selectedEl.textContent = wizardLocalPath;
    });
    listEl.appendChild(useBtn);

  } catch (err) {
    listEl.innerHTML = `<div class="folder-error">Network error.</div>`;
  }
}

async function handleCreate() {
  if (wizardCreating) return;

  // Build request body
  const body = {
    name: wizardName.trim(),
    lane_id: wizardLaneId,
    notes: wizardNotes.trim() || null,
    source_type: wizardSourceType,
  };

  if (wizardLaunchClaude) {
    body.initial_command = claudeLaunchCommand;
  }

  body.relay_enabled = wizardRelayEnabled;

  if (wizardSourceType === "github") {
    if (!wizardSelectedRepo) {
      wizardError = "Select a repo.";
      renderWizard();
      return;
    }
    body.repo_full_name = wizardSelectedRepo;
  } else {
    if (!wizardLocalPath.trim()) {
      wizardError = "Path is required.";
      renderWizard();
      return;
    }
    body.local_path = wizardLocalPath.trim();
  }

  // Disable button
  wizardCreating = true;
  wizardError = null;
  renderWizard();

  const token = getToken();
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 120000); // 120s for clone
    const res = await fetch("/cards", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    clearTimeout(timeout);

    const data = await res.json();

    if (!res.ok) {
      throw new Error(data?.error?.message || `HTTP ${res.status}`);
    }

    // Success — refresh sessions, then navigate
    const sessionId = data.session_id;
    const cardName = data.card_name;
    const cardId = data.card_id;
    resetWizard();

    // Refresh sessions so the new card appears in sidebar/board
    try {
      const sessions = await fetchSessions();
      lastSessions = sessions;
    } catch (_) {}

    if (sessionId) {
      // Re-render the layout (wizard replaced it) then open terminal
      if (wizardReturnView === "dashboard") {
        cameFromDashboard = true;
        renderSplitLayout(lastSessions || []);
        openSplitTerminal(sessionId, cardName, cardId);
      } else if (window.innerWidth >= 768) {
        renderBoard(lastSessions || []);
        openSplitTerminal(sessionId, cardName, cardId);
      } else {
        openTerminal(sessionId, cardName);
      }
    } else {
      // PTY spawn failed — return to the previous session surface.
      startPolling();
      currentView = wizardReturnView || initialViewMode();
      if (lastSessions) renderPreferredSessionView(lastSessions);
    }
  } catch (e) {
    wizardCreating = false;
    wizardError = e.name === "AbortError" ? "Request timed out — clone may still be running." : e.message;
    renderWizard();
  }
}

// --- Terminal View ---

async function startSessionAndOpen(cardId, cardName, initialCommand) {
  const token = getToken();
  try {
    const panel = document.querySelector(".split-panel");
    const body = { cols: panel ? Math.floor(panel.clientWidth / 9) : 80, rows: panel ? Math.floor(panel.clientHeight / 17) : 24 };
    if (initialCommand) body.initial_command = initialCommand;
    const res = await fetch(`/cards/${cardId}/session`, {
      method: "POST",
      headers: {
        "Authorization": `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      alert(err?.error?.message || "Failed to start session");
      return;
    }
    const data = await res.json();
    if (data.session_id) {
      openTerminal(data.session_id, cardName);
    }
  } catch (e) {
    alert("Failed to start session: " + e.message);
  }
}

function openTerminal(sessionId, cardName) {
  if (window.innerWidth >= 768) {
    const card = lastSessions?.find(s => s.session_id === sessionId);
    const cardId = card?.card_id || null;
    openSplitTerminal(sessionId, cardName, cardId);
    return;
  }
  currentView = "terminal";
  currentSessionId = sessionId;
  currentCardName = cardName;
  termStatus = "connecting";
  window.location.hash = `session/${sessionId}`;
  stopPolling();
  renderTerminal();
  connectWs(sessionId);
}

function renderTerminal() {
  const statusDot = termStatus === "live" ? "status-connected"
    : termStatus === "ended" ? "status-offline"
    : termStatus === "disconnected" ? "status-unreachable"
    : "status-unreachable";
  const statusLabel = termStatus === "live" ? "Live"
    : termStatus === "ended" ? "Session ended"
    : termStatus === "disconnected" ? "Disconnected"
    : "Connecting...";

  app.innerHTML = `
    <div class="terminal-view">
      <div class="terminal-header">
        <button class="back-btn" id="term-back">&larr;</button>
        <span class="terminal-title">${escapeHtml(currentCardName || "Terminal")}</span>
        <button class="voice-toggle${voiceMode ? " active" : ""}" id="voice-toggle" aria-label="Voice input mode">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="5" y="1" width="6" height="9" rx="3"/>
            <path d="M3 7a5 5 0 0 0 10 0"/>
            <line x1="8" y1="12" x2="8" y2="15"/>
            <line x1="5" y1="15" x2="11" y2="15"/>
          </svg>
        </button>
        <button class="upload-btn" id="term-upload-btn" aria-label="Upload files">&#11014;</button>
        <span class="terminal-status">
          <span class="status-dot ${statusDot}"></span>
          <span class="status-label">${statusLabel}</span>
        </span>
      </div>
      <div class="terminal-container" id="terminal-container"></div>
      <div class="arrow-bar" id="arrow-bar">
        <button class="arrow-btn toggle-btn" data-toggle="ctrl" id="btn-ctrl">Ctrl</button>
        <button class="arrow-btn toggle-btn" data-toggle="shift" id="btn-shift">Shift</button>
        <button class="arrow-btn" data-key="tab">Tab</button>
        <button class="arrow-btn" data-key="up">&uarr;</button>
        <button class="arrow-btn" data-key="down">&darr;</button>
        <button class="arrow-btn" data-key="left">&larr;</button>
        <button class="arrow-btn" data-key="right">&rarr;</button>
        <button class="arrow-btn" data-key="esc">Esc</button>
        <button class="arrow-btn" data-key="enter">Enter</button>
      </div>
    </div>
  `;

  document.getElementById("term-back").addEventListener("click", closeTerminal);
  document.getElementById("voice-toggle").addEventListener("click", toggleVoiceMode);

  // Upload button for mobile (guarded singleton)
  const termUploadBtn = document.getElementById("term-upload-btn");
  if (termUploadBtn) {
    if (!document.getElementById("upload-input")) {
      const inp = document.createElement("input");
      inp.type = "file";
      inp.id = "upload-input";
      inp.multiple = true;
      inp.style.display = "none";
      document.body.appendChild(inp);
      inp.addEventListener("change", () => {
        if (inp.files.length && activeCardId) {
          uploadFiles(inp.files, activeCardId);
        }
        inp.value = "";
      });
    }
    termUploadBtn.addEventListener("click", () => {
      document.getElementById("upload-input").click();
    });
  }

  // Modifier toggle state
  let ctrlActive = false;
  let shiftActive = false;
  const btnCtrl = document.getElementById("btn-ctrl");
  const btnShift = document.getElementById("btn-shift");

  btnCtrl.addEventListener("click", (e) => {
    e.preventDefault();
    ctrlActive = !ctrlActive;
    btnCtrl.classList.toggle("toggle-active", ctrlActive);
  });
  btnShift.addEventListener("click", (e) => {
    e.preventDefault();
    shiftActive = !shiftActive;
    btnShift.classList.toggle("toggle-active", shiftActive);
  });

  // Arrow/action key buttons
  const arrowKeys = {
    up: "\x1b[A",
    down: "\x1b[B",
    left: "\x1b[D",
    right: "\x1b[C",
    tab: "\t",
    esc: "\x1b",
    enter: "\r",
  };
  document.querySelectorAll(".arrow-btn:not(.toggle-btn)").forEach((btn) => {
    const key = btn.dataset.key;
    const seq = arrowKeys[key];
    if (!seq) return;
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      if (ws && ws.readyState === WebSocket.OPEN) {
        let data = seq;
        // Ctrl modifier: convert printable keys to control codes
        if (ctrlActive) {
          if (key === "enter") data = "\r";
          else if (seq.length === 1) {
            // Ctrl+letter: convert to control code (e.g. Ctrl+C = \x03)
            const code = seq.toUpperCase().charCodeAt(0);
            if (code >= 65 && code <= 90) data = String.fromCharCode(code - 64);
          }
          // Auto-release ctrl after use
          ctrlActive = false;
          btnCtrl.classList.remove("toggle-active");
        }
        // Shift modifier: uppercase arrow keys use different sequences
        if (shiftActive) {
          const shiftArrows = {
            up: "\x1b[1;2A",
            down: "\x1b[1;2B",
            left: "\x1b[1;2D",
            right: "\x1b[1;2C",
          };
          if (shiftArrows[key]) data = shiftArrows[key];
          // Shift stays active (toggle style) — user releases manually
        }
        ws.send(JSON.stringify({ type: "input", data }));
      }
    });
  });

  // Create xterm instance if not exists
  if (!termInstance) {
    termInstance = new Terminal({
      disableStdin: false,
      cursorBlink: true,
      fontSize: 12,
      scrollback: 10000,
      fontFamily: '"SF Mono", "Menlo", "Monaco", monospace',
      theme: {
        background: "#0a0a0a",
        foreground: "#e0e0e0",
        cursor: "#e0e0e0",
      },
    });
    fitAddon = new FitAddon();
    termInstance.loadAddon(fitAddon);
    termInstance.loadAddon(new WebLinksAddon());
  }

  const container = document.getElementById("terminal-container");
  termInstance.open(container);

  // Wire input to WS (Phase 4: bidirectional)
  termInstance.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      let out = data;
      // Apply Ctrl modifier to keyboard input (e.g. type 'c' with Ctrl active → Ctrl+C)
      if (ctrlActive && data.length === 1) {
        const code = data.toUpperCase().charCodeAt(0);
        if (code >= 65 && code <= 90) {
          out = String.fromCharCode(code - 64);
        }
        ctrlActive = false;
        btnCtrl.classList.remove("toggle-active");
      }
      ws.send(JSON.stringify({ type: "input", data: out }));
    }
  });

  // Don't call fitAddon.fit() — wait for desktop dimensions from buffer message.
  // Phone locks grid to desktop cols/rows and scales font to fit.

  // ResizeObserver: laptop uses fitAddon to fit container, mobile uses font scaling
  if (resizeObserver) resizeObserver.disconnect();
  const isLaptop = window.innerWidth >= 768;
  resizeObserver = new ResizeObserver(() => {
    if (isLaptop) {
      // Laptop: use fitAddon to compute cols/rows from container dimensions
      if (container.clientWidth > 0 && container.clientHeight > 0 && fitAddon) {
        requestAnimationFrame(() => {
          fitAddon.fit();
          const dims = fitAddon.proposeDimensions();
          if (dims && dims.cols > 0 && dims.rows > 0) {
            sendResizeMsg(dims.cols, dims.rows);
          }
        });
      }
    } else {
      // Mobile: existing font-scaling path
      if (desktopCols && desktopRows) {
        requestAnimationFrame(() => scaleTerminal());
      }
    }
  });
  resizeObserver.observe(container);

  // Restore voice panel if voice mode was active before re-render
  if (voiceMode) {
    insertVoicePanel();
    termInstance.options.disableStdin = true;
    if (termInstance.textarea) termInstance.textarea.blur();
  }
}

// --- Voice Input Mode ---

function toggleVoiceMode() {
  voiceMode = !voiceMode;

  const btn = document.getElementById("voice-toggle");
  if (btn) btn.classList.toggle("active", voiceMode);

  if (voiceMode) {
    if (termInstance) {
      termInstance.options.disableStdin = true;
      if (termInstance.textarea) termInstance.textarea.blur();
    }
    insertVoicePanel();
    // Focus textarea synchronously within click handler for iOS keyboard
    const ta = document.getElementById("voice-textarea");
    if (ta) ta.focus();
  } else {
    removeVoicePanel();
    if (termInstance) {
      termInstance.options.disableStdin = false;
    }
  }
}

function insertVoicePanel() {
  if (document.getElementById("voice-panel")) return;

  const panel = document.createElement("div");
  panel.id = "voice-panel";
  panel.className = "voice-panel";
  panel.innerHTML = `
    <textarea id="voice-textarea" class="voice-textarea" rows="3"
      placeholder="Tap microphone on keyboard to dictate..."
      autocomplete="off" autocorrect="on" autocapitalize="sentences"
      spellcheck="true"></textarea>
    <div class="voice-actions">
      <button class="btn-secondary voice-clear-btn" id="voice-clear">Clear</button>
      <button class="btn-primary voice-send-btn" id="voice-send">Send</button>
    </div>
  `;

  const header = document.querySelector(".terminal-header");
  const container = document.getElementById("terminal-container");
  if (header && container) {
    header.after(panel);
  }

  document.getElementById("voice-send").addEventListener("click", sendVoiceInput);
  document.getElementById("voice-clear").addEventListener("click", clearVoiceInput);
}

function removeVoicePanel() {
  const panel = document.getElementById("voice-panel");
  if (panel) panel.remove();
}

function sendVoiceInput() {
  const ta = document.getElementById("voice-textarea");
  if (!ta) return;

  const text = ta.value;
  if (!text) return;

  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: "input", data: text + "\n" }));
  }

  // Close voice mode and return to terminal
  toggleVoiceMode();
}

function clearVoiceInput() {
  const ta = document.getElementById("voice-textarea");
  if (!ta) return;
  ta.value = "";
  ta.focus();
}

function sendResizeMsg(cols, rows) {
  if (ws && ws.readyState === WebSocket.OPEN && cols > 0 && rows > 0) {
    ws.send(JSON.stringify({ type: "resize", cols, rows }));
  }
}

function scaleTerminal() {
  if (!termInstance || !desktopCols || !desktopRows) return;
  const container = document.getElementById("terminal-container");
  if (!container || container.clientWidth === 0 || container.clientHeight === 0) return;

  // Calculate font size to fit desktop grid in phone container.
  // Monospace char width ≈ fontSize * 0.602 (SF Mono / Menlo).
  const CHAR_RATIO = 0.602;
  const maxByWidth = container.clientWidth / (desktopCols * CHAR_RATIO);
  const maxByHeight = container.clientHeight / desktopRows;
  const fontSize = Math.max(4, Math.floor(Math.min(maxByWidth, maxByHeight)));

  termInstance.options.fontSize = fontSize;
  termInstance.resize(desktopCols, desktopRows);
}

function closeTerminal() {
  cleanupTerminal();
  if (currentView === "split") {
    activeCardId = null;
    const panel = document.querySelector(".split-panel");
    if (panel) panel.innerHTML = `<div class="split-empty">Select a session</div>`;
    document.querySelectorAll(".sidebar-session-entry").forEach(el => {
      el.classList.remove("sidebar-entry-active");
    });
    return;
  }
  currentView = "board";
  window.location.hash = "board";
  startPolling();
}

function cleanupTerminal() {
  if (ws) {
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
    ws = null;
  }
  if (resizeObserver) {
    resizeObserver.disconnect();
    resizeObserver = null;
  }
  if (termInstance) {
    termInstance.dispose();
    termInstance = null;
    fitAddon = null;
  }
  currentSessionId = null;
  currentCardName = null;
  bufferSeq = 0;
  desktopCols = null;
  desktopRows = null;
  voiceMode = false;
}

function updateTermStatus(newStatus) {
  termStatus = newStatus;
  // Update status indicator in header without re-rendering full terminal
  const dot = document.querySelector(".terminal-status .status-dot");
  const label = document.querySelector(".terminal-status .status-label");
  if (!dot || !label) return;

  dot.className = "status-dot " + (
    newStatus === "live" ? "status-connected"
    : newStatus === "ended" ? "status-offline"
    : newStatus === "disconnected" ? "status-unreachable"
    : "status-unreachable"
  );
  label.textContent = newStatus === "live" ? "Live"
    : newStatus === "ended" ? "Session ended"
    : newStatus === "disconnected" ? "Disconnected"
    : "Connecting...";
}

// --- WebSocket ---

function connectWs(sessionId) {
  if (ws) {
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
    ws = null;
  }

  const token = getToken();
  if (!token) {
    cleanupTerminal();
    clearToken();
    renderScanScreen("Session expired — please reconnect.");
    return;
  }

  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${window.location.host}/sessions/${sessionId}/stream?token=${encodeURIComponent(token)}`;

  updateTermStatus("connecting");
  ws = new WebSocket(url);

  ws.onopen = () => {
    // Wait for buffer message to set "live"
  };

  ws.onmessage = (event) => {
    let msg;
    try {
      msg = JSON.parse(event.data);
    } catch {
      return;
    }

    switch (msg.type) {
      case "buffer":
        if (msg.cols && msg.rows) {
          desktopCols = msg.cols;
          desktopRows = msg.rows;
        }
        if (termInstance) {
          if (window.innerWidth >= 768 && fitAddon) {
            // Laptop: fit to container, let ResizeObserver handle
            // Don't call scaleTerminal — the ResizeObserver will fire fit() + sendResizeMsg()
          } else if (desktopCols && desktopRows) {
            scaleTerminal();
          }
          termInstance.write(msg.data);
          termInstance.focus();
        }
        bufferSeq = msg.seq;
        updateTermStatus("live");
        break;

      case "output":
        if (msg.seq <= bufferSeq) break; // Skip already-buffered chunks
        if (termInstance) termInstance.write(msg.data);
        break;

      case "exit":
        if (termInstance) termInstance.write("\r\n\x1b[90m[Session ended]\x1b[0m\r\n");
        updateTermStatus("ended");
        break;

      case "resize":
        if (msg.cols && msg.rows) {
          desktopCols = msg.cols;
          desktopRows = msg.rows;
          if (window.innerWidth < 768) {
            scaleTerminal();
          }
          // On laptop, the ResizeObserver + fitAddon handles sizing — ignore server resize messages
        }
        break;

      case "lag":
        // Reconnect with fresh buffer
        if (ws) ws.close();
        ws = null;
        if (termInstance) termInstance.clear();
        setTimeout(() => {
          if ((currentView === "terminal" || currentView === "split") && currentSessionId === sessionId) {
            connectWs(sessionId);
          }
        }, WS_RECONNECT_MS);
        break;

      case "error":
        if (termInstance) {
          termInstance.write(`\r\n\x1b[31m${msg.message || "Unknown error"}\x1b[0m\r\n`);
        }
        updateTermStatus(msg.message?.includes("ended") ? "ended" : "disconnected");
        break;
    }
  };

  ws.onclose = () => {
    if (termStatus !== "ended") {
      updateTermStatus("disconnected");
    }
  };

  ws.onerror = () => {
    updateTermStatus("disconnected");
  };
}

// --- API ---

async function pair(key) {
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

async function fetchBootstrapStatus() {
  const token = getToken();
  const res = await timedFetch("/bootstrap/status", {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function triggerBootstrap() {
  const token = getToken();
  const res = await timedFetch("/bootstrap", {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error("auth");
  return { status: res.status, data: await res.json() };
}

async function fetchSessions() {
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

async function checkHealth() {
  try {
    const res = await timedFetch("/health");
    if (res.ok) {
      connectionState = "connected";
    } else {
      connectionState = "unreachable";
    }
  } catch {
    connectionState = navigator.onLine ? "unreachable" : "offline";
  }
}

// --- Polling ---

function startPolling() {
  stopPolling();
  if (!_brandingLoaded) loadBranding();   // pull NCC name + accent once we're authed

  async function pollSessions() {
    try {
      const sessions = await fetchSessions();
      lastSessions = sessions;
      if (currentView === "board" && !uiModalOpen) renderBoard(sessions);
      else if (currentView === "split" && !uiModalOpen) updateSidebar(sessions);
      else if (currentView === "dashboard" && !uiModalOpen) updateDashboard(sessions);
    } catch (e) {
      if (e.message === "auth") {
        stopPolling();
        clearToken();
        renderScanScreen("Session expired — please reconnect.");
        return;
      }
      if (lastSessions && currentView === "board" && !uiModalOpen) {
        renderBoard(lastSessions);
      } else if (lastSessions && currentView === "split" && !uiModalOpen) {
        updateSidebar(lastSessions);
      } else if (lastSessions && currentView === "dashboard" && !uiModalOpen) {
        updateDashboard(lastSessions);
      }
    }
    sessionPollTimer = setTimeout(pollSessions, SESSION_POLL_MS);
  }

  async function pollHealth() {
    await checkHealth();
    if (lastSessions && currentView === "board" && !uiModalOpen) {
      renderBoard(lastSessions);
    } else if (lastSessions && currentView === "split" && !uiModalOpen) {
      updateSidebar(lastSessions);
    } else if (lastSessions && currentView === "dashboard" && !uiModalOpen) {
      updateDashboard(lastSessions);
    }
    healthPollTimer = setTimeout(pollHealth, HEALTH_POLL_MS);
  }

  pollSessions();
  pollHealth();
}

function stopPolling() {
  if (sessionPollTimer) {
    clearTimeout(sessionPollTimer);
    sessionPollTimer = null;
  }
  if (healthPollTimer) {
    clearTimeout(healthPollTimer);
    healthPollTimer = null;
  }
  stopInboxPoll();
}

// --- iOS background/foreground reconnection (Step 5) ---

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;

  if ((currentView === "terminal" || currentView === "split") && currentSessionId && termStatus !== "ended") {
    // Reconnect with fresh buffer on foreground return
    if (ws) ws.close();
    ws = null;
    if (termInstance) {
      termInstance.clear();
      termInstance.write("\x1b[90m[Reconnecting...]\x1b[0m");
    }
    connectWs(currentSessionId);
  }
});

// --- Hash routing ---

function handleHash() {
  const hash = window.location.hash.slice(1);

  if (hash === "dashboard") {
    if (currentView === "terminal" || currentView === "split") cleanupTerminal();
    currentView = "dashboard";
    setStoredViewMode("dashboard");
    if (lastSessions) renderDashboard(lastSessions);
    startPolling();
    return;
  }

  if (hash === "split") {
    currentView = "split";
    setStoredViewMode("split");
    if (lastSessions) renderSplitLayout(lastSessions);
    startPolling();
    return;
  }

  if (hash === "board" || hash === "") {
    if (currentView === "terminal") {
      cleanupTerminal();
      currentView = "board";
      setStoredViewMode("board");
      if (lastSessions) {
        renderBoard(lastSessions);
      }
      startPolling();
    } else if (currentView === "split") {
      // Close terminal in split view but keep sidebar
      closeTerminal();
    } else if (currentView === "dashboard") {
      cleanupTerminal();
      currentView = "board";
      setStoredViewMode("board");
      if (lastSessions) renderBoard(lastSessions);
      startPolling();
    }
    return;
  }

  const sessionMatch = hash.match(/^session\/(.+)$/);
  if (sessionMatch && currentView !== "terminal") {
    // Deep link to terminal — need session info from last poll
    const sid = sessionMatch[1];
    const card = lastSessions?.find((s) => s.session_id === sid);
    if (card && card.is_alive) {
      if (currentView === "split") {
        openSplitTerminal(sid, card.card_name, card.card_id);
      } else {
        openTerminal(sid, card.card_name);
      }
    } else if (currentSessionId !== sid) {
      // Session not found or dead and we're not already viewing it — go to board
      window.location.hash = "board";
    }
  }
}

window.addEventListener("hashchange", handleHash);

// --- Boot ---

async function boot() {
  // Check for hash fragment
  const hash = window.location.hash;
  if (hash && hash.length > 1) {
    const key = hash.slice(1);

    // Lens finding #2: Only treat hex strings as pairing keys (64 chars = 32-byte key)
    if (/^[0-9a-f]{64}$/i.test(key)) {
      try {
        const token = await pair(key);
        setToken(token);
        window.history.replaceState({}, "", "/");
        connectionState = "connected";
        renderLoading();
        startPolling();
        return;
      } catch (e) {
        renderErrorScreen(e.message);
        return;
      }
    }
    // Not a pairing key — will be handled as route after token check
  }

  // Check for stored token — or try Cloudflare Access (no token needed)
  const token = getToken();
  if (!token) {
    // Try fetching without token — Cloudflare Access may handle auth via cookie
    try {
      const cfTest = await timedFetch("/sessions");
      if (cfTest.ok) {
        // Cloudflare Access is handling auth — set a dummy token so the PWA works
        setToken("cf-access-authenticated");
        const sessions = await cfTest.json();
        lastSessions = sessions;
        connectionState = "connected";
        renderPreferredSessionView(sessions);
        startPolling();
        if (window.location.hash && window.location.hash.length > 1) {
          handleHash();
        }
        return;
      }
    } catch (_) {}
    renderScanScreen();
    return;
  }

  // Validate stored token
  renderLoading();
  try {
    const sessions = await fetchSessions();
    lastSessions = sessions;
    connectionState = "connected";
    renderPreferredSessionView(sessions);
    startPolling();

    // Handle route hash after board loads (e.g., #session/ID)
    if (window.location.hash && window.location.hash.length > 1) {
      handleHash();
    }

    // First-run auth checks (fire and forget — don't block boot)
    const authToken = getToken();
    const authHeaders = { Authorization: `Bearer ${authToken}` };
    Promise.all([
      timedFetch("/gh/auth", { headers: authHeaders }).then((r) => r.json()).catch(() => null),
      timedFetch("/claude/auth", { headers: authHeaders }).then((r) => r.json()).catch(() => null),
    ]).then(([ghAuth, claudeAuth]) => {
      ghAuthStatus = ghAuth;
      claudeAuthStatus = claudeAuth;
      if (lastSessions) refreshCurrentSessionView(lastSessions);
    });

    // Wake status (fire and forget — updates sidebar indicator)
    timedFetch("/wake/status", { headers: { Authorization: "Bearer " + getToken() } })
      .then(r => r.ok ? r.json() : null)
      .then(ws => { if (ws) { window._nccWakeStatus = ws; updateWakeIndicator(); } })
      .catch(() => {});

    // Usage limits (fire and forget — updates sidebar usage bars)
    fetchUsage();
    setInterval(fetchUsage, 60000); // refresh every 60s

    // Bootstrap status (fire and forget — updates badge on board)
    fetchBootstrapStatus().then((bs) => {
      bootstrapState = bs;
      if (lastSessions) refreshCurrentSessionView(lastSessions);
    }).catch(() => {});
  } catch (e) {
    if (e.message === "auth") {
      clearToken();
      renderScanScreen("Session expired — please reconnect.");
    } else {
      connectionState = navigator.onLine ? "unreachable" : "offline";
      renderErrorScreen("Mac unreachable — check Tailscale connection.");
    }
  }
}

boot();

// Real-time badge update from SSE bootstrap:state events (dispatched by api.js)
document.addEventListener("bootstrap-state-update", (e) => {
  bootstrapState = e.detail;
  const bsSection = document.querySelector(".bootstrap-section");
  if (bsSection) {
    bsSection.outerHTML = bootstrapBadgeHtml();
    const runBtn = document.getElementById("bootstrap-run-btn");
    if (runBtn) runBtn.addEventListener("click", handleBootstrapRun);
  }
});

document.addEventListener("wake-status-update", (e) => {
  if (e.detail) window._nccWakeStatus = e.detail;
  updateWakeIndicator();
});

document.addEventListener("relay-message-detected", (e) => {
  const { workspace } = e.detail;
  if (lastSessions) {
    const card = lastSessions.find(c => c.workspace_path === workspace);
    if (card) card.relay_pending_count = (card.relay_pending_count || 0) + 1;
  }
  if (!uiModalOpen) {
    if (lastSessions) refreshCurrentSessionView(lastSessions);
  }
});

document.addEventListener("relay-tap-sent", (e) => {
  const { workspace } = e.detail;
  if (lastSessions) {
    const card = lastSessions.find(c => c.workspace_path === workspace);
    if (card) card.relay_pending_count = 0;
  }
  if (!uiModalOpen) {
    if (lastSessions) refreshCurrentSessionView(lastSessions);
  }
});

document.addEventListener("relay-pending-high-water", (e) => {
  const { workspace, count } = e.detail;
  const card = lastSessions?.find(c => c.workspace_path === workspace);
  const name = card ? card.card_name : "Unknown";
  showUploadStatus(`\u26a0 ${name} has ${count}+ pending relay messages`, true);
});

// Refresh wake badge on poll completion or dispatch detection.
document.addEventListener("wake-poll-completed", () => {
  timedFetch("/wake/status", { headers: { Authorization: "Bearer " + getToken() } })
    .then(r => r.ok ? r.json() : null)
    .then(ws => { if (ws) { window._nccWakeStatus = ws; updateWakeIndicator(); } })
    .catch(() => {});
});
document.addEventListener("wake-dispatch-detected", () => {
  timedFetch("/wake/status", { headers: { Authorization: "Bearer " + getToken() } })
    .then(r => r.ok ? r.json() : null)
    .then(ws => { if (ws) { window._nccWakeStatus = ws; updateWakeIndicator(); } })
    .catch(() => {});
});
