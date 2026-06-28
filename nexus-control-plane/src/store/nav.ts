import { create } from "zustand";

type View = "board" | "settings";
type SidebarMode = "sessions" | "files";
export type MissionControlTab = "actions" | "calendar" | "tools";

interface NavState {
  currentView: View;
  activeCardId: string | null;
  showNewSessionModal: boolean;
  sidebarCollapsed: boolean;
  sidebarMode: SidebarMode;
  pendingInitialCommand: string | null;
  missionControlTab: MissionControlTab;
  setView: (view: View) => void;
  selectCard: (cardId: string) => void;
  selectCardWithCommand: (cardId: string, command: string) => void;
  deselectCard: () => void;
  backToBoard: () => void;
  openNewSessionModal: () => void;
  closeNewSessionModal: () => void;
  toggleSidebar: () => void;
  setSidebarMode: (mode: SidebarMode) => void;
  clearPendingCommand: () => void;
  setMissionControlTab: (tab: MissionControlTab) => void;
}

export const useNavStore = create<NavState>((set) => ({
  currentView: "board",
  activeCardId: null,
  showNewSessionModal: false,
  sidebarCollapsed: false,
  sidebarMode: "sessions",
  pendingInitialCommand: null,
  missionControlTab: "actions",
  setView: (view) => set({ currentView: view }),
  selectCard: (cardId) => set({ activeCardId: cardId }),
  selectCardWithCommand: (cardId, command) =>
    set({ activeCardId: cardId, pendingInitialCommand: command }),
  deselectCard: () => set({ activeCardId: null }),
  backToBoard: () => set({ currentView: "board" }),
  openNewSessionModal: () => set({ showNewSessionModal: true }),
  closeNewSessionModal: () => set({ showNewSessionModal: false }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarMode: (mode) => set({ sidebarMode: mode }),
  clearPendingCommand: () => set({ pendingInitialCommand: null }),
  setMissionControlTab: (tab) => set({ missionControlTab: tab }),
}));
