import { useState, useMemo } from "react";
import { useCardsStore } from "../store/cards";
import { useFilesStore } from "../store/files";
import { useNavStore } from "../store/nav";
import { basename, joinPath } from "../utils/path";

interface CopyToWorkspaceModalProps {
  sources: string[];
  onClose: () => void;
  onCopied: () => void;
}

export default function CopyToWorkspaceModal({
  sources,
  onClose,
  onCopied,
}: CopyToWorkspaceModalProps) {
  const cards = useCardsStore((s) => s.cards);
  const activeCardId = useNavStore((s) => s.activeCardId);
  const copyFiles = useFilesStore((s) => s.copyFiles);

  const [search, setSearch] = useState("");
  const [selectedCardId, setSelectedCardId] = useState<string | null>(activeCardId);
  const [subfolder, setSubfolder] = useState("");
  const [copying, setCopying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (!search.trim()) return cards;
    const q = search.toLowerCase();
    return cards.filter(
      (c) =>
        c.name.toLowerCase().includes(q) ||
        c.workspace_path.toLowerCase().includes(q),
    );
  }, [cards, search]);

  const selectedCard = cards.find((c) => c.id === selectedCardId);

  const destination = selectedCard
    ? subfolder.trim()
      ? joinPath(selectedCard.workspace_path, subfolder.trim())
      : selectedCard.workspace_path
    : "";

  const handleCopy = async () => {
    if (!destination || sources.length === 0) return;
    setCopying(true);
    setError(null);
    try {
      await copyFiles(sources, destination);
      onCopied();
    } catch (e) {
      setError(String(e));
    } finally {
      setCopying(false);
    }
  };

  const fileNames = sources.map((s) => basename(s));

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="bg-nx-surface border border-nx-border rounded-xl shadow-nx-lg w-[420px] max-h-[500px] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="px-4 py-3 border-b border-nx-border-light shrink-0">
          <h3 className="text-sm font-body font-semibold text-nx-text">
            Copy to Workspace
          </h3>
          <p className="text-[10px] text-nx-muted mt-0.5">
            {sources.length} file{sources.length !== 1 ? "s" : ""}: {fileNames.join(", ")}
          </p>
        </div>

        {/* Search */}
        <div className="px-4 py-2 border-b border-nx-border-light shrink-0">
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search sessions..."
            autoFocus
            className="w-full px-3 py-1.5 text-xs bg-nx-bg border border-nx-border rounded text-nx-text placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50"
          />
        </div>

        {/* Card list */}
        <div className="flex-1 overflow-y-auto py-1 px-2">
          {filtered.map((card) => (
            <button
              key={card.id}
              onClick={() => setSelectedCardId(card.id)}
              className={`w-full flex items-center gap-2 px-2 py-1.5 rounded text-left transition-colors
                ${selectedCardId === card.id
                  ? "bg-nx-accent-light border border-nx-accent/20"
                  : "hover:bg-nx-surface-hover border border-transparent"
                }`}
            >
              <svg className="w-3.5 h-3.5 text-nx-accent shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M2.25 12.75V12A2.25 2.25 0 0 1 4.5 9.75h15A2.25 2.25 0 0 1 21.75 12v.75m-8.69-6.44-2.12-2.12a1.5 1.5 0 0 0-1.061-.44H4.5A2.25 2.25 0 0 0 2.25 6v12a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9a2.25 2.25 0 0 0-2.25-2.25h-5.379a1.5 1.5 0 0 1-1.06-.44Z" />
              </svg>
              <div className="flex-1 min-w-0">
                <span className="text-xs font-body text-nx-text block truncate">
                  {card.name}
                </span>
                <span className="text-[10px] text-nx-dim block truncate">
                  {card.workspace_path}
                </span>
              </div>
            </button>
          ))}
          {filtered.length === 0 && (
            <p className="text-[11px] text-nx-muted text-center py-4">
              No matching sessions
            </p>
          )}
        </div>

        {/* Subfolder input */}
        {selectedCard && (
          <div className="px-4 py-2 border-t border-nx-border-light shrink-0">
            <label className="block text-[10px] text-nx-muted mb-1">
              Subfolder (optional)
            </label>
            <input
              type="text"
              value={subfolder}
              onChange={(e) => setSubfolder(e.target.value)}
              placeholder="e.g. docs, src/data"
              className="w-full px-3 py-1.5 text-xs bg-nx-bg border border-nx-border rounded text-nx-text placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50"
            />
            <p className="text-[10px] text-nx-dim mt-1 truncate">
              {destination}
            </p>
          </div>
        )}

        {/* Error */}
        {error && (
          <p className="px-4 py-1 text-[10px] text-red-400 shrink-0">{error}</p>
        )}

        {/* Actions */}
        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-nx-border-light shrink-0">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-xs font-body text-nx-muted hover:text-nx-text transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleCopy}
            disabled={!selectedCardId || copying}
            className="px-3 py-1.5 text-xs font-body font-medium bg-nx-accent text-white rounded-lg hover:bg-nx-accent-hover transition-colors disabled:opacity-40"
          >
            {copying ? "Copying..." : `Copy ${sources.length} file${sources.length !== 1 ? "s" : ""}`}
          </button>
        </div>
      </div>
    </div>
  );
}
