import { useState } from "react";
import { useSettingsStore } from "../../store/settings";
import { useNavStore } from "../../store/nav";
import NexusLinkPopover from "../NexusLinkPopover";
import WakeToggle from "../WakeToggle";

export default function SidebarFooter() {
  const userName = useSettingsStore((s) => s.settings.user_name);
  const accent = useSettingsStore((s) => s.settings.ncc_accent_color) || "#3B82F6";
  const setView = useNavStore((s) => s.setView);
  const [showNexusLink, setShowNexusLink] = useState(false);

  return (
    <div
      className="relative flex items-center justify-between px-3 py-2.5 border-t border-nx-border shrink-0"
      style={{ borderBottomColor: accent, borderBottomWidth: 3, borderBottomStyle: "solid" }}
    >
      {/* User name */}
      <span className="text-xs font-body text-nx-text-secondary truncate">
        {userName || "SkyNexus User"}
      </span>

      <div className="flex items-center gap-1">
        {/* Home — deselect card to show inbox/welcome */}
        <button
          onClick={() => useNavStore.getState().deselectCard()}
          title="Home (⌘W)"
          className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="m2.25 12 8.954-8.955a1.126 1.126 0 0 1 1.591 0L21.75 12M4.5 9.75v10.125c0 .621.504 1.125 1.125 1.125H9.75v-4.875c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125V21h4.125c.621 0 1.125-.504 1.125-1.125V9.75M8.25 21h8.25" />
          </svg>
        </button>

        {/* Wake toggle */}
        <WakeToggle />

        {/* NexusLink QR */}
        <button
          onClick={() => setShowNexusLink((s) => !s)}
          title="NexusLink"
          className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 4.875c0-.621.504-1.125 1.125-1.125h4.5c.621 0 1.125.504 1.125 1.125v4.5c0 .621-.504 1.125-1.125 1.125h-4.5A1.125 1.125 0 0 1 3.75 9.375v-4.5ZM3.75 14.625c0-.621.504-1.125 1.125-1.125h4.5c.621 0 1.125.504 1.125 1.125v4.5c0 .621-.504 1.125-1.125 1.125h-4.5a1.125 1.125 0 0 1-1.125-1.125v-4.5ZM13.5 4.875c0-.621.504-1.125 1.125-1.125h4.5c.621 0 1.125.504 1.125 1.125v4.5c0 .621-.504 1.125-1.125 1.125h-4.5a1.125 1.125 0 0 1-1.125-1.125v-4.5Z" />
            <path strokeLinecap="round" strokeLinejoin="round" d="M13.5 14.625v2.625m3.375-2.625V21m3.375-6.375v2.625M13.5 21h3.375m0-3.375h3.375" />
          </svg>
        </button>

        {/* Settings gear */}
        <button
          onClick={() => setView("settings")}
          title="Settings"
          className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z" />
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
          </svg>
        </button>
      </div>

      {/* NexusLink Popover */}
      {showNexusLink && (
        <NexusLinkPopover onClose={() => setShowNexusLink(false)} />
      )}
    </div>
  );
}
