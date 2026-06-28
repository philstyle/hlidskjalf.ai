// Shared utilities

const TOKEN_KEY = "nexuslink_token";
const SERVER_KEY = "nexuslink_server";
const FETCH_TIMEOUT_MS = 5000;

export function timedFetch(url, opts = {}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  return fetch(url, { ...opts, signal: controller.signal }).finally(() =>
    clearTimeout(timeout)
  );
}

export function getToken() {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token) {
  localStorage.setItem(TOKEN_KEY, token);
  localStorage.setItem(SERVER_KEY, window.location.origin);
}

export function clearToken() {
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(SERVER_KEY);
}

export function escapeHtml(str) {
  const el = document.createElement("span");
  el.textContent = str;
  return el.innerHTML;
}

export function sanitizeColor(color) {
  return /^#[0-9a-f]{3,8}$/i.test(color) ? color : "#6B7280";
}

export function haptic(style = "light") {
  if (!navigator.vibrate) return;
  switch (style) {
    case "light": navigator.vibrate(10); break;
    case "medium": navigator.vibrate(20); break;
    case "success": navigator.vibrate([10, 50, 10]); break;
  }
}

export function formatDuration(startedAt) {
  const ms = Date.now() - new Date(startedAt).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 60) return `Active ${mins}m`;
  const hrs = Math.floor(mins / 60);
  return `Active ${hrs}h ${mins % 60}m`;
}
