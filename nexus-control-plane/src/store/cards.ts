import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface Card {
  id: string;
  name: string;
  lane_id: string;
  notes: string | null;
  source_type: string;
  repo_url: string | null;
  repo_name: string | null;
  workspace_path: string;
  is_app_managed: boolean;
  process_name: string | null;
  telemetry_enabled: boolean;
  sort_order: number;
  created_at: string;
  updated_at: string;
  last_active_at: string | null;
  relay_enabled: boolean;
}

export interface CreateCardInput {
  name: string;
  lane_id: string;
  workspace_path: string;
  notes?: string | null;
  source_type?: string;
  repo_url?: string;
  repo_name?: string;
  is_app_managed?: boolean;
}

export interface UpdateCardInput {
  id: string;
  name: string;
  notes?: string | null;
}

interface CardsState {
  cards: Card[];
  loading: boolean;
  error: string | null;
  fetchCards: () => Promise<void>;
  createCard: (input: CreateCardInput) => Promise<Card>;
  updateCard: (input: UpdateCardInput) => Promise<Card>;
  deleteCard: (id: string) => Promise<void>;
  moveCard: (id: string, laneId: string, sortOrder: number) => Promise<void>;
}

export const useCardsStore = create<CardsState>((set, get) => ({
  cards: [],
  loading: false,
  error: null,
  fetchCards: async () => {
    set({ loading: true, error: null });
    try {
      const cards = await invoke<Card[]>("list_cards");
      set({ cards, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  createCard: async (input) => {
    const card = await invoke<Card>("create_card", { input });
    set((s) => ({ cards: [...s.cards, card] }));
    return card;
  },
  updateCard: async (input) => {
    const card = await invoke<Card>("update_card", { input });
    set((s) => ({
      cards: s.cards.map((c) => (c.id === card.id ? card : c)),
    }));
    return card;
  },
  deleteCard: async (id) => {
    await invoke("delete_card", { id });
    set((s) => ({ cards: s.cards.filter((c) => c.id !== id) }));
  },
  moveCard: async (id, laneId, sortOrder) => {
    const snapshot = get().cards;
    // Optimistic update
    set((s) => ({
      cards: s.cards.map((c) =>
        c.id === id ? { ...c, lane_id: laneId, sort_order: sortOrder } : c
      ),
    }));
    try {
      await invoke("move_card", { input: { id, lane_id: laneId, sort_order: sortOrder } });
    } catch {
      // Revert on failure
      set({ cards: snapshot });
    }
  },
}));
