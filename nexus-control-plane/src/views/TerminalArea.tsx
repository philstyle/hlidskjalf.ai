import { useState, useCallback, useEffect, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useNavStore } from "../store/nav";
import { useCardsStore } from "../store/cards";
import { useLanesStore } from "../store/lanes";
import { useFilesStore } from "../store/files";
import TerminalPanel from "./TerminalPanel";
import MissionControl from "../components/MissionControl";
import ContextBar from "../components/ContextBar";

export default function TerminalArea() {
  const activeCardId = useNavStore((s) => s.activeCardId);
  const card = useCardsStore((s) =>
    activeCardId ? s.cards.find((c) => c.id === activeCardId) : undefined,
  );
  const lane = useLanesStore((s) =>
    card ? s.lanes.find((l) => l.id === card.lane_id) : undefined,
  );
  const deselectCard = useNavStore((s) => s.deselectCard);
  const copyFiles = useFilesStore((s) => s.copyFiles);
  const [dropActive, setDropActive] = useState(false);
  const areaRef = useRef<HTMLDivElement>(null);

  // Native Finder drag-drop: copy files into active card's workspace
  useEffect(() => {
    if (!card) return;
    const webview = getCurrentWebview();
    const workspace = card.workspace_path;
    const unlistenPromise = webview.onDragDropEvent((event) => {
      const payload = event.payload;
      if (!areaRef.current) return;

      if (payload.type === "leave") {
        setDropActive(false);
        return;
      }

      const rect = areaRef.current.getBoundingClientRect();
      const pos = payload.position;
      const isOver =
        pos.x >= rect.left &&
        pos.x <= rect.right &&
        pos.y >= rect.top &&
        pos.y <= rect.bottom;

      if (payload.type === "enter" || payload.type === "over") {
        setDropActive(isOver);
      } else if (payload.type === "drop") {
        setDropActive(false);
        if (isOver && payload.paths.length > 0) {
          copyFiles(payload.paths, workspace);
        }
      }
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [card, copyFiles]);

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      if (!card || !e.dataTransfer.types.includes("application/x-ncc-files"))
        return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setDropActive(true);
    },
    [card],
  );

  const handleDragLeave = useCallback(() => setDropActive(false), []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      e.preventDefault();
      setDropActive(false);
      if (!card) return;
      const raw = e.dataTransfer.getData("application/x-ncc-files");
      if (!raw) return;
      try {
        const sources: string[] = JSON.parse(raw);
        await copyFiles(sources, card.workspace_path);
      } catch {
        // silently fail — user will see the file didn't appear
      }
    },
    [card, copyFiles],
  );

  if (!activeCardId) {
    return <MissionControl />;
  }

  return (
    <div
      ref={areaRef}
      className="flex flex-col h-full bg-nx-bg"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* Terminal header */}
      <div className="flex items-center gap-3 px-4 py-2 bg-nx-surface border-b border-nx-border shrink-0">
        <span className="text-sm font-body font-medium text-nx-text truncate">
          {card?.name ?? "Unknown"}
        </span>
        {lane && (
          <span
            className="text-[10px] font-secondary px-1.5 py-0.5 rounded-full font-medium"
            style={{
              backgroundColor: lane.color + "15",
              color: lane.color,
            }}
          >
            {lane.name}
          </span>
        )}
        <div className="flex-1" />
        <button
          onClick={deselectCard}
          className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
          title="Close terminal panel"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Terminal body */}
      <div
        className={`flex-1 min-h-0 rounded-2xl overflow-hidden m-2 mb-0 shadow-nx-md border transition-colors ${
          dropActive
            ? "border-nx-accent bg-nx-accent/5"
            : "border-nx-border-light"
        }`}
      >
        <TerminalPanel key={activeCardId} cardId={activeCardId} />
        {/* Drop overlay */}
        {dropActive && (
          <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
            <div className="px-4 py-2 bg-nx-accent/90 rounded-lg shadow-nx-md">
              <span className="text-sm font-body font-medium text-white">
                Drop to copy into workspace
              </span>
            </div>
          </div>
        )}
      </div>

      {/* Context bar */}
      <ContextBar cardId={activeCardId} />
    </div>
  );
}
