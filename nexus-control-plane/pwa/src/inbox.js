// Inbox view — Mission Control for mobile PWA

import { escapeHtml, haptic } from "./utils.js";
import { fetchInbox, fetchDispatchPrs, updateInboxItem } from "./api.js";

let _viewManager = null;
let _showBoard = null;
let _inboxItems = [];
let _activeTab = "all"; // "all" | "email" | "dispatch" | "personal"

export function initInbox({ viewManager, showBoard }) {
  _viewManager = viewManager;
  _showBoard = showBoard;
}

export function pushInboxView() {
  _activeTab = "all";
  _viewManager.push("inbox", (container) => {
    renderInboxLoading(container);
    loadAndRender(container);
  }, {
    transition: "slideLeft",
  });
}

function categorizeSource(item) {
  if (!item.from) return "personal";
  if (item.from.includes("@")) return "email";
  return "dispatch";
}

async function loadAndRender(container) {
  try {
    _inboxItems = await fetchInbox();

    // Merge dispatch PRs
    try {
      const prs = await fetchDispatchPrs();
      for (const pr of prs) {
        const body = pr.body || "";
        const actionItems = [];
        for (const line of body.split("\n")) {
          const m = line.match(/^- \[([ x])\] (.+)$/);
          if (m) actionItems.push({ checked: m[1] === "x", text: m[2] });
        }
        const labels = pr.labels || [];
        _inboxItems.push({
          id: `dispatch-${pr.number}`,
          filename: `dispatch-${pr.number}`,
          filepath: pr.url || "",
          from: pr.author?.login || "",
          date: (pr.createdAt || "").slice(0, 10),
          subject: pr.title || "",
          category: "dispatch",
          priority: labels.some((l) => l.name === "P0") ? "P0"
            : labels.some((l) => l.name === "P1") ? "P1" : "P2",
          workstream: (labels.find((l) => !l.name.startsWith("P")) || {}).name || "",
          due_date: "",
          waiting_on: "",
          status: "not-started",
          summary: body.split("\n").filter((l) => l.trim()).slice(0, 2).join(" ").slice(0, 200),
          action_items: actionItems,
        });
      }
    } catch { /* dispatch fetch failed — continue with inbox only */ }

    renderInboxContent(container);
  } catch {
    container.innerHTML = `
      <div class="inbox-header">
        <button class="inbox-back" id="inbox-back">&larr;</button>
        <span class="inbox-title">Mission Control</span>
      </div>
      <div class="screen">
        <p class="muted">Could not load inbox</p>
        <button class="inbox-retry" id="inbox-retry">Retry</button>
      </div>
    `;
    container.querySelector("#inbox-back")?.addEventListener("click", () => _viewManager.pop());
    container.querySelector("#inbox-retry")?.addEventListener("click", () => loadAndRender(container));
  }
}

function renderInboxLoading(container) {
  container.innerHTML = `
    <div class="inbox-header">
      <button class="inbox-back" id="inbox-back">&larr;</button>
      <span class="inbox-title">Mission Control</span>
    </div>
    <div class="inbox-list">
      <div class="inbox-item skeleton"><div class="skeleton-line skeleton-short"></div><div class="skeleton-line skeleton-long"></div></div>
      <div class="inbox-item skeleton"><div class="skeleton-line skeleton-short"></div><div class="skeleton-line skeleton-long"></div></div>
      <div class="inbox-item skeleton"><div class="skeleton-line skeleton-short"></div><div class="skeleton-line skeleton-long"></div></div>
    </div>
  `;
  container.querySelector("#inbox-back")?.addEventListener("click", () => _viewManager.pop());
}

function renderInboxContent(container) {
  if (_inboxItems.length === 0) {
    container.innerHTML = `
      <div class="inbox-header">
        <button class="inbox-back" id="inbox-back">&larr;</button>
        <span class="inbox-title">Mission Control</span>
      </div>
      <div class="screen">
        <p class="muted">Inbox clear</p>
      </div>
    `;
    container.querySelector("#inbox-back")?.addEventListener("click", () => _viewManager.pop());
    return;
  }

  // Categorize items
  const counts = { all: _inboxItems.length, email: 0, dispatch: 0, personal: 0 };
  for (const item of _inboxItems) {
    counts[categorizeSource(item)]++;
  }

  const filteredItems = _activeTab === "all"
    ? _inboxItems
    : _inboxItems.filter((i) => categorizeSource(i) === _activeTab);

  const now = new Date();
  const overdueCount = _inboxItems.filter((i) =>
    i.due_date && new Date(i.due_date + "T00:00:00") < now
  ).length;
  const openActions = _inboxItems.reduce(
    (sum, i) => sum + i.action_items.filter((a) => !a.checked).length, 0
  );

  let statsHtml = "";
  if (overdueCount > 0) statsHtml += `<span class="inbox-stat inbox-stat-overdue">${overdueCount} overdue</span>`;
  if (openActions > 0) statsHtml += `<span class="inbox-stat">${openActions} open actions</span>`;

  const tabs = [
    { key: "all", label: "All" },
    { key: "email", label: "Inbound" },
    { key: "dispatch", label: "Dispatch" },
    { key: "personal", label: "Personal" },
  ];

  const tabsHtml = tabs
    .filter((t) => t.key === "all" || counts[t.key] > 0)
    .map((t) => `<button class="inbox-tab ${_activeTab === t.key ? "active" : ""}" data-tab="${t.key}">${t.label} (${counts[t.key]})</button>`)
    .join("");

  const itemsHtml = filteredItems.map((item, idx) => inboxItemHtml(item, idx)).join("");

  container.innerHTML = `
    <div class="inbox-header">
      <button class="inbox-back" id="inbox-back">&larr;</button>
      <span class="inbox-title">Mission Control</span>
      <span class="inbox-count">${_inboxItems.length}</span>
    </div>
    ${statsHtml ? `<div class="inbox-stats">${statsHtml}</div>` : ""}
    <div class="inbox-tabs-row">${tabsHtml}</div>
    <div class="inbox-list">
      ${itemsHtml.length > 0 ? itemsHtml : '<div class="screen"><p class="muted">No items in this category</p></div>'}
    </div>
  `;

  // Wire back button
  container.querySelector("#inbox-back")?.addEventListener("click", () => _viewManager.pop());

  // Wire tabs
  container.querySelectorAll(".inbox-tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      _activeTab = tab.dataset.tab;
      renderInboxContent(container);
    });
  });

  // Wire expand/collapse
  container.querySelectorAll(".inbox-item-row").forEach((row) => {
    row.addEventListener("click", () => {
      const detail = row.nextElementSibling;
      const isOpen = detail?.classList.contains("open");
      container.querySelectorAll(".inbox-detail.open").forEach((d) => d.classList.remove("open"));
      container.querySelectorAll(".inbox-item-row.expanded").forEach((r) => r.classList.remove("expanded"));
      if (!isOpen && detail) {
        detail.classList.add("open");
        row.classList.add("expanded");
      }
    });
  });

  // Wire checkboxes
  container.querySelectorAll(".inbox-checkbox").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const filename = btn.dataset.filename;
      const actionIdx = parseInt(btn.dataset.actionIdx, 10);
      haptic("light");
      try {
        await updateInboxItem(filename, "toggle_action", actionIdx);
        const item = _inboxItems.find((i) => i.filename === filename);
        if (item && item.action_items[actionIdx]) {
          item.action_items[actionIdx].checked = !item.action_items[actionIdx].checked;
        }
        renderInboxContent(container);
      } catch { /* silent */ }
    });
  });

  // Wire dismiss buttons
  container.querySelectorAll(".inbox-dismiss").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const filename = btn.dataset.filename;
      haptic("success");
      try {
        await updateInboxItem(filename, "dismiss");
        _inboxItems = _inboxItems.filter((i) => i.filename !== filename);
        renderInboxContent(container);
      } catch { /* silent */ }
    });
  });
}

function inboxItemHtml(item) {
  const priorityColors = {
    P0: "var(--nx-red)",
    P1: "var(--nx-yellow)",
    P2: "var(--nx-accent)",
    P3: "var(--nx-muted)",
  };
  const pColor = priorityColors[item.priority] || priorityColors.P3;

  const dueDateStr = formatDueDate(item.due_date);
  const isOverdue = dueDateStr === "overdue";
  const isDueToday = dueDateStr === "today";
  const dueClass = isOverdue ? "inbox-due-overdue" : isDueToday ? "inbox-due-today" : "inbox-due";

  const totalActions = item.action_items.length;
  const checkedActions = item.action_items.filter((a) => a.checked).length;
  const allDone = totalActions > 0 && checkedActions === totalActions;

  const actionsHtml = item.action_items.map((a, i) => `
    <button class="inbox-checkbox" data-filename="${escapeHtml(item.filename)}" data-action-idx="${i}">
      <span class="inbox-check-icon ${a.checked ? "checked" : ""}">${a.checked ? "&#9745;" : "&#9744;"}</span>
      <span class="inbox-check-text ${a.checked ? "checked" : ""}">${escapeHtml(a.text)}</span>
    </button>
  `).join("");

  return `
    <div class="inbox-item ${allDone ? "inbox-item-done" : ""}">
      <button class="inbox-item-row">
        <span class="inbox-priority" style="color: ${pColor}; background: ${pColor}15">${escapeHtml(item.priority)}</span>
        <span class="inbox-subject ${allDone ? "line-through" : ""}">${escapeHtml(item.subject)}</span>
        ${item.waiting_on ? '<span class="inbox-waiting">waiting</span>' : ''}
        ${dueDateStr ? `<span class="${dueClass}">${dueDateStr}</span>` : ''}
        ${totalActions > 0 ? `<span class="inbox-progress ${allDone ? "inbox-progress-done" : ""}">${checkedActions}/${totalActions}</span>` : ''}
        <span class="inbox-chevron">&#x276F;</span>
      </button>
      <div class="inbox-detail">
        ${item.summary ? `<p class="inbox-summary">${escapeHtml(item.summary)}</p>` : ''}
        ${item.from ? `<div class="inbox-meta">From: ${escapeHtml(item.from)} &middot; ${escapeHtml(item.date)}</div>` : ''}
        ${item.workstream ? `<div class="inbox-meta">Workstream: ${escapeHtml(item.workstream)}</div>` : ''}
        ${actionsHtml ? `<div class="inbox-actions-list">${actionsHtml}</div>` : ''}
        <button class="inbox-dismiss ${allDone ? "inbox-dismiss-done" : ""}" data-filename="${escapeHtml(item.filename)}">
          ${allDone ? "Done" : "Dismiss"}
        </button>
      </div>
    </div>
  `;
}

function formatDueDate(due) {
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
