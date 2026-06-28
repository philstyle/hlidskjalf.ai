import { useEffect, useState, useCallback, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useFilesStore, type FileEntry } from "../../store/files";
import { usePreviewStore } from "../../store/preview";
import { splitPath, reconstructPath, isRoot } from "../../utils/path";
import CopyToWorkspaceModal from "../CopyToWorkspaceModal";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function FileBrowser() {
  const {
    currentPath,
    entries,
    loading,
    showHidden,
    navigateTo,
    goHome,
    goUp,
    toggleHidden,
    copyFiles,
  } = useFilesStore();

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [showCopyModal, setShowCopyModal] = useState(false);
  const [nativeDropActive, setNativeDropActive] = useState(false);
  const browserRef = useRef<HTMLDivElement>(null);

  // Initialize at home directory
  useEffect(() => {
    if (!currentPath) goHome();
  }, [currentPath, goHome]);

  // Clear selection on path change
  useEffect(() => {
    setSelected(new Set());
  }, [currentPath]);

  // Native file manager drag-and-drop: listen for Tauri drag-drop events
  // and copy dropped files into the currently viewed directory
  useEffect(() => {
    const webview = getCurrentWebview();
    const unlistenPromise = webview.onDragDropEvent((event) => {
      const { type } = event.payload;
      if (type === "enter" || type === "over") {
        // Check if the drop position is over the file browser area
        if (browserRef.current) {
          const rect = browserRef.current.getBoundingClientRect();
          const pos = event.payload.position;
          const isOver =
            pos.x >= rect.left &&
            pos.x <= rect.right &&
            pos.y >= rect.top &&
            pos.y <= rect.bottom;
          setNativeDropActive(isOver);
        }
      } else if (type === "drop") {
        setNativeDropActive(false);
        if (!currentPath || !browserRef.current) return;
        // Check position is over the browser
        const rect = browserRef.current.getBoundingClientRect();
        const pos = event.payload.position;
        const isOver =
          pos.x >= rect.left &&
          pos.x <= rect.right &&
          pos.y >= rect.top &&
          pos.y <= rect.bottom;
        if (isOver && event.payload.paths.length > 0) {
          copyFiles(event.payload.paths, currentPath).then(() => {
            navigateTo(currentPath); // refresh
          });
        }
      } else if (type === "leave") {
        setNativeDropActive(false);
      }
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [currentPath, copyFiles, navigateTo]);

  const visibleEntries = showHidden
    ? entries
    : entries.filter((e) => !e.hidden);

  const toggleSelect = useCallback(
    (path: string, e: React.MouseEvent) => {
      if (e.metaKey || e.ctrlKey) {
        // Multi-select with Cmd/Ctrl
        setSelected((prev) => {
          const next = new Set(prev);
          if (next.has(path)) next.delete(path);
          else next.add(path);
          return next;
        });
      } else if (e.shiftKey && selected.size > 0) {
        // Range select with Shift
        const paths = visibleEntries.map((e) => e.path);
        const lastSelected = [...selected].pop()!;
        const lastIdx = paths.indexOf(lastSelected);
        const curIdx = paths.indexOf(path);
        if (lastIdx >= 0 && curIdx >= 0) {
          const [start, end] = lastIdx < curIdx ? [lastIdx, curIdx] : [curIdx, lastIdx];
          const range = paths.slice(start, end + 1);
          setSelected(new Set([...selected, ...range]));
        }
      } else {
        setSelected(new Set([path]));
      }
    },
    [selected, visibleEntries],
  );

  const handleDoubleClick = useCallback(
    (entry: FileEntry) => {
      if (entry.is_dir) {
        navigateTo(entry.path);
      } else if (entry.name.endsWith(".md")) {
        usePreviewStore.getState().openFile(entry.path);
      }
    },
    [navigateTo],
  );

  const handleDragStart = useCallback(
    (e: React.DragEvent, entry: FileEntry) => {
      // If dragging a selected file, drag all selected; otherwise just this one
      const files = selected.has(entry.path)
        ? [...selected]
        : [entry.path];
      e.dataTransfer.setData("application/x-ncc-files", JSON.stringify(files));
      e.dataTransfer.effectAllowed = "copy";
    },
    [selected],
  );

  // Breadcrumb segments
  const breadcrumbs = currentPath
    ? splitPath(currentPath)
    : [];

  return (
    <div
      ref={browserRef}
      className={`flex flex-col h-full transition-colors ${
        nativeDropActive ? "bg-nx-accent/5 ring-2 ring-inset ring-nx-accent/30" : ""
      }`}
    >
      {/* Native drop indicator */}
      {nativeDropActive && (
        <div className="flex items-center justify-center py-1.5 bg-nx-accent/10 border-b border-nx-accent/20 shrink-0">
          <span className="text-[10px] font-secondary font-medium text-nx-accent">
            Drop files here to copy into current folder
          </span>
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-nx-border-light shrink-0">
        <button
          onClick={goUp}
          disabled={!currentPath || isRoot(currentPath)}
          title="Go up"
          className="p-1 rounded text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors disabled:opacity-30"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 15.75 7.5-7.5 7.5 7.5" />
          </svg>
        </button>
        <button
          onClick={goHome}
          title="Home"
          className="p-1 rounded text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="m2.25 12 8.954-8.955a1.126 1.126 0 0 1 1.591 0L21.75 12M4.5 9.75v10.125c0 .621.504 1.125 1.125 1.125H9.75v-4.875c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125V21h4.125c.621 0 1.125-.504 1.125-1.125V9.75M8.25 21h8.25" />
          </svg>
        </button>
        <div className="flex-1" />
        <button
          onClick={toggleHidden}
          title={showHidden ? "Hide dotfiles" : "Show dotfiles"}
          className={`p-1 rounded transition-colors ${
            showHidden
              ? "text-nx-accent bg-nx-accent-light"
              : "text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover"
          }`}
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M2.036 12.322a1.012 1.012 0 0 1 0-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178Z" />
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
          </svg>
        </button>
      </div>

      {/* Breadcrumb */}
      <div className="flex items-center gap-0.5 px-2 py-1 border-b border-nx-border-light overflow-x-auto shrink-0 scrollbar-none">
        <button
          onClick={() => goHome()}
          className="text-[10px] text-nx-muted hover:text-nx-text shrink-0"
        >
          /
        </button>
        {breadcrumbs.map((segment, i) => {
          const fullPath = reconstructPath(breadcrumbs, i);
          const isLast = i === breadcrumbs.length - 1;
          return (
            <span key={fullPath} className="flex items-center gap-0.5 shrink-0">
              <span className="text-[10px] text-nx-dim">/</span>
              <button
                onClick={() => navigateTo(fullPath)}
                className={`text-[10px] truncate max-w-[80px] ${
                  isLast ? "text-nx-text font-medium" : "text-nx-muted hover:text-nx-text"
                }`}
              >
                {segment}
              </button>
            </span>
          );
        })}
      </div>

      {/* File list */}
      <div className="flex-1 overflow-y-auto py-1 px-1">
        {loading ? (
          <div className="flex items-center justify-center py-8">
            <span className="text-[11px] text-nx-muted">Loading...</span>
          </div>
        ) : visibleEntries.length === 0 ? (
          <div className="flex items-center justify-center py-8">
            <span className="text-[11px] text-nx-muted">Empty folder</span>
          </div>
        ) : (
          visibleEntries.map((entry) => (
            <FileRow
              key={entry.path}
              entry={entry}
              isSelected={selected.has(entry.path)}
              onClick={(e) => toggleSelect(entry.path, e)}
              onDoubleClick={() => handleDoubleClick(entry)}
              onDragStart={(e) => handleDragStart(e, entry)}
            />
          ))
        )}
      </div>

      {/* Action bar — shown when files selected */}
      {selected.size > 0 && (
        <div className="flex items-center gap-2 px-2 py-1.5 border-t border-nx-border-light shrink-0 bg-nx-surface">
          <span className="text-[10px] text-nx-muted flex-1">
            {selected.size} selected
          </span>
          <button
            onClick={() => setShowCopyModal(true)}
            className="px-2 py-0.5 text-[10px] font-medium bg-nx-accent text-white rounded-full hover:bg-nx-accent-hover transition-colors"
          >
            Copy to workspace
          </button>
        </div>
      )}

      {/* Copy modal */}
      {showCopyModal && (
        <CopyToWorkspaceModal
          sources={[...selected]}
          onClose={() => setShowCopyModal(false)}
          onCopied={() => {
            setSelected(new Set());
            setShowCopyModal(false);
          }}
        />
      )}
    </div>
  );
}

function FileRow({
  entry,
  isSelected,
  onClick,
  onDoubleClick,
  onDragStart,
}: {
  entry: FileEntry;
  isSelected: boolean;
  onClick: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  return (
    <div
      draggable
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onDragStart={onDragStart}
      className={`flex items-center gap-2 px-2 py-1 rounded cursor-pointer select-none transition-colors
        ${isSelected
          ? "bg-nx-accent-light border border-nx-accent/20"
          : "hover:bg-nx-surface-hover border border-transparent"
        }`}
    >
      {/* Icon */}
      {entry.is_dir ? (
        <svg className="w-3.5 h-3.5 text-nx-accent shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 0 1 4.5 9.75h15A2.25 2.25 0 0 1 21.75 12v.75m-8.69-6.44-2.12-2.12a1.5 1.5 0 0 0-1.061-.44H4.5A2.25 2.25 0 0 0 2.25 6v12a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9a2.25 2.25 0 0 0-2.25-2.25h-5.379a1.5 1.5 0 0 1-1.06-.44Z" />
        </svg>
      ) : (
        <svg className="w-3.5 h-3.5 text-nx-muted shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" />
        </svg>
      )}

      {/* Name */}
      <span className="flex-1 text-xs font-body text-nx-text truncate">
        {entry.name}
      </span>

      {/* Size (files only) */}
      {!entry.is_dir && (
        <span className="text-[10px] text-nx-dim shrink-0">
          {formatSize(entry.size)}
        </span>
      )}
    </div>
  );
}
