import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

interface RelayInfo {
  workspacePath: string;
  relayMode: string;
  pendingCount: number;
}

interface RelayState {
  agents: Record<string, RelayInfo>;
  loading: boolean;
  fetchRelayInfo: () => Promise<void>;
  setMode: (workspacePath: string, mode: string) => Promise<void>;
  clearPending: (cardId: string) => Promise<void>;
  setEnabled: (cardId: string, enabled: boolean) => Promise<void>;
}

export const useRelayStore = create<RelayState>((set, get) => ({
  agents: {},
  loading: false,
  fetchRelayInfo: async () => {
    set({ loading: true });
    try {
      const infos = await invoke<
        { workspace_path: string; relay_mode: string; pending_count: number }[]
      >("list_relay_info");
      const agents: Record<string, RelayInfo> = {};
      for (const info of infos) {
        agents[info.workspace_path] = {
          workspacePath: info.workspace_path,
          relayMode: info.relay_mode,
          pendingCount: info.pending_count,
        };
      }
      set({ agents, loading: false });
    } catch {
      set({ loading: false });
    }
  },
  setMode: async (workspacePath, mode) => {
    await invoke("set_relay_mode", { workspacePath, mode });
    // Optimistic update
    const current = get().agents[workspacePath];
    if (current) {
      set((s) => ({
        agents: { ...s.agents, [workspacePath]: { ...current, relayMode: mode } },
      }));
    }
  },
  clearPending: async (cardId) => {
    await invoke("clear_relay_pending", { cardId });
    await get().fetchRelayInfo();
  },
  setEnabled: async (cardId, enabled) => {
    await invoke("set_relay_enabled", { cardId, enabled });
    // Re-fetch after toggle — registration may have happened server-side
    await get().fetchRelayInfo();
  },
}));
