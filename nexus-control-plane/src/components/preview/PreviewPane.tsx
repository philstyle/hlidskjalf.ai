import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { usePreviewStore } from "../../store/preview";
import { basename } from "../../utils/path";
import MarkdownRenderer from "./MarkdownRenderer";

export default function PreviewPane({ onClose }: { onClose: () => void }) {
  const filePath = usePreviewStore((s) => s.filePath);
  const content = usePreviewStore((s) => s.content);
  const setContent = usePreviewStore((s) => s.setContent);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fileName = filePath ? basename(filePath) : "Preview";

  // Load file content and poll for changes every 2s
  useEffect(() => {
    if (!filePath) return;

    const loadFile = async () => {
      try {
        const text = await invoke<string>("read_file", { path: filePath });
        setContent(text);
      } catch (e) {
        console.error("[preview] Failed to read file:", filePath, e);
        setContent(`*Failed to read file: ${filePath}*\n\n\`${e}\``);
      }
    };

    loadFile();
    intervalRef.current = setInterval(loadFile, 2000);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [filePath, setContent]);

  return (
    <div className="flex flex-col h-full bg-nx-surface border-l border-nx-border">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-nx-border-light shrink-0">
        <span className="text-xs font-secondary font-medium text-nx-text-secondary truncate flex-1">
          {fileName}
        </span>
        <button
          onClick={onClose}
          className="p-1 rounded-lg text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover transition-colors"
        >
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Content */}
      {filePath ? (
        <div className="flex-1 overflow-y-auto px-4 py-4">
          <MarkdownRenderer content={content} />
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center p-4">
          <span className="text-xs font-secondary text-nx-muted text-center">
            No file selected for preview
          </span>
        </div>
      )}
    </div>
  );
}
