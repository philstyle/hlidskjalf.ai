import { useState, useEffect, useCallback, useMemo } from "react";
import {
  DndContext,
  DragOverlay,
  PointerSensor,
  useSensor,
  useSensors,
  closestCenter,
  type DragStartEvent,
  type DragOverEvent,
  type DragEndEvent,
} from "@dnd-kit/core";
import { arrayMove } from "@dnd-kit/sortable";
import { useLanesStore } from "../../store/lanes";
import { useCardsStore, type Card } from "../../store/cards";
import { useNavStore } from "../../store/nav";
import LaneSection from "./LaneSection";
import SidebarHeader from "./SidebarHeader";
import SidebarFooter from "./SidebarFooter";
import UsageBars from "./UsageBars";
import FileBrowser from "./FileBrowser";
import ContextMenu from "../ContextMenu";
import EditCardModal from "../EditCardModal";

// Lanes that default to expanded
const EXPANDED_LANES = new Set(["active", "queued"]);

export default function Sidebar() {
  const sidebarMode = useNavStore((s) => s.sidebarMode);
  const { lanes } = useLanesStore();
  const { cards, moveCard } = useCardsStore();

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    cardId: string;
  } | null>(null);
  const [editCardId, setEditCardId] = useState<string | null>(null);

  // DnD state
  const [activeCardId, setActiveCardId] = useState<string | null>(null);
  const [overLaneId, setOverLaneId] = useState<string | null>(null);
  const [localCards, setLocalCards] = useState<Card[]>([]);

  // Sync store cards -> local cards when not mid-drag
  useEffect(() => {
    if (!activeCardId) {
      setLocalCards(cards);
    }
  }, [cards, activeCardId]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );

  // Group cards by lane, sorted by sort_order
  const cardsByLane = useMemo(() => {
    const map = new Map<string, Card[]>();
    for (const lane of lanes) {
      map.set(lane.id, []);
    }
    for (const card of localCards) {
      const arr = map.get(card.lane_id);
      if (arr) arr.push(card);
    }
    for (const arr of map.values()) {
      arr.sort((a, b) => a.sort_order - b.sort_order);
    }
    return map;
  }, [localCards, lanes]);

  // Resolve which lane an "over" id belongs to
  const resolveLaneId = useCallback(
    (overId: string): string | null => {
      if (lanes.some((l) => l.id === overId)) return overId;
      const card = localCards.find((c) => c.id === overId);
      return card?.lane_id ?? null;
    },
    [lanes, localCards],
  );

  const handleDragStart = (event: DragStartEvent) => {
    setActiveCardId(event.active.id as string);
  };

  const handleDragOver = (event: DragOverEvent) => {
    const { active, over } = event;
    if (!over) {
      setOverLaneId(null);
      return;
    }

    const targetLaneId = resolveLaneId(over.id as string);
    setOverLaneId(targetLaneId);

    // Cross-lane: optimistically move card's lane_id
    const activeId = active.id as string;
    const card = localCards.find((c) => c.id === activeId);
    if (card && targetLaneId && card.lane_id !== targetLaneId) {
      setLocalCards((prev) =>
        prev.map((c) =>
          c.id === activeId ? { ...c, lane_id: targetLaneId } : c,
        ),
      );
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    setActiveCardId(null);
    setOverLaneId(null);

    if (!over) {
      setLocalCards(cards);
      return;
    }

    const activeId = active.id as string;
    const targetLaneId = resolveLaneId(over.id as string);
    if (!targetLaneId) {
      setLocalCards(cards);
      return;
    }

    const laneCards = localCards
      .filter((c) => c.lane_id === targetLaneId)
      .sort((a, b) => a.sort_order - b.sort_order);

    const oldIndex = laneCards.findIndex((c) => c.id === activeId);
    const overCard = laneCards.find((c) => c.id === (over.id as string));
    const newIndex = overCard
      ? laneCards.findIndex((c) => c.id === overCard.id)
      : laneCards.length - 1;

    const reordered =
      oldIndex !== -1 && newIndex !== -1
        ? arrayMove(laneCards, oldIndex, newIndex)
        : laneCards;

    // Reassign sort orders
    const updatedCards = localCards.map((c) => {
      if (c.lane_id !== targetLaneId) return c;
      const idx = reordered.findIndex((r) => r.id === c.id);
      if (idx === -1) return c;
      return { ...c, sort_order: (idx + 1) * 1000 };
    });

    setLocalCards(updatedCards);

    // Persist
    for (const [i, c] of reordered.entries()) {
      moveCard(c.id, targetLaneId, (i + 1) * 1000);
    }
  };

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, cardId: string) => {
      setContextMenu({ x: e.clientX, y: e.clientY, cardId });
    },
    [],
  );

  const closeContextMenu = useCallback(() => setContextMenu(null), []);

  // Active card for drag overlay
  const activeCard = activeCardId
    ? localCards.find((c) => c.id === activeCardId)
    : null;
  const activeCardLane = activeCard
    ? lanes.find((l) => l.id === activeCard.lane_id)
    : null;

  return (
    <div className="flex flex-col h-full bg-nx-surface border-r border-nx-border">
      <SidebarHeader />

      {sidebarMode === "files" ? (
        <FileBrowser />
      ) : (
        <>
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragStart={handleDragStart}
            onDragOver={handleDragOver}
            onDragEnd={handleDragEnd}
          >
            {/* Scrollable lane sections */}
            <div className="flex-1 overflow-y-auto py-2 px-1.5 space-y-1">
              {lanes.map((lane) => (
                <LaneSection
                  key={lane.id}
                  lane={lane}
                  cards={cardsByLane.get(lane.id) ?? []}
                  isOver={overLaneId === lane.id}
                  defaultExpanded={EXPANDED_LANES.has(lane.name.toLowerCase())}
                  onContextMenu={handleContextMenu}
                />
              ))}
            </div>

            <DragOverlay>
              {activeCard ? (
                <div className="flex items-center gap-2 px-2 py-1.5 bg-nx-surface rounded-lg shadow-nx-md border border-nx-accent/20">
                  <div
                    className="w-0.5 h-6 rounded-full shrink-0"
                    style={{ backgroundColor: activeCardLane?.color ?? "#888" }}
                  />
                  <span className="text-xs font-body text-nx-text truncate">
                    {activeCard.name}
                  </span>
                </div>
              ) : null}
            </DragOverlay>
          </DndContext>

          <UsageBars />
          <SidebarFooter />
        </>
      )}

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          cardId={contextMenu.cardId}
          onClose={closeContextMenu}
          onEdit={(id) => {
            closeContextMenu();
            setEditCardId(id);
          }}
        />
      )}

      {/* Edit card modal */}
      {editCardId && (
        <EditCardModal
          cardId={editCardId}
          onClose={() => setEditCardId(null)}
        />
      )}
    </div>
  );
}
