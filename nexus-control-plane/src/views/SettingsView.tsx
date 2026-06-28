import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useNavStore } from "../store/nav";
import { useSettingsStore } from "../store/settings";
import { useLanesStore } from "../store/lanes";
import LaneEditor from "../components/LaneEditor";
import LaneIcon from "../components/icons/LaneIcon";

interface GhAuthStatus {
  authenticated: boolean;
  username: string | null;
  error: string | null;
}

export default function SettingsView() {
  const backToBoard = useNavStore((s) => s.backToBoard);
  const { settings, updateSetting } = useSettingsStore();
  const { lanes } = useLanesStore();

  // Local input state for auto-save-on-blur fields
  const [userName, setUserName] = useState(settings.user_name);
  const [githubOrg, setGithubOrg] = useState(settings.github_org);
  const [workspaceRoot, setWorkspaceRoot] = useState(settings.workspace_root);
  const [apiKey, setApiKey] = useState(settings.anthropic_api_key);
  const [showKey, setShowKey] = useState(false);
  const [defaultShell, setDefaultShell] = useState(settings.default_shell);
  const [inboxPath, setInboxPath] = useState(settings.inbox_path);

  // Default lane for new cards
  const [defaultLaneId, setDefaultLaneId] = useState("");

  // GitHub auth status
  const [ghAuth, setGhAuth] = useState<GhAuthStatus | null>(null);

  useEffect(() => {
    invoke<GhAuthStatus>("check_gh_auth").then(setGhAuth).catch(() => {
      setGhAuth({ authenticated: false, username: null, error: "Failed to check auth" });
    });
  }, []);

  // Load default lane setting
  useEffect(() => {
    invoke<string | null>("get_setting", { key: "default_lane_id" }).then(
      (id) => {
        if (id && lanes.some((l) => l.id === id)) {
          setDefaultLaneId(id);
        } else if (lanes.length > 0) {
          setDefaultLaneId(lanes[0].id);
        }
      }
    );
  }, [lanes]);

  // Sync local state when settings load
  useEffect(() => {
    setUserName(settings.user_name);
    setGithubOrg(settings.github_org);
    setWorkspaceRoot(settings.workspace_root);
    setApiKey(settings.anthropic_api_key);
    setDefaultShell(settings.default_shell);
    setInboxPath(settings.inbox_path);
  }, [settings.user_name, settings.github_org, settings.workspace_root, settings.anthropic_api_key, settings.default_shell, settings.inbox_path]);

  const handleBrowse = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) {
      setWorkspaceRoot(selected);
      updateSetting("workspace_root", selected);
    }
  };

  const inputClass =
    "w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded text-nx-text placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50";
  const labelClass = "block text-xs text-nx-muted mb-1.5";
  const sectionClass = "mb-8";
  const sectionTitle =
    "text-[10px] font-semibold text-nx-dim uppercase tracking-widest mb-3";

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-3 bg-nx-surface border-b border-nx-border shrink-0">
        <button
          onClick={backToBoard}
          className="text-xs text-nx-muted hover:text-nx-text transition-colors"
        >
          &larr; Back
        </button>
        <div className="w-px h-4 bg-nx-border" />
        <span className="text-sm font-semibold text-nx-text">Settings</span>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-5 py-6 max-w-xl mx-auto w-full">
        {/* IDENTITY */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Identity</h3>
          <div>
            <label className={labelClass}>Name</label>
            <input
              type="text"
              value={userName}
              onChange={(e) => setUserName(e.target.value)}
              onBlur={() => updateSetting("user_name", userName)}
              placeholder="Your name"
              className={inputClass}
            />
          </div>
        </div>

        {/* THEME */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Theme</h3>
          <div className="flex gap-1 rounded-lg border border-nx-border p-1 bg-nx-bg">
            {(["light", "dark", "system"] as const).map((mode) => (
              <button
                key={mode}
                onClick={() => updateSetting("theme_mode", mode)}
                className={`flex-1 px-3 py-1.5 text-xs font-medium rounded-md transition-colors capitalize ${
                  settings.theme_mode === mode
                    ? "bg-nx-accent text-white"
                    : "text-nx-muted hover:text-nx-text"
                }`}
              >
                {mode}
              </button>
            ))}
          </div>
          <p className="text-[10px] text-nx-dim mt-1.5">
            System follows your OS preference.
          </p>
        </div>

        {/* GITHUB */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>GitHub</h3>
          <div className="mb-3">
            <label className={labelClass}>Organization</label>
            <input
              type="text"
              value={githubOrg}
              onChange={(e) => setGithubOrg(e.target.value)}
              onBlur={() => updateSetting("github_org", githubOrg)}
              placeholder="e.g. SkyNexus-AI"
              className={inputClass}
            />
          </div>
          <div>
            <label className={labelClass}>Auth Status</label>
            {ghAuth === null ? (
              <p className="text-xs text-nx-dim">Checking...</p>
            ) : ghAuth.authenticated ? (
              <p className="text-xs text-green-400">
                Authenticated as {ghAuth.username}
              </p>
            ) : (
              <p className="text-xs text-red-400">
                {ghAuth.error ?? "Not authenticated"}
              </p>
            )}
          </div>
        </div>

        {/* WORKSPACE */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Workspace</h3>
          <div>
            <label className={labelClass}>Root Directory</label>
            <div className="flex gap-2">
              <input
                type="text"
                value={workspaceRoot}
                onChange={(e) => setWorkspaceRoot(e.target.value)}
                onBlur={() => updateSetting("workspace_root", workspaceRoot)}
                className={`${inputClass} flex-1`}
              />
              <button
                onClick={handleBrowse}
                className="px-3 py-2 text-xs font-medium bg-nx-bg border border-nx-border rounded-lg text-nx-text hover:bg-nx-surface-hover transition-colors shrink-0"
              >
                Browse
              </button>
            </div>
            <p className="text-[10px] text-nx-dim mt-1.5">
              Changing this does not move existing sessions. New sessions will
              use the updated path.
            </p>
          </div>
        </div>

        {/* INBOX */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Inbox</h3>
          <div>
            <label className={labelClass}>Inbox Path</label>
            <input
              type="text"
              value={inboxPath}
              onChange={(e) => setInboxPath(e.target.value)}
              onBlur={() => updateSetting("inbox_path", inboxPath)}
              placeholder="/path/to/stephen-work/inbox"
              className={inputClass}
            />
            <p className="text-[10px] text-nx-dim mt-1.5">
              Absolute path to your work tracker inbox directory containing markdown files.
            </p>
          </div>
        </div>

        {/* FILE ACCESS */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>File Access</h3>
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs text-nx-text">File Browser</p>
              <p className="text-[10px] text-nx-dim mt-0.5">
                Browse and copy files from your filesystem into workspaces.
              </p>
            </div>
            <button
              onClick={() =>
                updateSetting(
                  "file_access",
                  settings.file_access === "on" ? "off" : "on"
                )
              }
              className={`relative w-9 h-5 rounded-full transition-colors ${
                settings.file_access === "on"
                  ? "bg-nx-accent"
                  : "bg-nx-border"
              }`}
            >
              <span
                className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                  settings.file_access === "on"
                    ? "translate-x-4"
                    : "translate-x-0"
                }`}
              />
            </button>
          </div>
        </div>

        {/* AI SUMMARIES */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>AI Summaries</h3>
          <div className="flex items-center justify-between mb-3">
            <div>
              <p className="text-xs text-nx-text">Session Summaries</p>
              <p className="text-[10px] text-nx-dim mt-0.5">
                Show AI-generated summaries of what each session is doing.
              </p>
            </div>
            <button
              onClick={() =>
                updateSetting(
                  "ai_summaries",
                  settings.ai_summaries === "on" ? "off" : "on"
                )
              }
              className={`relative w-9 h-5 rounded-full transition-colors ${
                settings.ai_summaries === "on"
                  ? "bg-nx-accent"
                  : "bg-nx-border"
              }`}
            >
              <span
                className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                  settings.ai_summaries === "on"
                    ? "translate-x-4"
                    : "translate-x-0"
                }`}
              />
            </button>
          </div>
          <div>
            <label className={labelClass}>API Key</label>
            <div className="flex gap-2">
              <input
                type={showKey ? "text" : "password"}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                onBlur={() => updateSetting("anthropic_api_key", apiKey)}
                placeholder="sk-ant-..."
                className={`${inputClass} flex-1`}
              />
              <button
                onClick={() => setShowKey(!showKey)}
                className="px-3 py-2 text-xs font-medium bg-nx-bg border border-nx-border rounded-lg text-nx-text hover:bg-nx-surface-hover transition-colors shrink-0"
              >
                {showKey ? "Hide" : "Show"}
              </button>
            </div>
            <p className="text-[10px] text-nx-dim mt-1.5">
              Your key stays local. Uses Claude Haiku for low-cost summaries.
            </p>
          </div>
        </div>

        {/* LANES */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Lanes</h3>
          <LaneEditor />
          <div className="mt-4">
            <label className={labelClass}>Default lane for new cards</label>
            <div className="flex items-center gap-2">
              {defaultLaneId && lanes.find((l) => l.id === defaultLaneId) && (
                <LaneIcon
                  name={lanes.find((l) => l.id === defaultLaneId)!.name}
                  className="w-4 h-4 shrink-0"
                />
              )}
              <select
                value={defaultLaneId}
                onChange={(e) => {
                  setDefaultLaneId(e.target.value);
                  invoke("set_setting", {
                    key: "default_lane_id",
                    value: e.target.value,
                  });
                }}
                className={inputClass}
              >
                {lanes.map((lane) => (
                  <option key={lane.id} value={lane.id}>
                    {lane.name}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </div>

        {/* TERMINAL */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Terminal</h3>
          <div className="mb-3">
            <label className={labelClass}>Default Shell</label>
            <input
              type="text"
              value={defaultShell}
              onChange={(e) => setDefaultShell(e.target.value)}
              onBlur={() => updateSetting("default_shell", defaultShell)}
              placeholder={
                navigator.platform.startsWith("Win")
                  ? "powershell.exe"
                  : navigator.platform === "MacIntel"
                    ? "/bin/zsh"
                    : "/bin/bash"
              }
              className={inputClass}
            />
            <p className="text-[10px] text-nx-dim mt-1.5">
              Leave blank for platform default. Examples: powershell.exe, cmd.exe, bash, /bin/zsh
            </p>
          </div>
          <div className="flex gap-4">
            <div className="flex-1">
              <label className={labelClass}>Font Size</label>
              <input
                type="number"
                min={10}
                max={24}
                value={settings.terminal_font_size}
                onChange={(e) => {
                  const val = Math.min(24, Math.max(10, Number(e.target.value)));
                  updateSetting("terminal_font_size", String(val));
                }}
                className={inputClass}
              />
            </div>
            <div className="flex-1">
              <label className={labelClass}>Scrollback</label>
              <select
                value={settings.terminal_scrollback}
                onChange={(e) =>
                  updateSetting("terminal_scrollback", e.target.value)
                }
                className={inputClass}
              >
                <option value={500}>500 lines</option>
                <option value={1000}>1,000 lines</option>
                <option value={5000}>5,000 lines</option>
              </select>
            </div>
          </div>
          <p className="text-[10px] text-nx-dim mt-1.5">
            Changes apply to new terminal sessions.
          </p>
        </div>

        {/* LAYOUT */}
        <div className={sectionClass}>
          <h3 className={sectionTitle}>Layout</h3>
          <p className="text-xs text-nx-text">Mode: Split</p>
          <p className="text-[10px] text-nx-dim mt-1">
            Additional layout modes in a future update.
          </p>
        </div>
      </div>
    </div>
  );
}
