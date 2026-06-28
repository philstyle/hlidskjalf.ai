// Wizard — new session creation bottom sheet (single-page form)

import { timedFetch, getToken, escapeHtml } from "./utils.js";
import { state } from "./state.js";
import { fetchSessions } from "./api.js";

let _viewManager = null;
let _openTerminal = null;

export function initWizard({ viewManager, openTerminal }) {
  _viewManager = viewManager;
  _openTerminal = openTerminal;
}

// Wizard state
let wizardLoading = false;
let wizardName = "", wizardLaneId = "", wizardNotes = "";
let wizardSourceType = "github";
let wizardLocalPath = "", wizardSelectedRepo = null;
let wizardLanes = [], wizardRepos = [], wizardRepoSearch = "";
let wizardOrg = "";
let wizardError = null, wizardCreating = false;
let wizardReposLoading = false;
let wizardContainer = null;

export async function openWizard() {
  wizardError = null;
  wizardCreating = false;
  wizardLoading = true;
  wizardName = "";
  wizardNotes = "";
  wizardSourceType = "github";
  wizardLocalPath = "";
  wizardSelectedRepo = null;
  wizardRepoSearch = "";
  wizardRepos = [];
  wizardReposLoading = false;
  wizardLanes = [];
  wizardOrg = "";

  // Show the sheet immediately with a loading state
  _viewManager.present("wizard", (container) => {
    wizardContainer = container;
    renderWizardContent();
  }, {
    onCleanup: () => {
      wizardContainer = null;
      resetWizard();
    },
  });

  // Fetch data in the background
  const token = getToken();
  const headers = { Authorization: `Bearer ${token}` };

  try {
    const [lanesRes, defaultLaneRes, orgRes] = await Promise.all([
      timedFetch("/lanes", { headers }),
      timedFetch("/settings/default_lane_id", { headers }).catch(() => null),
      timedFetch("/settings/github_org", { headers }).catch(() => null),
    ]);

    if (!lanesRes.ok) throw new Error("Failed to load lanes");
    wizardLanes = await lanesRes.json();

    if (defaultLaneRes && defaultLaneRes.ok) {
      const d = await defaultLaneRes.json();
      wizardLaneId = d.value;
    }
    if (!wizardLaneId && wizardLanes.length > 0) {
      wizardLaneId = wizardLanes[0].id;
    }

    if (orgRes && orgRes.ok) {
      const o = await orgRes.json();
      wizardOrg = o.value || "";
    }

    wizardLoading = false;
    renderWizardContent();

    // Auto-load repos if GitHub source and org is set
    if (wizardOrg && wizardRepos.length === 0) {
      loadRepos();
    }
  } catch (e) {
    wizardLoading = false;
    wizardError = "Failed to load — check your connection.";
    renderWizardContent();
  }
}

function closeWizard() {
  _viewManager.dismiss();
}

function resetWizard() {
  wizardLoading = false;
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
  wizardOrg = "";
  wizardError = null;
  wizardCreating = false;
}

// --- Drag-to-dismiss ---

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

// --- Render ---

function renderWizardContent() {
  if (!wizardContainer) return;

  // Loading state — show immediately while API calls are in flight
  if (wizardLoading) {
    wizardContainer.innerHTML = `
      <div class="sheet-handle-bar"><div class="sheet-handle"></div></div>
      <div class="sheet-header">
        <h2>New Session</h2>
      </div>
      <div class="sheet-body">
        <div class="repo-loading"><span class="spinner"></span> Loading...</div>
      </div>
    `;
    return;
  }

  const laneOptions = wizardLanes
    .map(
      (l) =>
        `<option value="${escapeHtml(l.id)}"${l.id === wizardLaneId ? " selected" : ""}>${escapeHtml(l.name)}</option>`
    )
    .join("");

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
      <div class="field-group">
        <label class="field-label" for="wiz-path">Path on Mac</label>
        <input class="field-input" id="wiz-path" type="text" placeholder="/Users/you/projects/my-repo" value="${escapeHtml(wizardLocalPath)}" autocomplete="off" autocapitalize="off" />
      </div>
    `;
  }

  const errorHtml = wizardError
    ? `<div class="wizard-error">${escapeHtml(wizardError)}</div>`
    : "";

  const createDisabled = wizardCreating ? " disabled" : "";
  const createLabel = wizardCreating
    ? `<span class="spinner"></span>Creating...`
    : "Create Session";

  wizardContainer.innerHTML = `
    <div class="sheet-handle-bar"><div class="sheet-handle"></div></div>
    <div class="sheet-header">
      <h2>New Session</h2>
    </div>
    <div class="sheet-body">
      ${errorHtml}
      <div class="field-group">
        <label class="field-label" for="wiz-name">Name</label>
        <input class="field-input" id="wiz-name" type="text" placeholder="e.g. auth-refactor" value="${escapeHtml(wizardName)}" autocomplete="off" autocapitalize="off" />
      </div>
      <div class="field-group">
        <label class="field-label">Source</label>
        <div class="source-tabs">
          <button class="source-tab${ghActive}" data-source="github">GitHub</button>
          <button class="source-tab${localActive}" data-source="local">Local Path</button>
        </div>
      </div>
      <div id="wiz-source-content">
        ${sourceBody}
      </div>
      <div class="field-group">
        <label class="field-label" for="wiz-lane">Lane</label>
        <select class="field-select" id="wiz-lane">${laneOptions}</select>
      </div>
      <div class="field-group">
        <label class="field-label" for="wiz-notes">Notes (optional)</label>
        <textarea class="field-textarea" id="wiz-notes" rows="2" placeholder="What are you working on?">${escapeHtml(wizardNotes)}</textarea>
      </div>
    </div>
    <div class="sheet-footer">
      <button class="btn-primary" id="wiz-create"${createDisabled}>${createLabel}</button>
    </div>
  `;

  // Wire events
  wireWizardEvents();
  setupSheetDrag(wizardContainer, closeWizard);
}

function wireWizardEvents() {
  if (!wizardContainer) return;

  wizardContainer.querySelector("#wiz-create")?.addEventListener("click", handleCreate);

  wizardContainer.querySelectorAll(".source-tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      if (wizardCreating) return;
      const src = tab.dataset.source;
      if (src !== wizardSourceType) {
        captureFormState();
        wizardSourceType = src;
        wizardError = null;
        renderWizardContent();
        // Load repos on first switch to GitHub tab
        if (src === "github" && wizardOrg && wizardRepos.length === 0) {
          loadRepos();
        }
      }
    });
  });

  const searchEl = wizardContainer.querySelector("#wiz-repo-search");
  if (searchEl) {
    searchEl.addEventListener("input", (e) => {
      wizardRepoSearch = e.target.value;
      captureFormState();
      renderWizardContent();
      const newSearch = wizardContainer.querySelector("#wiz-repo-search");
      if (newSearch) {
        newSearch.focus();
        newSearch.setSelectionRange(wizardRepoSearch.length, wizardRepoSearch.length);
      }
    });
  }

  wizardContainer.querySelectorAll(".repo-item").forEach((el) => {
    el.addEventListener("click", () => {
      if (wizardCreating) return;
      captureFormState();
      wizardSelectedRepo = el.dataset.repo;
      wizardError = null;
      renderWizardContent();
    });
  });

  const pathEl = wizardContainer.querySelector("#wiz-path");
  if (pathEl) {
    pathEl.addEventListener("input", (e) => {
      wizardLocalPath = e.target.value;
    });
  }
}

function captureFormState() {
  if (!wizardContainer) return;
  const nameEl = wizardContainer.querySelector("#wiz-name");
  const laneEl = wizardContainer.querySelector("#wiz-lane");
  const notesEl = wizardContainer.querySelector("#wiz-notes");
  if (nameEl) wizardName = nameEl.value;
  if (laneEl) wizardLaneId = laneEl.value;
  if (notesEl) wizardNotes = notesEl.value;
}

async function loadRepos() {
  wizardReposLoading = true;
  renderWizardContent();

  const token = getToken();
  try {
    const res = await timedFetch(`/gh/repos?org=${encodeURIComponent(wizardOrg)}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (!res.ok) {
      const body = await res.json().catch(() => null);
      throw new Error(body?.error?.message || `HTTP ${res.status}`);
    }
    wizardRepos = await res.json();
  } catch (e) {
    wizardError = `Failed to load repos: ${e.message}`;
  }
  wizardReposLoading = false;
  renderWizardContent();
}

async function handleCreate() {
  if (wizardCreating) return;
  captureFormState();

  if (!wizardName.trim()) {
    wizardError = "Name is required.";
    renderWizardContent();
    return;
  }

  const body = {
    name: wizardName.trim(),
    lane_id: wizardLaneId,
    notes: wizardNotes.trim() || null,
    source_type: wizardSourceType,
  };

  if (wizardSourceType === "github") {
    if (!wizardSelectedRepo) {
      wizardError = "Select a repo.";
      renderWizardContent();
      return;
    }
    body.repo_full_name = wizardSelectedRepo;
  } else {
    if (!wizardLocalPath.trim()) {
      wizardError = "Path is required.";
      renderWizardContent();
      return;
    }
    body.local_path = wizardLocalPath.trim();
  }

  wizardCreating = true;
  wizardError = null;
  renderWizardContent();

  const token = getToken();
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 120000);
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

    const sessionId = data.session_id;
    const cardName = data.card_name;
    closeWizard();

    if (sessionId) {
      fetchSessions()
        .then((s) => { state.lastSessions = s; })
        .catch(() => {});
      _openTerminal(sessionId, cardName);
    }
  } catch (e) {
    wizardCreating = false;
    wizardError = e.name === "AbortError" ? "Request timed out — clone may still be running." : e.message;
    renderWizardContent();
  }
}
