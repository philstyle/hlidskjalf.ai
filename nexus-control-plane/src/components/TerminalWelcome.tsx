import { useEffect } from "react";
import { useNavStore } from "../store/nav";
import { useInboxStore } from "../store/inbox";
import { useSettingsStore } from "../store/settings";
import InboxView from "./inbox/InboxView";
import logoBlack from "../assets/brand/logo-black.png";
import logoWhite from "../assets/brand/logo-white.png";
import heroBg from "../assets/brand/hero-bg.jpg";

export default function TerminalWelcome() {
  const inboxPath = useInboxStore((s) => s.inboxPath);
  const setInboxPath = useInboxStore((s) => s.setInboxPath);
  const items = useInboxStore((s) => s.items);
  const fetchInbox = useInboxStore((s) => s.fetchInbox);
  const inboxPathSetting = useSettingsStore((s) => s.settings.inbox_path);

  // Resolve inbox path: try configured setting first, then fallback candidates
  useEffect(() => {
    if (inboxPath) return;

    const tryPaths = async () => {
      const { invoke } = await import("@tauri-apps/api/core");

      const candidates = [
        inboxPathSetting,
        `/Users/stephen/Git/stephen-work/inbox`,
      ].filter(Boolean);

      for (const path of candidates) {
        try {
          const files: { name: string }[] = await invoke("list_directory", { path });
          if (files.some((f) => f.name.endsWith(".md"))) {
            setInboxPath(path);
            return;
          }
        } catch {
          // Try next path
        }
      }
    };

    tryPaths();
  }, [inboxPath, setInboxPath, inboxPathSetting]);

  // Fetch inbox items once path is resolved
  useEffect(() => {
    if (inboxPath) fetchInbox();
  }, [inboxPath, fetchInbox]);

  // Show inbox if we have items
  if (inboxPath && items.length > 0) {
    return <InboxView />;
  }

  // Default welcome screen
  return (
    <div className="relative flex flex-col items-center justify-center h-full text-center px-8 overflow-hidden">
      {/* Background image — dimmed */}
      <div
        className="absolute inset-0 bg-cover bg-center opacity-[0.15] dark:opacity-[0.25]"
        style={{ backgroundImage: `url(${heroBg})` }}
      />

      {/* Content */}
      <div className="relative z-10 flex flex-col items-center">
        {/* SkyNexus logo — swap black/white for light/dark mode */}
        <img
          src={logoBlack}
          alt="SkyNexus"
          className="w-80 h-80 object-contain -mb-10 opacity-80 dark:hidden"
        />
        <img
          src={logoWhite}
          alt="SkyNexus"
          className="w-80 h-80 object-contain -mb-10 opacity-80 hidden dark:block"
        />

        <h2 className="text-xl font-heading font-semibold text-nx-text mb-2">
          Welcome to Nexus Command Center
        </h2>
        <p className="text-sm font-body text-nx-text-secondary max-w-sm mb-8">
          Select a session from the sidebar to open a terminal, or create a new
          one to get started.
        </p>

        <button
          onClick={() => useNavStore.getState().openNewSessionModal()}
          className="px-6 py-2.5 bg-nx-accent text-white rounded-full font-body font-medium text-sm hover:bg-nx-accent-hover transition-colors shadow-nx"
        >
          + New Session
        </button>

        {/* Keyboard shortcut hints */}
        <div className="flex gap-6 mt-8 text-[11px] font-secondary text-nx-muted">
          <span>
            <kbd className="px-1.5 py-0.5 bg-nx-surface border border-nx-border-light rounded text-[10px] font-mono">
              ⌘N
            </kbd>{" "}
            New session
          </span>
          <span>
            <kbd className="px-1.5 py-0.5 bg-nx-surface border border-nx-border-light rounded text-[10px] font-mono">
              ⌘B
            </kbd>{" "}
            Toggle sidebar
          </span>
          <span>
            <kbd className="px-1.5 py-0.5 bg-nx-surface border border-nx-border-light rounded text-[10px] font-mono">
              ⌘,
            </kbd>{" "}
            Settings
          </span>
        </div>
      </div>
    </div>
  );
}
