import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable";
import { restrictToVerticalAxis } from "@dnd-kit/modifiers";
import { CSS } from "@dnd-kit/utilities";
import { useLanesStore, type Lane } from "../store/lanes";
import { useCardsStore } from "../store/cards";
import LaneIcon from "./icons/LaneIcon";

function SortableLaneRow({
  lane,
  cardCount,
  isLastLane,
  onRename,
  onDelete,
}: {
  lane: Lane;
  cardCount: number;
  isLastLane: boolean;
  onRename: (id: string, name: string) => Promise<void>;
  onDelete: (id: string) => Promise<string | null>;
}) {
  const [editName, setEditName] = useState(lane.name);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: lane.id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  // Sync editName when lane.name changes from external updates
  useEffect(() => {
    setEditName(lane.name);
  }, [lane.name]);

  const handleBlur = useCallback(async () => {
    const trimmed = editName.trim();
    if (!trimmed || trimmed === lane.name) {
      setEditName(lane.name);
      return;
    }
    try {
      await onRename(lane.id, trimmed);
    } catch {
      setEditName(lane.name);
    }
  }, [editName, lane.id, lane.name, onRename]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      (e.target as HTMLInputElement).blur();
    }
    if (e.key === "Escape") {
      setEditName(lane.name);
      (e.target as HTMLInputElement).blur();
    }
  };

  const handleDelete = async () => {
    setConfirmDelete(false);
    const err = await onDelete(lane.id);
    if (err) {
      setError(err);
      setTimeout(() => setError(null), 3000);
    }
  };

  const deleteDisabled = isLastLane || cardCount > 0;

  return (
    <div ref={setNodeRef} style={style}>
      <div className="flex items-center gap-2 py-1.5">
        {/* Drag handle */}
        <button
          className="cursor-grab active:cursor-grabbing text-nx-dim hover:text-nx-muted transition-colors shrink-0 touch-none"
          {...attributes}
          {...listeners}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
            <circle cx="4" cy="2" r="1" />
            <circle cx="8" cy="2" r="1" />
            <circle cx="4" cy="6" r="1" />
            <circle cx="8" cy="6" r="1" />
            <circle cx="4" cy="10" r="1" />
            <circle cx="8" cy="10" r="1" />
          </svg>
        </button>

        {/* Lane icon */}
        <LaneIcon name={lane.name} className="w-4 h-4 text-nx-muted shrink-0" />

        {/* Name input */}
        <input
          type="text"
          value={editName}
          onChange={(e) => setEditName(e.target.value)}
          onBlur={handleBlur}
          onKeyDown={handleKeyDown}
          maxLength={30}
          className="flex-1 px-2 py-1 text-sm bg-nx-bg border border-nx-border rounded text-nx-text focus:outline-none focus:border-nx-accent/50"
        />

        {/* Card count badge */}
        {cardCount > 0 && (
          <span className="text-[10px] text-nx-dim shrink-0">
            {cardCount} card{cardCount !== 1 ? "s" : ""}
          </span>
        )}

        {/* Delete button / inline confirm */}
        {!confirmDelete ? (
          <button
            onClick={() => setConfirmDelete(true)}
            disabled={deleteDisabled}
            className={`shrink-0 text-xs px-1.5 py-0.5 rounded transition-colors ${
              deleteDisabled
                ? "text-nx-dim/30 cursor-not-allowed"
                : "text-red-400 hover:bg-red-500/10"
            }`}
            title={
              isLastLane
                ? "Cannot delete the last lane"
                : cardCount > 0
                  ? "Move cards out first"
                  : "Delete lane"
            }
          >
            🗑
          </button>
        ) : (
          <div className="flex items-center gap-1 shrink-0">
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
      </div>

      {/* Error message */}
      {error && (
        <p className="text-[10px] text-red-400 ml-[52px] -mt-0.5 mb-1">
          {error}
        </p>
      )}
    </div>
  );
}

export default function LaneEditor() {
  const { lanes, updateLane, deleteLane, reorderLanes } = useLanesStore();
  const cards = useCardsStore((s) => s.cards);
  const [defaultLaneId, setDefaultLaneId] = useState<string | null>(null);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // Load default lane setting on mount
  useEffect(() => {
    invoke<string | null>("get_setting", { key: "default_lane_id" }).then(
      (val) => {
        setDefaultLaneId(val ?? (lanes.length > 0 ? lanes[0].id : null));
      }
    );
  }, [lanes]);

  // If current default lane was deleted, fall back to first remaining
  useEffect(() => {
    if (
      defaultLaneId &&
      lanes.length > 0 &&
      !lanes.find((l) => l.id === defaultLaneId)
    ) {
      const fallback = lanes[0].id;
      setDefaultLaneId(fallback);
      invoke("set_setting", { key: "default_lane_id", value: fallback });
    }
  }, [lanes, defaultLaneId]);

  const handleRename = useCallback(
    async (id: string, name: string) => {
      await updateLane(id, name);
    },
    [updateLane]
  );

  const handleDelete = useCallback(
    async (id: string): Promise<string | null> => {
      try {
        await deleteLane(id);
        return null;
      } catch (e) {
        return String(e);
      }
    },
    [deleteLane]
  );

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;

      const oldIndex = lanes.findIndex((l) => l.id === active.id);
      const newIndex = lanes.findIndex((l) => l.id === over.id);
      const reordered = arrayMove(lanes, oldIndex, newIndex);
      reorderLanes(reordered.map((l) => l.id));
    },
    [lanes, reorderLanes]
  );

  const handleDefaultChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value;
    setDefaultLaneId(value);
    invoke("set_setting", { key: "default_lane_id", value });
  };

  const cardCountMap = new Map<string, number>();
  for (const card of cards) {
    cardCountMap.set(card.lane_id, (cardCountMap.get(card.lane_id) ?? 0) + 1);
  }

  const isLastLane = lanes.length <= 1;

  return (
    <div>
      {/* Sortable lane list */}
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        modifiers={[restrictToVerticalAxis]}
        onDragEnd={handleDragEnd}
      >
        <SortableContext
          items={lanes.map((l) => l.id)}
          strategy={verticalListSortingStrategy}
        >
          <div className="space-y-0.5">
            {lanes.map((lane) => (
              <SortableLaneRow
                key={lane.id}
                lane={lane}
                cardCount={cardCountMap.get(lane.id) ?? 0}
                isLastLane={isLastLane}
                onRename={handleRename}
                onDelete={handleDelete}
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>

      {/* Default lane selector */}
      <div className="mt-4 flex items-center gap-3">
        <label className="text-xs text-nx-muted whitespace-nowrap">
          Default lane for new cards:
        </label>
        <select
          value={defaultLaneId ?? ""}
          onChange={handleDefaultChange}
          className="flex-1 px-2 py-1.5 text-sm bg-nx-bg border border-nx-border rounded text-nx-text focus:outline-none focus:border-nx-accent/50"
        >
          {lanes.map((lane) => (
            <option key={lane.id} value={lane.id}>
              {lane.name}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
