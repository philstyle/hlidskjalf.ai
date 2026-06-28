import { useState, useEffect, useRef, useCallback } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { useLanesStore } from "../store/lanes";
import { useCardsStore } from "../store/cards";
import { useNavStore } from "../store/nav";
import { useSessionsStore } from "../store/sessions";
import { useRelayStore } from "../store/relay";
import LaneIcon from "./icons/LaneIcon";

interface ContextMenuProps {
  x: number;
  y: number;
  cardId: string;
  onClose: () => void;
  onEdit: (id: string) => void;
}

export default function ContextMenu({ x, y, cardId, onClose, onEdit }: ContextMenuProps) {
  const { lanes } = useLanesStore();
  const { cards, deleteCard, moveCard } = useCardsStore();
  const selectCard = useNavStore((s) => s.selectCard);
  const session = useSessionsStore((s) => s.sessions[cardId]);
  const removeSession = useSessionsStore((s) => s.removeSession);
  const [showMoveSubmenu, setShowMoveSubmenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const card = cards.find((c) => c.id === cardId);
  const relayInfo = useRelayStore((s) => s.agents[card?.workspace_path ?? ""]);
  const relaySetMode = useRelayStore((s) => s.setMode);
  const relayClearPending = useRelayStore((s) => s.clearPending);
  const relaySetEnabled = useRelayStore((s) => s.setEnabled);

  // Clamp position to viewport
  const menuWidth = 200;
  const menuHeight = 400;
  const clampedX = Math.min(x, window.innerWidth - menuWidth - 8);
  const clampedY = Math.min(y, window.innerHeight - menuHeight - 8);

  const handleClose = useCallback(() => {
    onClose();
  }, [onClose]);

  // Dismiss on outside click, escape, scroll
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        handleClose();
      }
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") handleClose();
    };
    const handleScroll = () => handleClose();

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleEscape);
    window.addEventListener("scroll", handleScroll, true);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleEscape);
      window.removeEventListener("scroll", handleScroll, true);
    };
  }, [handleClose]);

  if (!card) return null;

  const otherLanes = lanes.filter((l) => l.id !== card.lane_id);

  const handleOpenTerminal = () => {
    selectCard(card.id);
    handleClose();
  };

  const handleKillSession = async () => {
    if (session?.sessionId) {
      await invoke("kill_session", { sessionId: session.sessionId });
      removeSession(cardId);
    }
    handleClose();
  };

  const handleCopyPath = () => {
    navigator.clipboard.writeText(card.workspace_path);
    handleClose();
  };

  const handleOpenInFileManager = () => {
    invoke("open_in_file_manager", { id: card.id });
    handleClose();
  };

  const handleMoveTo = async (laneId: string) => {
    // Place at end of target lane
    const laneCards = cards.filter((c) => c.lane_id === laneId);
    const maxSort = laneCards.reduce((m, c) => Math.max(m, c.sort_order), 0);
    await moveCard(card.id, laneId, maxSort + 1000);
    handleClose();
  };

  const handleDelete = async () => {
    await deleteCard(card.id);
    handleClose();
  };

  const handleSetRelayMode = async (mode: string) => {
    await relaySetMode(card.workspace_path, mode);
    handleClose();
  };

  const handleToggleRelayEnabled = async () => {
    await relaySetEnabled(card.id, !card.relay_enabled);
    useCardsStore.getState().fetchCards();
    handleClose();
  };

  const handleClearPending = async () => {
    await relayClearPending(card.id);
    handleClose();
  };

  const handleReregisterRelay = async () => {
    try {
      await invoke("reregister_relay", { cardId: card.id });
      useRelayStore.getState().fetchRelayInfo();
    } catch (e) {
      console.error("[relay] re-register failed:", e);
    }
    handleClose();
  };

  const itemClass =
    "w-full text-left px-3 py-1.5 text-xs text-nx-text hover:bg-nx-surface-hover transition-colors";
  const disabledClass =
    "w-full text-left px-3 py-1.5 text-xs text-nx-dim cursor-not-allowed";

  return createPortal(
    <div
      ref={menuRef}
      className="fixed z-[100] w-[200px] bg-nx-surface border border-nx-border rounded-xl shadow-nx-xl py-1 overflow-hidden"
      style={{ left: clampedX, top: clampedY }}
    >
      {/* Open Terminal */}
      <button className={itemClass} onClick={handleOpenTerminal}>
        Open Terminal
      </button>

      <button className={itemClass} onClick={() => onEdit(card.id)}>
        Edit Card
      </button>

      {/* Move to Lane */}
      <div
        className="relative"
        onMouseEnter={() => setShowMoveSubmenu(true)}
        onMouseLeave={() => setShowMoveSubmenu(false)}
      >
        <button className={`${itemClass} flex items-center justify-between`}>
          Move to Lane
          <span className="text-nx-dim">&#9656;</span>
        </button>
        {showMoveSubmenu && otherLanes.length > 0 && (
          <div className="absolute left-full top-0 ml-1 w-[180px] bg-nx-surface border border-nx-border rounded-xl shadow-nx-xl py-1">
            {otherLanes.map((lane) => (
              <button
                key={lane.id}
                className={itemClass}
                onClick={() => handleMoveTo(lane.id)}
              >
                <span className="flex items-center gap-1.5">
                  <LaneIcon name={lane.name} className="w-3.5 h-3.5" />
                  {lane.name}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="my-1 border-t border-nx-border-light" />

      <button className={itemClass} onClick={handleCopyPath}>
        Copy Path
      </button>

      <button className={itemClass} onClick={handleOpenInFileManager}>
        Reveal in File Manager
      </button>

      {/* Relay section */}
      <div className="my-1 border-t border-nx-border-light" />

      {relayInfo && (
        <>
          <button
            className={`${itemClass} flex items-center justify-between`}
            onClick={() => handleSetRelayMode('auto')}
          >
            Relay: Auto
            {relayInfo.relayMode === 'auto' && <span className="text-nx-accent">✓</span>}
          </button>
          <button
            className={`${itemClass} flex items-center justify-between`}
            onClick={() => handleSetRelayMode('manual')}
          >
            Relay: Manual
            {relayInfo.relayMode === 'manual' && <span className="text-nx-accent">✓</span>}
          </button>
        </>
      )}

      <button
        className={`${itemClass} flex items-center justify-between`}
        onClick={handleToggleRelayEnabled}
      >
        Enable Relay
        {card.relay_enabled && <span className="text-nx-accent">✓</span>}
      </button>

      {card.relay_enabled && (
        <button className={itemClass} onClick={handleReregisterRelay}>
          Re-register Relay
        </button>
      )}

      {(relayInfo?.pendingCount ?? 0) > 0 && (
        <button
          className="w-full text-left px-3 py-1.5 text-xs text-orange-400 hover:bg-orange-500/10 transition-colors"
          onClick={handleClearPending}
        >
          Clear Pending ({relayInfo!.pendingCount})
        </button>
      )}

      <div className="my-1 border-t border-nx-border-light" />

      {/* Kill Session — enabled when session exists and is alive */}
      {session?.isAlive ? (
        <button
          className="w-full text-left px-3 py-1.5 text-xs text-orange-400 hover:bg-orange-500/10 transition-colors"
          onClick={handleKillSession}
        >
          Kill Session
        </button>
      ) : (
        <button className={disabledClass} disabled>
          Kill Session
        </button>
      )}

      {/* Delete with inline confirm */}
      {!confirmDelete ? (
        <button
          className="w-full text-left px-3 py-1.5 text-xs text-red-400 hover:bg-red-500/10 transition-colors"
          onClick={() => setConfirmDelete(true)}
        >
          Delete Card
        </button>
      ) : (
        <div className="flex items-center gap-1 px-3 py-1.5">
          <span className="text-xs text-red-400">Delete?</span>
          <button
            className="px-2 py-0.5 text-[10px] font-medium text-red-400 bg-red-500/10 rounded hover:bg-red-500/20 transition-colors"
            onClick={handleDelete}
          >
            Yes
          </button>
          <button
            className="px-2 py-0.5 text-[10px] font-medium text-nx-muted hover:text-nx-text transition-colors"
            onClick={() => setConfirmDelete(false)}
          >
            No
          </button>
        </div>
      )}
    </div>,
    document.body
  );
}
