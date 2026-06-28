import { useNavStore } from "../../store/nav";
import { useSettingsStore } from "../../store/settings";
import titleLogoBlack from "../../assets/brand/title-logo-black.png";
import titleLogoWhite from "../../assets/brand/title-logo-white.png";

export default function SidebarHeader() {
  const openModal = useNavStore((s) => s.openNewSessionModal);
  const toggleSidebar = useNavStore((s) => s.toggleSidebar);
  const sidebarMode = useNavStore((s) => s.sidebarMode);
  const setSidebarMode = useNavStore((s) => s.setSidebarMode);
  const fileAccess = useSettingsStore((s) => s.settings.file_access);
  const nccName = useSettingsStore((s) => s.settings.ncc_display_name) || "Nexus Command Center";
  const accent = useSettingsStore((s) => s.settings.ncc_accent_color) || "#3B82F6";

  return (
    <div
      className="flex flex-col border-b border-nx-border shrink-0"
      style={{ borderTopColor: accent, borderTopWidth: 3, borderTopStyle: "solid" }}
    >
      {/* Top row: logo + actions */}
      <div className="flex items-center justify-between px-3 py-2.5">
        <div className="min-w-0">
          <img src={titleLogoBlack} alt="SkyNexus AI" className="h-5 object-contain dark:hidden" />
          <img src={titleLogoWhite} alt="SkyNexus AI" className="h-5 object-contain hidden dark:block" />
          <div
            className="mt-1 truncate text-[11px] font-body font-medium leading-none text-nx-text-secondary"
            title={nccName}
          >
            {nccName}
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          {sidebarMode === "sessions" && (
            <button
              onClick={openModal}
              className="px-2.5 py-0.5 text-[11px] font-body font-medium bg-nx-accent text-white rounded-full hover:bg-nx-accent-hover transition-colors"
            >
              + New
            </button>
          )}
          <button
            onClick={toggleSidebar}
            title="Collapse sidebar (Cmd+B)"
            className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
          >
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="m15 19-7-7 7-7" />
            </svg>
          </button>
        </div>
      </div>

      {/* Mode toggle — only show if file access enabled */}
      {fileAccess === "on" && (
        <div className="flex mx-3 mb-2 bg-nx-bg rounded-lg p-0.5">
          <button
            onClick={() => setSidebarMode("sessions")}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1 rounded-md text-[10px] font-secondary font-medium transition-colors ${
              sidebarMode === "sessions"
                ? "bg-nx-surface text-nx-text shadow-sm"
                : "text-nx-muted hover:text-nx-text"
            }`}
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6.75 7.5l3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0021 18V6a2.25 2.25 0 00-2.25-2.25H5.25A2.25 2.25 0 003 6v12a2.25 2.25 0 002.25 2.25z" />
            </svg>
            Sessions
          </button>
          <button
            onClick={() => setSidebarMode("files")}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1 rounded-md text-[10px] font-secondary font-medium transition-colors ${
              sidebarMode === "files"
                ? "bg-nx-surface text-nx-text shadow-sm"
                : "text-nx-muted hover:text-nx-text"
            }`}
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z" />
            </svg>
            Files
          </button>
        </div>
      )}
    </div>
  );
}
