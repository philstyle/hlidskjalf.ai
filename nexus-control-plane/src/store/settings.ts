import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

interface AppSettings {
  user_name: string;
  github_org: string;
  workspace_root: string;
  default_shell: string;
  terminal_font_size: number;
  terminal_scrollback: number;
  layout_mode: string;
  sidebar_width: number;
  preview_width: number;
  file_access: string;
  anthropic_api_key: string;
  ai_summaries: string;
  theme_mode: string;
  inbox_path: string;
  ncc_display_name: string;
  ncc_accent_color: string;
}

const DEFAULTS: AppSettings = {
  user_name: "",
  github_org: "",
  workspace_root: "~/.skynexus-sessions",
  default_shell: "",
  terminal_font_size: 13,
  terminal_scrollback: 10000,
  layout_mode: "split",
  sidebar_width: 240,
  preview_width: 350,
  file_access: "off",
  anthropic_api_key: "",
  ai_summaries: "off",
  theme_mode: "system",
  inbox_path: "",
  ncc_display_name: "Nexus Command Center",
  ncc_accent_color: "#3B82F6",
};

const NUMERIC_KEYS = new Set<keyof AppSettings>([
  "terminal_font_size",
  "terminal_scrollback",
  "sidebar_width",
  "preview_width",
]);

interface SettingsState {
  settings: AppSettings;
  loaded: boolean;
  loadSettings: () => Promise<void>;
  updateSetting: (key: keyof AppSettings, value: string) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: { ...DEFAULTS },
  loaded: false,

  loadSettings: async () => {
    const keys = Object.keys(DEFAULTS) as (keyof AppSettings)[];
    const entries = await Promise.all(
      keys.map(async (key) => {
        const val = await invoke<string | null>("get_setting", { key });
        if (val === null) return [key, DEFAULTS[key]] as const;
        if (NUMERIC_KEYS.has(key)) {
          const num = Number(val);
          return [key, isNaN(num) ? DEFAULTS[key] : num] as const;
        }
        return [key, val] as const;
      }),
    );
    const settings = { ...DEFAULTS } as Record<string, string | number>;
    for (const [k, v] of entries) {
      settings[k] = v;
    }
    set({ settings: settings as unknown as AppSettings, loaded: true });
  },

  updateSetting: async (key, value) => {
    await invoke("set_setting", { key, value: String(value) });
    const parsed = NUMERIC_KEYS.has(key) ? Number(value) : value;
    set({
      settings: { ...get().settings, [key]: parsed },
    });
  },
}));
