import { useState, useCallback } from "react";
import { useNavStore } from "../store/nav";
import { useSettingsStore } from "../store/settings";
import { usePreviewStore } from "../store/preview";
import ResizeDivider from "./ResizeDivider";
import NewSessionModal from "./NewSessionModal";
import TerminalArea from "../views/TerminalArea";
import Sidebar from "./sidebar/Sidebar";
import PreviewPane from "./preview/PreviewPane";

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 400;
const PREVIEW_MIN = 280;
const PREVIEW_MAX = 500;

export default function AppShell() {
  const sidebarCollapsed = useNavStore((s) => s.sidebarCollapsed);
  const showNewSessionModal = useNavStore((s) => s.showNewSessionModal);
  const closeNewSessionModal = useNavStore((s) => s.closeNewSessionModal);
  const previewOpen = usePreviewStore((s) => s.open);
  const closePreview = usePreviewStore((s) => s.close);
  const defaultSidebarWidth = useSettingsStore(
    (s) => s.settings.sidebar_width,
  );
  const defaultPreviewWidth = useSettingsStore(
    (s) => s.settings.preview_width,
  );
  const accent = useSettingsStore((s) => s.settings.ncc_accent_color) || "#3B82F6";

  const [sidebarWidth, setSidebarWidth] = useState(defaultSidebarWidth);
  const [previewWidth, setPreviewWidth] = useState(defaultPreviewWidth);

  // Keyboard shortcuts are handled in App.tsx — no duplicates here

  const handleSidebarResize = useCallback(
    (delta: number) => {
      setSidebarWidth((w) =>
        Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, w + delta)),
      );
    },
    [],
  );

  const handlePreviewResize = useCallback(
    (delta: number) => {
      // Dragging the divider left makes preview wider (negative delta)
      setPreviewWidth((w) =>
        Math.min(PREVIEW_MAX, Math.max(PREVIEW_MIN, w - delta)),
      );
    },
    [],
  );

  return (
    <div className="flex h-full w-full">
      {/* Sidebar */}
      {sidebarCollapsed ? (
        <div
          className="h-full shrink-0 w-11 flex flex-col items-center bg-nx-surface border-r border-nx-border"
          style={{ borderTopColor: accent, borderTopWidth: 3, borderTopStyle: "solid", borderBottomColor: accent, borderBottomWidth: 3, borderBottomStyle: "solid" }}
        >
          <button
            onClick={useNavStore.getState().toggleSidebar}
            title="Expand sidebar (Cmd+B)"
            className="mt-2.5 p-1.5 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5" />
            </svg>
          </button>
          <button
            onClick={useNavStore.getState().openNewSessionModal}
            title="New session (Cmd+N)"
            className="mt-1 p-1.5 rounded-lg text-nx-muted hover:text-nx-accent hover:bg-nx-accent-light transition-colors"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
            </svg>
          </button>
        </div>
      ) : (
        <>
          <div
            className="h-full shrink-0 overflow-hidden"
            style={{ width: sidebarWidth }}
          >
            <Sidebar />
          </div>
          <ResizeDivider onResize={handleSidebarResize} direction="horizontal" />
        </>
      )}

      {/* Terminal (hero — takes remaining space) */}
      <div className="flex-1 min-w-0 h-full">
        <TerminalArea />
      </div>

      {/* Preview pane */}
      {previewOpen && (
        <>
          <ResizeDivider
            onResize={handlePreviewResize}
            direction="horizontal"
          />
          <div
            className="h-full shrink-0 overflow-hidden"
            style={{ width: previewWidth }}
          >
            <PreviewPane onClose={closePreview} />
          </div>
        </>
      )}

      {/* New Session Modal */}
      {showNewSessionModal && (
        <NewSessionModal onClose={closeNewSessionModal} />
      )}
    </div>
  );
}
