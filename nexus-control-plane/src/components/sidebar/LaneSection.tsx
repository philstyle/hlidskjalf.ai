import { useState } from "react";
import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import type { Lane } from "../../store/lanes";
import type { Card } from "../../store/cards";
import SessionItem from "./SessionItem";
import LaneIcon from "../icons/LaneIcon";

interface LaneSectionProps {
  lane: Lane;
  cards: Card[];
  isOver: boolean;
  defaultExpanded: boolean;
  onContextMenu: (e: React.MouseEvent, cardId: string) => void;
}

export default function LaneSection({
  lane,
  cards,
  isOver,
  defaultExpanded,
  onContextMenu,
}: LaneSectionProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const { setNodeRef } = useDroppable({ id: lane.id });

  const cardIds = cards.map((c) => c.id);

  return (
    <div ref={setNodeRef}>
      {/* Lane header — click to toggle */}
      <button
        onClick={() => setExpanded((e) => !e)}
        className={`w-full flex items-center justify-between px-3 py-1.5 rounded-lg transition-colors
          ${isOver ? "bg-nx-accent-light" : "hover:bg-nx-surface-hover"}`}
      >
        <div className="flex items-center gap-1.5">
          {/* Chevron */}
          <svg
            className={`w-3 h-3 text-nx-muted transition-transform ${expanded ? "rotate-90" : ""}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="m9 5 7 7-7 7" />
          </svg>
          <LaneIcon name={lane.name} className="w-3.5 h-3.5 text-nx-muted" />
          <span className="text-xs font-body font-medium text-nx-text-secondary uppercase tracking-wider">
            {lane.name}
          </span>
        </div>
        <span className="text-[10px] text-nx-muted bg-nx-bg px-1.5 py-0.5 rounded-full font-secondary">
          {cards.length}
        </span>
      </button>

      {/* Card list */}
      {expanded && (
        <SortableContext items={cardIds} strategy={verticalListSortingStrategy}>
          <div className="flex flex-col gap-0.5 mt-0.5 ml-2">
            {cards.map((card) => (
              <SessionItem
                key={card.id}
                card={card}
                laneColor={lane.color}
                onContextMenu={onContextMenu}
              />
            ))}
            {cards.length === 0 && (
              <div className="px-3 py-2 text-[10px] text-nx-muted italic">
                No cards
              </div>
            )}
          </div>
        </SortableContext>
      )}
    </div>
  );
}
