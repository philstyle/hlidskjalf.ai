import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useCardsStore } from "../store/cards";

interface EditCardModalProps {
  cardId: string;
  onClose: () => void;
}

export default function EditCardModal({ cardId, onClose }: EditCardModalProps) {
  const { cards, updateCard } = useCardsStore();
  const card = cards.find((c) => c.id === cardId);

  const [name, setName] = useState(card?.name ?? "");
  const [notes, setNotes] = useState(card?.notes ?? "");
  const [relayEnabled, setRelayEnabled] = useState(card?.relay_enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!card) return null;

  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);
    setError(null);
    try {
      await updateCard({
        id: card.id,
        name: name.trim(),
        notes: notes.trim() || null,
      });
      if (relayEnabled !== card.relay_enabled) {
        await invoke("set_relay_enabled", { cardId: card.id, enabled: relayEnabled });
        useCardsStore.getState().fetchCards();
      }
      onClose();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const canSave = name.trim() && !saving;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={handleBackdropClick}
    >
      <div className="w-[400px] bg-nx-surface rounded-2xl border border-nx-border shadow-nx-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-nx-border-light">
          <h2 className="text-sm font-semibold text-nx-text">Edit Card</h2>
          <button
            onClick={onClose}
            className="text-nx-muted hover:text-nx-text transition-colors text-lg leading-none"
          >
            &times;
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 flex flex-col gap-4">
          <div>
            <label className="block text-xs text-nx-muted mb-1.5">
              Name <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
              className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded-lg text-nx-text
                focus:outline-none focus:border-nx-accent/50"
            />
          </div>

          <div>
            <label className="block text-xs text-nx-muted mb-1.5">Notes</label>
            <textarea
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              rows={3}
              className="w-full px-3 py-2 text-sm bg-nx-bg border border-nx-border rounded-lg text-nx-text
                focus:outline-none focus:border-nx-accent/50 resize-none"
            />
          </div>

          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={relayEnabled}
              onChange={(e) => setRelayEnabled(e.target.checked)}
              className="w-3.5 h-3.5 rounded border-nx-border accent-nx-accent"
            />
            <span className="text-xs text-nx-text">Enable Relay</span>
          </label>

          {error && <p className="text-xs text-red-400">{error}</p>}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 py-3 border-t border-nx-border-light">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-xs text-nx-muted hover:text-nx-text transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!canSave}
            className={`px-4 py-1.5 text-xs font-medium rounded-full transition-colors
              ${canSave
                ? "bg-nx-accent text-white hover:bg-nx-accent/80"
                : "bg-nx-accent/30 text-white/40 cursor-not-allowed"
              }`}
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
