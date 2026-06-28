// Terminal view — rendering, WS connection, voice input, scaling, gestures

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { escapeHtml, getToken, clearToken, haptic } from "./utils.js";
import { state } from "./state.js";
import { setupSwipeBack } from "./gestures.js";

const WS_RECONNECT_MS = 500;

// Terminal state
let currentSessionId = null;
let currentCardName = null;
let termInstance = null;
let fitAddon = null;
let ws = null;
let bufferSeq = 0;
let termStatus = "connecting";
let resizeObserver = null;
let desktopCols = null;
let desktopRows = null;
let voiceMode = false;
let ctrlActive = false;
let toolbarExpanded = false;

let _viewManager = null;
let _renderScanScreen = null;

export function initTerminal({ viewManager, renderScanScreen }) {
  _viewManager = viewManager;
  _renderScanScreen = renderScanScreen;
}

export function getCurrentSessionId() { return currentSessionId; }
export function getTermStatus() { return termStatus; }

// --- Session management ---

export async function startSessionAndOpen(cardId, cardName) {
  const token = getToken();
  try {
    const res = await fetch(`/cards/${cardId}/session`, {
      method: "POST",
      headers: {
        "Authorization": `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ cols: 80, rows: 24 }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      alert(err?.error?.message || "Failed to start session");
      return;
    }
    const data = await res.json();
    if (data.session_id) {
      haptic("success");
      openTerminal(data.session_id, cardName);
    }
  } catch (e) {
    alert("Failed to start session: " + e.message);
  }
}

export function openTerminal(sessionId, cardName) {
  currentSessionId = sessionId;
  currentCardName = cardName;
  termStatus = "connecting";
  ctrlActive = false;
  toolbarExpanded = false;
  window.location.hash = `session/${sessionId}`;

  _viewManager.push("terminal", (container) => {
    renderTerminalContent(container);
  }, {
    transition: "slideRight",
    onEnter: (container) => {
      setupSwipeBack(container, () => closeTerminal());
      connectWs(sessionId);
      if (termInstance && !voiceMode) termInstance.focus();
      handleOrientation();
      window.addEventListener("resize", handleOrientation);
    },
    onExit: () => {
      window.removeEventListener("resize", handleOrientation);
    },
    onCleanup: async () => {
      window.removeEventListener("resize", handleOrientation);
      cleanupTerminal();
    },
  });
}

// --- Render ---

function renderTerminalContent(container) {
  const statusDot = termStatus === "live" ? "status-connected"
    : termStatus === "ended" ? "status-offline"
    : termStatus === "disconnected" ? "status-unreachable"
    : "status-unreachable";
  const statusLabel = termStatus === "live" ? "Live"
    : termStatus === "ended" ? "Session ended"
    : termStatus === "disconnected" ? "Disconnected"
    : "Connecting...";

  container.innerHTML = `
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
        <span class="terminal-status">
          <span class="status-dot ${statusDot}"></span>
          <span class="status-label">${statusLabel}</span>
        </span>
      </div>
      <div class="terminal-container" id="terminal-container"></div>
      <div class="toolbar-wrapper${toolbarExpanded ? " expanded" : ""}" id="toolbar">
        <div class="toolbar-row toolbar-main">
          <button class="tool-btn" data-key="\t">Tab</button>
          <button class="tool-btn tool-ctrl${ctrlActive ? " active" : ""}" data-modifier="ctrl">Ctrl</button>
          <button class="tool-btn" data-key="\x1b[D">&larr;</button>
          <button class="tool-btn" data-key="\x1b[A">&uarr;</button>
          <button class="tool-btn" data-key="\x1b[B">&darr;</button>
          <button class="tool-btn" data-key="\x1b[C">&rarr;</button>
          <button class="tool-btn" data-key="\x1b">Esc</button>
          <button class="tool-btn" data-action="copy">Copy</button>
          <button class="tool-btn" data-action="paste">Paste</button>
        </div>
        <div class="toolbar-row toolbar-extra">
          <button class="tool-btn" data-key="c">C</button>
          <button class="tool-btn" data-key="d">D</button>
          <button class="tool-btn" data-key="z">Z</button>
          <button class="tool-btn" data-key="l">L</button>
          <button class="tool-btn" data-key="a">A</button>
          <button class="tool-btn" data-key="/">/</button>
          <button class="tool-btn" data-key="|">|</button>
          <button class="tool-btn" data-key="~">~</button>
        </div>
      </div>
    </div>
  `;

  container.querySelector("#term-back").addEventListener("click", closeTerminal);
  container.querySelector("#voice-toggle").addEventListener("click", toggleVoiceMode);

  // Smart toolbar events
  wireToolbar(container);

  if (!termInstance) {
    const cs = getComputedStyle(document.documentElement);
    const termBg = cs.getPropertyValue("--nx-term-bg").trim() || "#0D1B2A";
    const termFg = cs.getPropertyValue("--nx-term-fg").trim() || "#E2E6EC";
    const termCursor = cs.getPropertyValue("--nx-term-cursor").trim() || "#388BFD";
    termInstance = new Terminal({
      disableStdin: false,
      cursorBlink: true,
      fontSize: 12,
      scrollback: 10000,
      fontFamily: '"SF Mono", "Menlo", "Monaco", monospace',
      theme: {
        background: termBg,
        foreground: termFg,
        cursor: termCursor,
      },
    });
    fitAddon = new FitAddon();
    termInstance.loadAddon(fitAddon);
  }

  const termContainer = container.querySelector("#terminal-container");
  termInstance.open(termContainer);

  // Guard: dispose previous onData listener before registering a new one
  if (termInstance._nccOnData) {
    termInstance._nccOnData.dispose();
    termInstance._nccOnData = null;
  }

  // iOS dictation dedup — track last composition text to suppress duplicate onData
  let lastCompositionText = null;
  let lastCompositionTime = 0;
  const textarea = termInstance.textarea;
  if (textarea) {
    textarea.addEventListener("compositionend", (e) => {
      lastCompositionText = e.data;
      lastCompositionTime = Date.now();
    });
  }

  termInstance._nccOnData = termInstance.onData((data) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    // Suppress duplicate from iOS dictation — if onData fires with the same text
    // as a compositionend within 100ms, it's a duplicate
    if (lastCompositionText && data === lastCompositionText && Date.now() - lastCompositionTime < 100) {
      lastCompositionText = null;
      return;
    }
    lastCompositionText = null;

    if (ctrlActive && data.length === 1) {
      const code = data.toUpperCase().charCodeAt(0);
      if (code >= 65 && code <= 90) {
        ws.send(JSON.stringify({ type: "input", data: String.fromCharCode(code - 64) }));
        ctrlActive = false;
        const ctrlBtn = document.querySelector(".tool-ctrl");
        if (ctrlBtn) ctrlBtn.classList.remove("active");
        return;
      }
    }
    ws.send(JSON.stringify({ type: "input", data }));
  });

  // Double-tap fullscreen
  setupDoubleTapFullscreen(termContainer);

  if (resizeObserver) resizeObserver.disconnect();
  resizeObserver = new ResizeObserver(() => {
    if (desktopCols && desktopRows) {
      requestAnimationFrame(() => scaleTerminal());
    }
  });
  resizeObserver.observe(termContainer);

  if (voiceMode) {
    insertVoicePanel();
    termInstance.options.disableStdin = true;
    if (termInstance.textarea) termInstance.textarea.blur();
  }
}

// --- Smart Keyboard Toolbar ---

function wireToolbar(container) {
  const toolbar = container.querySelector("#toolbar");
  if (!toolbar) return;

  // Swipe up on toolbar to expand extra row
  let toolbarStartY = 0;
  toolbar.addEventListener("touchstart", (e) => {
    toolbarStartY = e.touches[0].clientY;
  }, { passive: true });
  toolbar.addEventListener("touchend", (e) => {
    const dy = toolbarStartY - e.changedTouches[0].clientY;
    if (dy > 20 && !toolbarExpanded) {
      toolbarExpanded = true;
      toolbar.classList.add("expanded");
    } else if (dy < -20 && toolbarExpanded) {
      toolbarExpanded = false;
      toolbar.classList.remove("expanded");
    }
  }, { passive: true });

  container.querySelectorAll(".tool-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      haptic("light");

      if (btn.dataset.modifier === "ctrl") {
        ctrlActive = !ctrlActive;
        btn.classList.toggle("active", ctrlActive);
        haptic("medium");
        return;
      }

      if (btn.dataset.action === "copy") {
        handleCopy();
        return;
      }
      if (btn.dataset.action === "paste") {
        handlePaste();
        return;
      }

      const key = btn.dataset.key;
      if (!key || !ws || ws.readyState !== WebSocket.OPEN) return;

      if (ctrlActive) {
        // Convert to Ctrl+key: charCode - 64 for uppercase letters
        const upper = key.toUpperCase();
        if (upper.length === 1 && upper >= "A" && upper <= "Z") {
          const charCode = upper.charCodeAt(0) - 64;
          ws.send(JSON.stringify({ type: "input", data: String.fromCharCode(charCode) }));
        } else {
          ws.send(JSON.stringify({ type: "input", data: key }));
        }
        ctrlActive = false;
        const ctrlBtn = container.querySelector(".tool-ctrl");
        if (ctrlBtn) ctrlBtn.classList.remove("active");
      } else {
        ws.send(JSON.stringify({ type: "input", data: key }));
      }
    });
  });
}

// --- Copy / Paste ---

function handleCopy() {
  if (!termInstance) return;
  const sel = termInstance.getSelection();
  if (!sel) {
    haptic("light");
    return;
  }
  navigator.clipboard.writeText(sel).then(() => {
    haptic("success");
    termInstance.clearSelection();
  }).catch(() => {});
}

function handlePaste() {
  navigator.clipboard.readText().then((text) => {
    if (text && ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "input", data: text }));
      haptic("success");
    }
  }).catch(() => {});
}

// --- Double-tap Fullscreen ---

let lastTap = 0;

function setupDoubleTapFullscreen(termContainer) {
  termContainer.addEventListener("touchend", (e) => {
    if (e.touches.length > 0) return;
    const now = Date.now();
    if (now - lastTap < 300) {
      toggleFullscreen();
    }
    lastTap = now;
  });
}

function toggleFullscreen() {
  const termView = document.querySelector(".terminal-view");
  if (!termView) return;
  termView.classList.toggle("fullscreen");
  if (desktopCols && desktopRows) {
    setTimeout(() => scaleTerminal(), 100);
  }
}

// --- Landscape Optimization ---

function handleOrientation() {
  const isLandscape = window.innerWidth > window.innerHeight;
  const termView = document.querySelector(".terminal-view");
  if (!termView) return;
  termView.classList.toggle("landscape", isLandscape);
  if (desktopCols && desktopRows) {
    setTimeout(() => scaleTerminal(), 50);
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
  if (header) {
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

  toggleVoiceMode();
}

function clearVoiceInput() {
  const ta = document.getElementById("voice-textarea");
  if (!ta) return;
  ta.value = "";
  ta.focus();
}

// --- Terminal scaling ---

function scaleTerminal() {
  if (!termInstance || !desktopCols || !desktopRows) return;
  const container = document.getElementById("terminal-container");
  if (!container || container.clientWidth === 0 || container.clientHeight === 0) return;

  const CHAR_RATIO = 0.602;
  const maxByWidth = container.clientWidth / (desktopCols * CHAR_RATIO);
  const maxByHeight = container.clientHeight / desktopRows;
  const fontSize = Math.max(4, Math.floor(Math.min(maxByWidth, maxByHeight)));

  termInstance.options.fontSize = fontSize;
  termInstance.resize(desktopCols, desktopRows);
}

function closeTerminal() {
  window.location.hash = "board";
  _viewManager.pop();
}

function cleanupTerminal() {
  if (ws) {
    ws.close();
    ws = null;
  }
  if (resizeObserver) {
    resizeObserver.disconnect();
    resizeObserver = null;
  }
  if (termInstance) {
    if (termInstance._nccOnData) {
      termInstance._nccOnData.dispose();
      termInstance._nccOnData = null;
    }
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
  ctrlActive = false;
  toolbarExpanded = false;
}

function updateTermStatus(newStatus) {
  termStatus = newStatus;
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

export function connectWs(sessionId) {
  if (ws) {
    ws.close();
    ws = null;
  }

  const token = getToken();
  if (!token) {
    cleanupTerminal();
    clearToken();
    _renderScanScreen("Session expired — scan QR to reconnect.");
    return;
  }

  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${window.location.host}/sessions/${sessionId}/stream?token=${encodeURIComponent(token)}`;

  updateTermStatus("connecting");
  ws = new WebSocket(url);

  ws.onopen = () => {};

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
          if (desktopCols && desktopRows) scaleTerminal();
          termInstance.write(msg.data);
          termInstance.scrollToBottom();
          termInstance.focus();
        }
        bufferSeq = msg.seq;
        updateTermStatus("live");
        break;

      case "output":
        if (msg.seq <= bufferSeq) break;
        if (termInstance) {
          // Only force scroll-to-bottom if user was already at bottom.
          // Respects scroll position so the user can read history while the
          // agent is still responding.
          const buf = termInstance.buffer.active;
          const wasAtBottom = buf.viewportY >= buf.baseY;
          termInstance.write(msg.data);
          if (wasAtBottom) {
            termInstance.scrollToBottom();
          }
        }
        break;

      case "exit":
        if (termInstance) termInstance.write("\r\n\x1b[90m[Session ended]\x1b[0m\r\n");
        updateTermStatus("ended");
        break;

      case "resize":
        if (msg.cols && msg.rows) {
          desktopCols = msg.cols;
          desktopRows = msg.rows;
          scaleTerminal();
        }
        break;

      case "lag":
        if (ws) ws.close();
        ws = null;
        if (termInstance) termInstance.clear();
        setTimeout(() => {
          if (_viewManager.currentViewName === "terminal" && currentSessionId === sessionId) {
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

// --- Reconnection on foreground ---

export function reconnectIfNeeded() {
  if (_viewManager.currentViewName === "terminal" && currentSessionId && termStatus !== "ended") {
    if (ws) ws.close();
    ws = null;
    if (termInstance) {
      termInstance.clear();
      termInstance.write("\x1b[90m[Reconnecting...]\x1b[0m");
    }
    connectWs(currentSessionId);
  }
}
