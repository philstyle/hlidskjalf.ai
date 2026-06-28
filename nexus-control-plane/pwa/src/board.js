// Board view — rendering, cards, header, pre-auth screens

import { escapeHtml, sanitizeColor, getToken, formatDuration, haptic } from "./utils.js";
import { setTheme, getThemeMode, isDarkActive } from "./theme.js";
import { state } from "./state.js";
import { setupPullToRefresh, setupCardSwipe } from "./gestures.js";
import { patchCard, killSession, fetchLanes } from "./api.js";
import { pushInboxView } from "./inbox.js";

// These are injected by app.js to avoid circular imports
let _viewManager = null;
let _openTerminal = null;
let _startSessionAndOpen = null;
let _openWizard = null;
let _startPolling = null;
let _stopPolling = null;
let _refreshSessions = null;

export function initBoard({ viewManager, openTerminal, startSessionAndOpen, openWizard, startPolling, stopPolling, refreshSessions }) {
  _viewManager = viewManager;
  _openTerminal = openTerminal;
  _startSessionAndOpen = startSessionAndOpen;
  _openWizard = openWizard;
  _startPolling = startPolling;
  _stopPolling = stopPolling;
  _refreshSessions = refreshSessions;
}

// --- Pre-auth Screens (bypass ViewManager) ---

export function renderScanScreen(message) {
  _viewManager.reset();
  const app = document.getElementById("app");
  app.innerHTML = `
    <div class="screen">
      <div class="logo logo-float">NX</div>
      <h1 class="scan-title">NexusLink</h1>
      <p class="muted">${message || "Point your camera at the QR code on your Mac"}</p>
    </div>
  `;
}

export function renderErrorScreen(message) {
  _viewManager.reset();
  const app = document.getElementById("app");
  app.innerHTML = `
    <div class="screen">
      <div class="logo">NX</div>
      <h1>NexusLink</h1>
      <p class="error-text">${message}</p>
    </div>
  `;
}

export function renderLoading() {
  _viewManager.reset();
  const app = document.getElementById("app");
  const skeletonCard = `
    <div class="card-v2 skeleton">
      <div class="card-indicator" style="background: var(--nx-dim)"></div>
      <div class="card-body">
        <div class="skeleton-line skeleton-short"></div>
        <div class="skeleton-line skeleton-long"></div>
      </div>
    </div>`;
  app.innerHTML = `
    ${appHeaderHtml()}
    <div class="view-content board-wrapper">
      <div class="board">
        ${skeletonCard}${skeletonCard}${skeletonCard}
      </div>
    </div>
  `;
}

// --- Board ---

export function showBoard(sessions) {
  _viewManager.update((container) => {
    renderBoardContent(container, sessions);
  });
}

function renderBoardContent(container, sessions) {
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

  const totalCount = seenOrder.reduce((n, l) => n + byLane[l].length, 0);
  const fabHtml = `<button class="fab" id="new-btn">+</button>`;

  if (totalCount === 0) {
    container.innerHTML = `
      ${appHeaderHtml()}
      <div class="view-content board-wrapper">
        <div class="empty-state">
          <div class="empty-icon">&#x1F4CB;</div>
          <p class="empty-text">No sessions yet</p>
          <p class="empty-hint">Tap + to create one</p>
        </div>
      </div>
      ${fabHtml}
    `;
    wireHeaderToggle(container);
    container.querySelector("#new-btn").addEventListener("click", _openWizard);
    return;
  }

  const allActive = state.activeLaneFilter === null ? " active" : "";
  let tabsHtml = `<button class="lane-tab${allActive}" data-lane="">All (${totalCount})</button>`;
  for (const lane of seenOrder) {
    const count = byLane[lane].length;
    const isActive = state.activeLaneFilter === lane ? " active" : "";
    tabsHtml += `<button class="lane-tab${isActive}" data-lane="${escapeHtml(lane)}">${escapeHtml(lane)} (${count})</button>`;
  }

  const filteredCards = state.activeLaneFilter
    ? (byLane[state.activeLaneFilter] || [])
    : seenOrder.flatMap((l) => byLane[l]);

  let contentHtml;
  if (filteredCards.length === 0) {
    contentHtml = `
      <div class="empty-state">
        <div class="empty-icon">&#x1F4CB;</div>
        <p class="empty-text">No sessions in ${escapeHtml(state.activeLaneFilter)}</p>
        <p class="empty-hint">Tap + to create one</p>
      </div>`;
  } else {
    contentHtml = `<div class="board">${filteredCards.map((c) => cardHtml(c)).join("")}</div>`;
  }

  container.innerHTML = `
    ${appHeaderHtml()}
    <div class="view-content board-wrapper">
      <div class="lane-tabs">${tabsHtml}</div>
      ${contentHtml}
    </div>
    ${fabHtml}
  `;

  container.querySelectorAll(".lane-tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      const lane = tab.dataset.lane;
      state.activeLaneFilter = lane || null;
      if (state.lastSessions && _viewManager.currentViewName === "board") showBoard(state.lastSessions);
    });
  });

  container.querySelectorAll(".card-clickable").forEach((el) => {
    el.addEventListener("click", () => {
      if (el._longPressTriggered) { el._longPressTriggered = false; return; }
      const sid = el.dataset.sessionId;
      const name = el.dataset.cardName;
      if (sid) _openTerminal(sid, name);
    });
  });
  container.querySelectorAll(".card-dormant").forEach((el) => {
    el.addEventListener("click", () => {
      if (el._longPressTriggered) { el._longPressTriggered = false; return; }
      const cardId = el.dataset.cardId;
      const name = el.dataset.cardName;
      if (cardId) _startSessionAndOpen(cardId, name);
    });
  });

  container.querySelector("#new-btn").addEventListener("click", _openWizard);
  wireHeaderToggle(container);

  // Cache lanes for card actions (Archived lane ID, move-to-lane picker)
  ensureLanesCache();

  // Card swipe gestures + long-press
  wireCardGestures(container);

  // Pull-to-refresh — attach to the scrollable view container (idempotent)
  if (_refreshSessions) {
    setupPullToRefresh(container, _refreshSessions);
  }
}

function wireHeaderToggle(container) {
  container.querySelector("#theme-toggle")?.addEventListener("click", () => {
    const mode = getThemeMode();
    const next = mode === "light" ? "dark" : mode === "dark" ? "system" : "light";
    setTheme(next);
    if (state.lastSessions && _viewManager.currentViewName === "board") showBoard(state.lastSessions);
  });
  container.querySelector("#inbox-btn")?.addEventListener("click", () => {
    pushInboxView();
  });
}

export function pushBoardView(sessions) {
  const app = document.getElementById("app");
  app.innerHTML = "";
  _viewManager.reset();
  _viewManager.push("board", (container) => {
    renderBoardContent(container, sessions);
  }, {
    transition: null,
    onEnter: () => { _startPolling(); },
    onExit: () => { _stopPolling(); },
  });
}

function cardHtml(card) {
  const laneColor = sanitizeColor(card.lane_color || "#6B7280");
  const token = getToken();

  let previewHtml = "";
  if (card.is_alive && card.has_preview_image && card.session_id) {
    const cacheBuster = Math.floor(Date.now() / 30000) * 30000;
    const imgUrl = `/sessions/${card.session_id}/preview?token=${encodeURIComponent(token)}&t=${cacheBuster}`;
    previewHtml = `<img class="card-preview-img" src="${imgUrl}" loading="lazy" alt="" />`;
  }

  const cls = card.is_alive && card.session_id ? "card-v2 card-clickable" : "card-v2 card-dormant";
  const dataAttrs = ` data-card-id="${escapeHtml(card.card_id)}" data-card-name="${escapeHtml(card.card_name)}"` +
    (card.session_id ? ` data-session-id="${escapeHtml(card.session_id)}"` : "");

  return `
    <div class="card-wrapper">
      <div class="${cls}"${dataAttrs}>
        <div class="card-indicator" style="background: ${laneColor}"></div>
        <div class="card-body">
          <div class="card-top">
            <span class="card-name">${escapeHtml(card.card_name)}</span>
            <span class="dot-${card.activity || (card.is_alive ? 'alive' : 'dead')}"></span>
          </div>
          ${card.ai_summary ? `<div class="card-summary">${escapeHtml(card.ai_summary)}</div>` : ''}
          ${card.is_alive && card.started_at ? `<div class="card-duration">${formatDuration(card.started_at)}</div>` : ''}
          ${previewHtml}
        </div>
      </div>
    </div>`;
}

function signalBarsHtml() {
  const lat = state.connectionLatency;
  let bars, color, label;

  if (state.connectionState === "offline") {
    bars = 0; color = "var(--nx-red)"; label = "Offline";
  } else if (state.connectionState === "unreachable") {
    bars = 0; color = "var(--nx-yellow)"; label = "Unreachable";
  } else if (lat < 100) {
    bars = 3; color = "var(--nx-green)"; label = `${lat}ms`;
  } else if (lat < 300) {
    bars = 2; color = "var(--nx-yellow)"; label = `${lat}ms`;
  } else {
    bars = 1; color = "var(--nx-red)"; label = `${lat}ms`;
  }

  const barsHtml = [1, 2, 3].map((i) =>
    `<div class="signal-bar ${bars >= i ? "active" : ""}" style="--bar-color: ${color}; height: ${6 + i * 4}px"></div>`
  ).join("");

  return `<div class="signal-bars" title="${label}">${barsHtml}</div><span class="status-label">${label}</span>`;
}

function appHeaderHtml() {
  const themeIcon = isDarkActive()
    ? `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>`
    : `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>`;
  return `
    <header class="app-header">
      <span class="app-wordmark">NexusLink</span>
      <div class="app-status">
        ${signalBarsHtml()}
      </div>
      <button class="header-btn" id="inbox-btn" title="Mission Control">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="m2.25 12 8.954-8.955a1.126 1.126 0 0 1 1.591 0L21.75 12M4.5 9.75v10.125c0 .621.504 1.125 1.125 1.125H9.75v-4.875c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125V21h4.125c.621 0 1.125-.504 1.125-1.125V9.75M8.25 21h8.25"/>
        </svg>
      </button>
      <button class="header-btn" id="theme-toggle" title="Toggle theme">${themeIcon}</button>
    </header>`;
}

// --- Lane cache ---

async function ensureLanesCache() {
  if (state.cachedLanes) return;
  try {
    state.cachedLanes = await fetchLanes();
  } catch {
    // Non-critical — actions will gracefully fail without lane data
  }
}

function getArchivedLaneId() {
  if (!state.cachedLanes) return null;
  const archived = state.cachedLanes.find((l) => l.name === "Archived");
  return archived?.id ?? null;
}

// --- Card gestures (swipe + long-press) ---

function wireCardGestures(container) {
  container.querySelectorAll(".card-v2").forEach((cardEl) => {
    const cardId = cardEl.dataset.cardId;
    const sessionId = cardEl.dataset.sessionId;
    const cardName = cardEl.dataset.cardName;
    const isAlive = cardEl.querySelector(".dot-alive, .dot-waiting") !== null;

    // Swipe gestures
    setupCardSwipe(cardEl, {
      onSwipeRight: () => archiveCard(cardId),
      onSwipeLeft: () => {
        // Long-press is a better UX for action menu on cards
        // Swipe-left opens the same action sheet
        showCardActions(cardId, sessionId, cardName, isAlive);
      },
    });

    // Long-press (300ms)
    let pressTimer = null;
    let pressStartX = 0, pressStartY = 0;

    cardEl.addEventListener("touchstart", (e) => {
      pressStartX = e.touches[0].clientX;
      pressStartY = e.touches[0].clientY;
      pressTimer = setTimeout(() => {
        pressTimer = null;
        cardEl._longPressTriggered = true;
        haptic("medium");
        showCardActions(cardId, sessionId, cardName, isAlive);
      }, 300);
    }, { passive: true });

    cardEl.addEventListener("touchmove", (e) => {
      if (!pressTimer) return;
      const dx = Math.abs(e.touches[0].clientX - pressStartX);
      const dy = Math.abs(e.touches[0].clientY - pressStartY);
      if (dx > 10 || dy > 10) {
        clearTimeout(pressTimer);
        pressTimer = null;
      }
    }, { passive: true });

    cardEl.addEventListener("touchend", () => {
      if (pressTimer) {
        clearTimeout(pressTimer);
        pressTimer = null;
      }
    }, { passive: true });

    cardEl.addEventListener("touchcancel", () => {
      if (pressTimer) {
        clearTimeout(pressTimer);
        pressTimer = null;
      }
    }, { passive: true });
  });
}

// --- Card actions ---

async function archiveCard(cardId) {
  const archivedId = getArchivedLaneId();
  if (!archivedId) return;
  try {
    await patchCard(cardId, { lane_id: archivedId });
    haptic("success");
  } catch { /* board will refresh regardless */ }
  if (_refreshSessions) _refreshSessions();
}

function showCardActions(cardId, sessionId, cardName, isAlive) {
  if (!_viewManager) return;

  _viewManager.present("card-actions", (container) => {
    let actionsHtml = `
      <div class="action-sheet-item" data-action="archive">Archive</div>`;

    if (isAlive && sessionId) {
      actionsHtml += `
      <div class="action-sheet-item destructive" data-action="kill">Kill Session</div>`;
    }

    actionsHtml += `
      <div class="action-sheet-item" data-action="move">Move to Lane...</div>
      <div class="action-sheet-item cancel" data-action="cancel">Cancel</div>`;

    container.innerHTML = `
      <div class="sheet-handle-bar"><div class="sheet-handle"></div></div>
      <div class="sheet-header">
        <h2>${escapeHtml(cardName)}</h2>
      </div>
      <div class="action-sheet">${actionsHtml}</div>
    `;

    // Wire action handlers
    container.querySelectorAll(".action-sheet-item").forEach((item) => {
      item.addEventListener("click", async () => {
        const action = item.dataset.action;
        if (action === "cancel") {
          _viewManager.dismiss();
          return;
        }
        if (action === "archive") {
          _viewManager.dismiss();
          await archiveCard(cardId);
          return;
        }
        if (action === "kill") {
          try {
            await killSession(sessionId);
            haptic("success");
          } catch { /* refresh will show current state */ }
          _viewManager.dismiss();
          if (_refreshSessions) _refreshSessions();
          return;
        }
        if (action === "move") {
          showLanePicker(container, cardId);
          return;
        }
      });
    });

    // Drag-to-dismiss
    setupSheetDrag(container, () => _viewManager.dismiss());
  });
}

function setupSheetDrag(sheetEl, onDismiss) {
  const handle = sheetEl.querySelector(".sheet-handle-bar");
  if (!handle) return;
  let startY = 0, currentY = 0;

  handle.addEventListener("touchstart", (e) => {
    startY = e.touches[0].clientY;
    sheetEl.style.transition = "none";
  }, { passive: true });

  handle.addEventListener("touchmove", (e) => {
    currentY = e.touches[0].clientY - startY;
    if (currentY > 0) {
      sheetEl.style.transform = `translateY(${currentY}px)`;
    }
  }, { passive: true });

  handle.addEventListener("touchend", () => {
    sheetEl.style.transition = "transform 0.35s cubic-bezier(0.2, 0, 0, 1)";
    if (currentY > 120) {
      onDismiss();
    } else {
      sheetEl.style.transform = "";
    }
    currentY = 0;
  }, { passive: true });
}

function showLanePicker(container, cardId) {
  if (!state.cachedLanes) return;

  const lanes = state.cachedLanes.filter((l) => l.name !== "Archived");

  let lanesHtml = lanes.map((l) =>
    `<div class="action-sheet-item" data-lane-id="${escapeHtml(l.id)}">${escapeHtml(l.name)}</div>`
  ).join("");
  lanesHtml += `<div class="action-sheet-item cancel" data-action="cancel">Cancel</div>`;

  container.innerHTML = `
    <div class="sheet-handle-bar"><div class="sheet-handle"></div></div>
    <div class="sheet-header">
      <h2>Move to Lane</h2>
    </div>
    <div class="action-sheet">${lanesHtml}</div>
  `;

  container.querySelectorAll(".action-sheet-item").forEach((item) => {
    item.addEventListener("click", async () => {
      if (item.dataset.action === "cancel") {
        _viewManager.dismiss();
        return;
      }
      const laneId = item.dataset.laneId;
      if (!laneId) return;
      try {
        await patchCard(cardId, { lane_id: laneId });
        haptic("success");
      } catch { /* refresh shows current state */ }
      _viewManager.dismiss();
      if (_refreshSessions) _refreshSessions();
    });
  });

  setupSheetDrag(container, () => _viewManager.dismiss());
}
