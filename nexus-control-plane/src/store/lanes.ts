import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface Lane {
  id: string;
  name: string;
  emoji: string;
  color: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

interface LanesState {
  lanes: Lane[];
  loading: boolean;
  error: string | null;
  fetchLanes: () => Promise<void>;
  updateLane: (id: string, name: string) => Promise<void>;
  deleteLane: (id: string) => Promise<void>;
  reorderLanes: (orderedIds: string[]) => Promise<void>;
}

export const useLanesStore = create<LanesState>((set) => ({
  lanes: [],
  loading: false,
  error: null,
  fetchLanes: async () => {
    set({ loading: true, error: null });
    try {
      const lanes = await invoke<Lane[]>("list_lanes");
      set({ lanes, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  updateLane: async (id, name) => {
    const updated = await invoke<Lane>("update_lane", { id, name });
    set((s) => ({
      lanes: s.lanes.map((l) => (l.id === updated.id ? updated : l)),
    }));
  },
  deleteLane: async (id) => {
    await invoke("delete_lane", { id });
    set((s) => ({ lanes: s.lanes.filter((l) => l.id !== id) }));
  },
  reorderLanes: async (orderedIds) => {
    const order = orderedIds.map((id, i) => ({
      id,
      sort_order: (i + 1) * 1000,
    }));
    await invoke("reorder_lanes", { order });
    set((s) => {
      const updated = s.lanes.map((l) => {
        const o = order.find((item) => item.id === l.id);
        return o ? { ...l, sort_order: o.sort_order } : l;
      });
      updated.sort((a, b) => a.sort_order - b.sort_order);
      return { lanes: updated };
    });
  },
}));
