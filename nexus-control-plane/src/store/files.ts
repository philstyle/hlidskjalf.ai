import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { isRoot, parentPath } from "../utils/path";

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  hidden: boolean;
}

interface FilesState {
  currentPath: string | null;
  entries: FileEntry[];
  loading: boolean;
  error: string | null;
  showHidden: boolean;

  /** Navigate to a directory and list its contents */
  navigateTo: (path: string) => Promise<void>;

  /** Navigate to the user's home directory */
  goHome: () => Promise<void>;

  /** Navigate to the parent directory */
  goUp: () => Promise<void>;

  /** Copy files/folders to a destination */
  copyFiles: (sources: string[], destination: string) => Promise<number>;

  /** Read a file's text content */
  readFile: (path: string) => Promise<string>;

  /** Toggle hidden file visibility */
  toggleHidden: () => void;

  /** Clear file browser state */
  clear: () => void;
}

export const useFilesStore = create<FilesState>((set, get) => ({
  currentPath: null,
  entries: [],
  loading: false,
  error: null,
  showHidden: false,

  navigateTo: async (path) => {
    set({ loading: true, error: null });
    try {
      const entries = await invoke<FileEntry[]>("list_directory", { path });
      set({ currentPath: path, entries, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  goHome: async () => {
    try {
      const home = await invoke<string>("get_home_dir");
      await get().navigateTo(home);
    } catch (e) {
      set({ error: String(e) });
    }
  },

  goUp: async () => {
    const { currentPath, navigateTo } = get();
    if (!currentPath || isRoot(currentPath)) return;
    const parent = parentPath(currentPath) || currentPath;
    await navigateTo(parent);
  },

  copyFiles: async (sources, destination) => {
    const count = await invoke<number>("copy_files", { sources, destination });
    // Refresh current directory if we copied into it
    const { currentPath, navigateTo } = get();
    if (currentPath === destination) {
      await navigateTo(destination);
    }
    return count;
  },

  readFile: async (path) => {
    return invoke<string>("read_file", { path });
  },

  toggleHidden: () => set((s) => ({ showHidden: !s.showHidden })),

  clear: () =>
    set({ currentPath: null, entries: [], loading: false, error: null }),
}));
