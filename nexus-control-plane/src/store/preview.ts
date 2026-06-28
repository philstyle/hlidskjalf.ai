import { create } from "zustand";

interface PreviewState {
  filePath: string | null;
  content: string;
  open: boolean;
  openFile: (path: string) => void;
  setContent: (content: string) => void;
  close: () => void;
  toggle: () => void;
}

export const usePreviewStore = create<PreviewState>((set, get) => ({
  filePath: null,
  content: "",
  open: false,

  openFile: (path) => set({ filePath: path, open: true }),

  setContent: (content) => set({ content }),

  close: () => set({ open: false, filePath: null, content: "" }),

  toggle: () => {
    const { open } = get();
    if (open) {
      set({ open: false, filePath: null, content: "" });
    } else {
      set({ open: true });
    }
  },
}));
