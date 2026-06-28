import { useState, useEffect, useRef } from "react";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { listen } from "@tauri-apps/api/event";
import type { Card } from "../../store/cards";
import { useNavStore } from "../../store/nav";
import { useSessionsStore } from "../../store/sessions";
import { useSummariesStore } from "../../store/summaries";
import ClaudeStatusBadge from "../ClaudeStatusBadge";
import { useRelayStore } from "../../store/relay";
import RelayBadge from "../RelayBadge";

function formatDuration(startedAt: string): string {
  const ms = Date.now() - new Date(startedAt).getTime();
  const mins = Math.floor(ms / 60_000);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ${mins % 60}m`;
}

const ACTIVE_THRESHOLD_MS = 8_000;

type SessionActivity = "active" | "waiting" | "dead";

const activityDot: Record<SessionActivity, string> = {
  active: "bg-green-400 animate-pulse",
  waiting: "bg-amber-400",
  dead: "bg-gray-400",
};

interface SessionItemProps {
  card: Card;
  laneColor: string;
  onContextMenu: (e: React.MouseEvent, cardId: string) => void;
}

export default function SessionItem({ card, laneColor, onContextMenu }: SessionItemProps) {
  const selectCard = useNavStore((s) => s.selectCard);
  const activeCardId = useNavStore((s) => s.activeCardId);
  const session = useSessionsStore((s) => s.sessions[card.id]);
  const isSelected = activeCardId === card.id;
  const relayInfo = useRelayStore((s) => s.agents[card.workspace_path]);
  const [, tick] = useState(0);
  const lastActivityRef = useRef<number>(0);
  const summary = useSummariesStore((s) => s.summaries[card.id]);
  const fetchSummary = useSummariesStore((s) => s.fetchSummary);

  // Listen directly to raw PTY output events for this session — bypasses store entirely
  useEffect(() => {
    if (!session?.sessionId || !session?.isAlive) return;
    let cleanup: (() => void) | null = null;
    const setup = listen(`pty-output-${session.sessionId}`, () => {
      lastActivityRef.current = Date.now();
    });
    setup.then((unlisten) => { cleanup = unlisten; });
    return () => { cleanup?.(); };
  }, [session?.sessionId, session?.isAlive]);

  const activity: SessionActivity = !session
    ? "waiting"
    : !session.isAlive
      ? "dead"
      : lastActivityRef.current > 0 && Date.now() - lastActivityRef.current < ACTIVE_THRESHOLD_MS
        ? "active"
        : "waiting";

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: card.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
  };

  // Poll for AI summary every 30s
  useEffect(() => {
    if (!session?.sessionId || !session?.isAlive) return;
    fetchSummary(card.id, session.sessionId);
    const id = setInterval(() => fetchSummary(card.id, session.sessionId), 30_000);
    return () => clearInterval(id);
  }, [session?.sessionId, session?.isAlive, card.id, fetchSummary]);

  // Tick every 2s for alive sessions to update activity state + duration
  useEffect(() => {
    if (!session?.isAlive) return;
    const id = setInterval(() => tick((t) => t + 1), 2_000);
    return () => clearInterval(id);
  }, [session?.isAlive]);

  const selectCardWithCommand = useNavStore((s) => s.selectCardWithCommand);

  const handleClick = () => selectCard(card.id);

  const handleResume = (e: React.MouseEvent) => {
    e.stopPropagation();
    selectCardWithCommand(card.id, "claude --resume --dangerously-skip-permissions");
  };

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    onContextMenu(e, card.id);
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
      className={`flex items-center gap-2 px-2 py-1.5 rounded-lg cursor-pointer select-none transition-colors
        ${isSelected
          ? "bg-nx-accent-light border border-nx-accent/20"
          : "hover:bg-nx-surface-hover border border-transparent"
        }
        ${isDragging ? "shadow-nx-md" : ""}`}
    >
      {/* Lane color bar */}
      <div
        className="w-0.5 self-stretch rounded-full shrink-0"
        style={{ backgroundColor: laneColor }}
      />

      {/* Activity indicator */}
      {session && (
        session.claudeState ? (
          <ClaudeStatusBadge
            state={session.claudeState}
            tool={session.claudeTool}
            agentCount={session.agentCount}
            compact
          />
        ) : (
          <span
            className={`inline-block w-1.5 h-1.5 rounded-full shrink-0 mt-1 ${
              activityDot[activity]
            }`}
          />
        )
      )}

      {/* Name + summary */}
      <div className="flex-1 min-w-0">
        <span className="text-xs font-body text-nx-text truncate block">
          {card.name}
        </span>
        {card.workspace_path && (
          <span className="text-[10px] text-nx-muted truncate block" title={card.workspace_path}>
            {card.source_type} · {card.workspace_path.split("/").slice(-2).join("/")}
          </span>
        )}
        {summary && (
          <span className="text-[10px] text-nx-dim font-secondary block mt-0.5 break-words whitespace-normal leading-tight">
            {summary}
          </span>
        )}
      </div>

      {/* Duration */}
      {session?.isAlive && session.startedAt && (
        <span className="text-[10px] text-nx-muted shrink-0 font-secondary">
          {formatDuration(session.startedAt)}
        </span>
      )}

      {/* Resume button for dead sessions */}
      {session && !session.isAlive && (
        <button
          onClick={handleResume}
          className="text-[11px] px-1.5 py-0.5 rounded text-nx-muted hover:text-nx-accent hover:bg-nx-accent-light/50 transition-colors shrink-0"
          title="Resume session"
        >
          ↻
        </button>
      )}

      {/* Relay indicator */}
      {card.relay_enabled && (
        <div className="flex items-center gap-1 shrink-0">
          <span
            className="text-[10px] text-nx-dim"
            title={relayInfo?.relayMode === 'auto' ? 'Relay: Auto' : 'Relay: Manual'}
          >
            {relayInfo?.relayMode === 'auto' ? '📡' : '⏸'}
          </span>
          {(relayInfo?.pendingCount ?? 0) > 0 && (
            <RelayBadge count={relayInfo!.pendingCount} />
          )}
        </div>
      )}
    </div>
  );
}
