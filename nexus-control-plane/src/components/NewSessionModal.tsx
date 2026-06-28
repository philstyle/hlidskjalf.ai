import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useLanesStore } from "../store/lanes";
import { useCardsStore } from "../store/cards";
import { useNavStore } from "../store/nav";
import RepoPicker, { type RepoSummary } from "./RepoPicker";

interface GhAuthStatus {
  authenticated: boolean;
  username: string | null;
  error: string | null;
}

interface BranchSummary {
  name: string;
  is_default: boolean;
}

interface NewSessionModalProps {
  onClose: () => void;
}

export default function NewSessionModal({ onClose }: NewSessionModalProps) {
  const { lanes } = useLanesStore();
  const { createCard } = useCardsStore();
  const selectCard = useNavStore((s) => s.selectCard);
  const selectCardWithCommand = useNavStore((s) => s.selectCardWithCommand);
  const closeNewSessionModal = useNavStore((s) => s.closeNewSessionModal);

  // Form fields
  const [name, setName] = useState("");
  const [laneId, setLaneId] = useState("");
  const [notes, setNotes] = useState("");
  const [showDetails, setShowDetails] = useState(false);
  const [relayEnabled, setRelayEnabled] = useState(true);

  // Source
  const [sourceType, setSourceType] = useState<"github" | "local">("github");
  const [selectedRepo, setSelectedRepo] = useState<RepoSummary | null>(null);
  const [folderPath, setFolderPath] = useState("");

  // GitHub sub-state
  const [ghAuth, setGhAuth] = useState<GhAuthStatus | null>(null);
  const [repos, setRepos] = useState<RepoSummary[]>([]);
  const [reposLoading, setReposLoading] = useState(false);
  const [reposError, setReposError] = useState<string | null>(null);
  const [org, setOrg] = useState("");
  const [orgInput, setOrgInput] = useState("");
  const [orgLoaded, setOrgLoaded] = useState(false);

  // Branch selection
  const [branches, setBranches] = useState<BranchSummary[]>([]);
  const [selectedBranch, setSelectedBranch] = useState("");
  const [branchesLoading, setBranchesLoading] = useState(false);

  // Claude launch option
  const [launchClaude, setLaunchClaude] = useState(false);

  // Creation progress
  const [creating, setCreating] = useState(false);
  const [createSteps, setCreateSteps] = useState<{
    card: "pending" | "active" | "done" | "error";
    clone: "pending" | "active" | "done" | "error";
    ready: "pending" | "active" | "done" | "error";
  }>({ card: "pending", clone: "pending", ready: "pending" });
  const [error, setError] = useState<string | null>(null);

  // Set default lane from settings
  useEffect(() => {
    invoke<string | null>("get_setting", { key: "default_lane_id" }).then(
      (id) => {
        if (id && lanes.some((l) => l.id === id)) {
          setLaneId(id);
        } else if (lanes.length > 0) {
          setLaneId(lanes[0].id);
        }
      }
    );
  }, [lanes]);

  // Load saved org + eagerly check auth and load repos on mount
  useEffect(() => {
    invoke<string | null>("get_setting", { key: "github_org" }).then((val) => {
      if (val) {
        setOrg(val);
        setOrgInput(val);
        // Eagerly check auth + load repos
        invoke<GhAuthStatus>("check_gh_auth").then((status) => {
          setGhAuth(status);
          if (status.authenticated) {
            loadRepos(val);
          }
        });
      }
      setOrgLoaded(true);
    });
  }, []);

  // Load branches when a repo is selected
  useEffect(() => {
    if (!selectedRepo) {
      setBranches([]);
      setSelectedBranch("");
      return;
    }

    setBranchesLoading(true);
    invoke<BranchSummary[]>("list_branches", {
      fullName: selectedRepo.full_name,
    })
      .then((result) => {
        setBranches(result);
        const defaultBranch = result.find(
          (b) => b.name === selectedRepo.default_branch
        );
        setSelectedBranch(
          defaultBranch ? defaultBranch.name : result[0]?.name ?? ""
        );
      })
      .catch(() => {
        setBranches([]);
        setSelectedBranch(selectedRepo.default_branch);
      })
      .finally(() => setBranchesLoading(false));
  }, [selectedRepo]);

  const loadRepos = async (orgName: string) => {
    setReposLoading(true);
    setReposError(null);
    try {
      const result = await invoke<RepoSummary[]>("list_org_repos", {
        org: orgName,
      });
      setRepos(result);
    } catch (e) {
      setReposError(String(e));
    } finally {
      setReposLoading(false);
    }
  };

  const handleOrgSubmit = async () => {
    if (!orgInput.trim()) return;
    const committed = orgInput.trim();
    await invoke("set_setting", { key: "github_org", value: committed });
    setOrg(committed);
    setGhAuth(null);
    // Check auth + load repos for newly committed org
    invoke<GhAuthStatus>("check_gh_auth").then((status) => {
      setGhAuth(status);
      if (status.authenticated) {
        loadRepos(committed);
      }
    });
  };

  const handleBrowse = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setFolderPath(selected);
  };

  const handleCreate = async () => {
    setCreating(true);
    setError(null);

    try {
      if (sourceType === "github" && selectedRepo) {
        // Compute workspace path
        const workspacePath = await invoke<string>("compute_workspace_path", {
          cardName: name,
        });

        // Step 1: Clone
        setCreateSteps({ card: "pending", clone: "active", ready: "pending" });
        await invoke("clone_repo", {
          fullName: selectedRepo.full_name,
          targetPath: workspacePath,
          branch:
            selectedBranch && selectedBranch !== selectedRepo.default_branch
              ? selectedBranch
              : null,
        });
        setCreateSteps((s) => ({ ...s, clone: "done" }));

        // Step 2: Create card
        setCreateSteps((s) => ({ ...s, card: "active" }));
        const card = await createCard({
          name: name.trim(),
          lane_id: laneId,
          workspace_path: workspacePath,
          notes: notes.trim() || null,
          source_type: "github",
          repo_name: selectedRepo.full_name,
          repo_url: `https://github.com/${selectedRepo.full_name}`,
          is_app_managed: true,
        });
        setCreateSteps((s) => ({ ...s, card: "done" }));

        if (!relayEnabled) {
          try {
            await invoke("set_relay_enabled", { cardId: card.id, enabled: false });
          } catch (e) {
            console.warn("Failed to disable relay for new card:", e);
          }
        }

        // Step 3: Ready
        setCreateSteps((s) => ({ ...s, ready: "done" }));
        if (launchClaude) {
          selectCardWithCommand(card.id, "claude --dangerously-skip-permissions");
        } else {
          selectCard(card.id);
        }
        setTimeout(() => closeNewSessionModal(), 400);
      } else {
        // Local — no clone step
        setCreateSteps({ card: "active", clone: "done", ready: "pending" });
        const card = await createCard({
          name: name.trim(),
          lane_id: laneId,
          workspace_path: folderPath.trim(),
          notes: notes.trim() || null,
        });
        setCreateSteps({ card: "done", clone: "done", ready: "done" });

        if (!relayEnabled) {
          try {
            await invoke("set_relay_enabled", { cardId: card.id, enabled: false });
          } catch (e) {
            console.warn("Failed to disable relay for new card:", e);
          }
        }

        if (launchClaude) {
          selectCardWithCommand(card.id, "claude --dangerously-skip-permissions");
        } else {
          selectCard(card.id);
        }
        setTimeout(() => closeNewSessionModal(), 400);
      }
    } catch (e) {
      setError(String(e));
      setCreateSteps((s) => ({
        card: s.card === "active" ? "error" : s.card,
        clone: s.clone === "active" ? "error" : s.clone,
        ready: "pending",
      }));
      setCreating(false);
    }
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget && !creating) onClose();
  };

  // Can create?
  const canCreate =
    name.trim().length > 0 &&
    (sourceType === "github"
      ? selectedRepo !== null
      : folderPath.trim().length > 0);

  const selectedLane = lanes.find((l) => l.id === laneId);

  const stepIcon = (status: "pending" | "active" | "done" | "error") => {
    if (status === "done")
      return (
        <span className="w-5 h-5 rounded-full bg-nx-success/20 text-nx-success flex items-center justify-center text-xs">
          &#10003;
        </span>
      );
    if (status === "active")
      return (
        <div className="w-5 h-5 border-2 border-nx-accent border-t-transparent rounded-full animate-spin" />
      );
    if (status === "error")
      return (
        <span className="w-5 h-5 rounded-full bg-red-500/20 text-red-400 flex items-center justify-center text-xs">
          !
        </span>
      );
    return (
      <span className="w-5 h-5 rounded-full bg-nx-bg text-nx-dim flex items-center justify-center text-[10px]">
        &#8226;
      </span>
    );
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={handleBackdropClick}
    >
      <div className="w-[500px] max-h-[80vh] flex flex-col bg-nx-surface rounded-2xl border border-nx-border shadow-nx-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-nx-border-light">
          <h2 className="text-sm font-semibold text-nx-text">New Session</h2>
          <button
            onClick={onClose}
            disabled={creating}
            className="text-nx-muted hover:text-nx-text transition-colors text-lg leading-none disabled:opacity-30"
          >
            &times;
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {!creating ? (
            <div className="flex flex-col gap-4">
              {/* Name */}
              <div>
                <label className="block text-xs text-nx-muted mb-1.5">
                  Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="e.g. auth-refactor"
                  autoFocus
                  className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                    placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50"
                />
              </div>

              {/* Source tabs */}
              <div>
                <label className="block text-xs text-nx-muted mb-1.5">
                  Source
                </label>
                <div className="flex border border-nx-border rounded overflow-hidden">
                  <button
                    onClick={() => setSourceType("github")}
                    className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
                      sourceType === "github"
                        ? "bg-nx-accent text-white"
                        : "bg-nx-bg text-nx-muted hover:text-nx-text"
                    }`}
                  >
                    GitHub
                  </button>
                  <button
                    onClick={() => setSourceType("local")}
                    className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
                      sourceType === "local"
                        ? "bg-nx-accent text-white"
                        : "bg-nx-bg text-nx-muted hover:text-nx-text"
                    }`}
                  >
                    Local Folder
                  </button>
                </div>
              </div>

              {/* GitHub content */}
              {sourceType === "github" && (
                <div className="flex flex-col gap-3">
                  {/* Org input — first-time setup */}
                  {orgLoaded && !org && (
                    <div>
                      <label className="block text-xs text-nx-muted mb-1.5">
                        GitHub Organization
                      </label>
                      <div className="flex gap-2">
                        <input
                          type="text"
                          value={orgInput}
                          onChange={(e) => setOrgInput(e.target.value)}
                          placeholder="e.g. SkyNexus-AI"
                          className="flex-1 px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                            placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50"
                          onKeyDown={(e) =>
                            e.key === "Enter" && handleOrgSubmit()
                          }
                        />
                        <button
                          onClick={handleOrgSubmit}
                          disabled={!orgInput.trim()}
                          className={`px-3 py-2 text-xs font-medium rounded-full transition-colors ${
                            orgInput.trim()
                              ? "bg-nx-accent text-white hover:bg-nx-accent-hover"
                              : "bg-nx-accent/30 text-white/40 cursor-not-allowed"
                          }`}
                        >
                          Save & Load
                        </button>
                      </div>
                      <p className="text-[10px] text-nx-dim mt-1">
                        Saved to app settings. Change later in Settings.
                      </p>
                    </div>
                  )}

                  {/* Org display (read-only once configured) */}
                  {orgLoaded && org && (
                    <div className="flex items-center justify-between">
                      <span className="text-xs text-nx-muted">
                        Org:{" "}
                        <span className="text-nx-text font-medium">{org}</span>
                      </span>
                      <button
                        onClick={() => {
                          setOrg("");
                          setOrgInput("");
                          setGhAuth(null);
                          setRepos([]);
                          setSelectedRepo(null);
                          setReposError(null);
                        }}
                        className="text-[10px] text-nx-dim hover:text-nx-text transition-colors"
                      >
                        Change
                      </button>
                    </div>
                  )}

                  {/* Auth error */}
                  {ghAuth && !ghAuth.authenticated && (
                    <div className="bg-red-500/10 border border-red-500/20 rounded px-3 py-2">
                      <p className="text-xs text-red-400">{ghAuth.error}</p>
                    </div>
                  )}

                  {/* Auth success + username */}
                  {ghAuth?.authenticated && ghAuth.username && (
                    <p className="text-[10px] text-nx-dim">
                      Authenticated as {ghAuth.username}
                    </p>
                  )}

                  {/* Repos error */}
                  {reposError && (
                    <div className="bg-red-500/10 border border-red-500/20 rounded px-3 py-2">
                      <p className="text-xs text-red-400">{reposError}</p>
                    </div>
                  )}

                  {/* Repo picker */}
                  {ghAuth?.authenticated && !reposError && (
                    <RepoPicker
                      repos={repos}
                      loading={reposLoading}
                      selected={selectedRepo}
                      onSelect={setSelectedRepo}
                      org={org}
                    />
                  )}

                  {/* Branch selector */}
                  {selectedRepo && (
                    <div>
                      <label className="block text-xs text-nx-muted mb-1.5">
                        Branch
                      </label>
                      {branchesLoading ? (
                        <div className="flex items-center gap-2 py-1">
                          <div className="w-3.5 h-3.5 border-2 border-nx-accent border-t-transparent rounded-full animate-spin" />
                          <span className="text-[10px] text-nx-dim">
                            Loading branches...
                          </span>
                        </div>
                      ) : branches.length > 0 ? (
                        <select
                          value={selectedBranch}
                          onChange={(e) => setSelectedBranch(e.target.value)}
                          className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                            focus:outline-none focus:border-nx-accent/50"
                        >
                          {branches.map((b) => (
                            <option key={b.name} value={b.name}>
                              {b.name}
                              {b.name === selectedRepo.default_branch
                                ? " (default)"
                                : ""}
                            </option>
                          ))}
                        </select>
                      ) : (
                        <p className="text-[10px] text-nx-dim">
                          Will clone default branch ({selectedRepo.default_branch})
                        </p>
                      )}
                    </div>
                  )}
                </div>
              )}

              {/* Local folder content */}
              {sourceType === "local" && (
                <div>
                  <label className="block text-xs text-nx-muted mb-1.5">
                    Folder <span className="text-red-400">*</span>
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={folderPath}
                      onChange={(e) => setFolderPath(e.target.value)}
                      placeholder="/path/to/project"
                      className="flex-1 px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                        placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50"
                    />
                    <button
                      onClick={handleBrowse}
                      className="px-3 py-2 text-xs font-medium bg-nx-bg border border-nx-border rounded-lg
                        text-nx-text hover:bg-nx-surface-hover transition-colors"
                    >
                      Browse
                    </button>
                  </div>
                </div>
              )}

              {/* Collapsible Details */}
              <div>
                <button
                  onClick={() => setShowDetails(!showDetails)}
                  className="flex items-center gap-1.5 text-xs text-nx-muted hover:text-nx-text transition-colors"
                >
                  <span className="text-[10px]">{showDetails ? "▾" : "▸"}</span>
                  Details
                  {selectedLane && !showDetails && (
                    <span className="text-nx-dim ml-1">
                      — {selectedLane.name}
                    </span>
                  )}
                </button>
                {showDetails && (
                  <div className="flex flex-col gap-3 mt-2 pl-3 border-l border-nx-border-light">
                    <div>
                      <label className="block text-xs text-nx-muted mb-1.5">
                        Lane
                      </label>
                      <select
                        value={laneId}
                        onChange={(e) => setLaneId(e.target.value)}
                        className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                          focus:outline-none focus:border-nx-accent/50"
                      >
                        {lanes.map((lane) => (
                          <option key={lane.id} value={lane.id}>
                            {lane.name}
                          </option>
                        ))}
                      </select>
                    </div>
                    <div>
                      <label className="block text-xs text-nx-muted mb-1.5">
                        Notes
                      </label>
                      <textarea
                        value={notes}
                        onChange={(e) => setNotes(e.target.value)}
                        placeholder="Optional notes..."
                        rows={2}
                        className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
                          placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50 resize-none"
                      />
                    </div>
                  </div>
                )}
              </div>

              {/* Claude launch toggle */}
              <label className="flex items-center gap-2.5 mt-1 cursor-pointer group">
                <div
                  onClick={() => setLaunchClaude(!launchClaude)}
                  className={`relative w-8 h-[18px] rounded-full transition-colors ${
                    launchClaude ? "bg-orange-500" : "bg-white/10"
                  }`}
                >
                  <div
                    className={`absolute top-[2px] w-[14px] h-[14px] rounded-full bg-white transition-transform ${
                      launchClaude ? "translate-x-[16px]" : "translate-x-[2px]"
                    }`}
                  />
                </div>
                <div>
                  <span className="text-xs text-nx-text group-hover:text-white transition-colors">
                    Start with Claude
                  </span>
                  <p className="text-[10px] text-nx-dim">
                    Launches claude --dangerously-skip-permissions after shell starts
                  </p>
                </div>
              </label>

              {/* Enable Relay toggle */}
              <label className="flex items-center gap-2.5 cursor-pointer group">
                <div
                  onClick={() => setRelayEnabled(!relayEnabled)}
                  className={`relative w-8 h-[18px] rounded-full transition-colors ${
                    relayEnabled ? "bg-nx-accent" : "bg-white/10"
                  }`}
                >
                  <div
                    className={`absolute top-[2px] w-[14px] h-[14px] rounded-full bg-white transition-transform ${
                      relayEnabled ? "translate-x-[16px]" : "translate-x-[2px]"
                    }`}
                  />
                </div>
                <div>
                  <span className="text-xs text-nx-text group-hover:text-white transition-colors">
                    Enable Relay
                  </span>
                  <p className="text-[10px] text-nx-dim">
                    Agent will receive relay messages from other participants
                  </p>
                </div>
              </label>

              {/* Error */}
              {error && (
                <div className="bg-red-500/10 border border-red-500/20 rounded px-3 py-2">
                  <p className="text-xs text-red-400">{error}</p>
                </div>
              )}
            </div>
          ) : (
            /* Creating state — step-by-step progress */
            <div className="flex flex-col gap-3 py-4">
              <h3 className="text-xs font-semibold text-nx-muted uppercase tracking-wider">
                Creating session...
              </h3>

              {sourceType === "github" && (
                <div className="flex items-center gap-3">
                  {stepIcon(createSteps.clone)}
                  <span
                    className={`text-sm ${
                      createSteps.clone === "done" || createSteps.clone === "active"
                        ? "text-nx-text"
                        : "text-nx-dim"
                    }`}
                  >
                    Cloning {selectedRepo?.full_name}
                    {selectedBranch &&
                    selectedBranch !== selectedRepo?.default_branch
                      ? ` (${selectedBranch})`
                      : ""}
                    ...
                  </span>
                </div>
              )}

              <div className="flex items-center gap-3">
                {stepIcon(createSteps.card)}
                <span
                  className={`text-sm ${
                    createSteps.card === "done" || createSteps.card === "active"
                      ? "text-nx-text"
                      : "text-nx-dim"
                  }`}
                >
                  Creating card
                </span>
              </div>

              <div className="flex items-center gap-3">
                {stepIcon(createSteps.ready)}
                <span
                  className={`text-sm ${
                    createSteps.ready === "done" ? "text-nx-text" : "text-nx-dim"
                  }`}
                >
                  Ready
                </span>
              </div>

              {error && (
                <div className="bg-red-500/10 border border-red-500/20 rounded px-3 py-2 mt-2">
                  <p className="text-xs text-red-400">{error}</p>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 py-3 border-t border-nx-border-light">
          {!creating && (
            <>
              <button
                onClick={onClose}
                className="px-3 py-1.5 text-xs text-nx-muted hover:text-nx-text transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleCreate}
                disabled={!canCreate}
                className={`px-4 py-1.5 text-xs font-medium rounded-full transition-colors ${
                  canCreate
                    ? "bg-nx-accent text-white hover:bg-nx-accent-hover"
                    : "bg-nx-accent/30 text-white/40 cursor-not-allowed"
                }`}
              >
                Create
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
