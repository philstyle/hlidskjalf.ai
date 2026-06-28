import { useEffect } from "react";
import { useNavStore } from "./store/nav";
import { usePreviewStore } from "./store/preview";
import { useSettingsStore } from "./store/settings";
import { useLanesStore } from "./store/lanes";
import { useCardsStore } from "./store/cards";
import { useRelayStore } from "./store/relay";
import { initSessionListeners } from "./store/sessions";
import { initPlatform } from "./utils/path";
import AppShell from "./components/AppShell";
import SettingsView from "./views/SettingsView";

export default function App() {
  const currentView = useNavStore((s) => s.currentView);
  const loadSettings = useSettingsStore((s) => s.loadSettings);
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const nccName = useSettingsStore((s) => s.settings.ncc_display_name) || "Nexus Command Center";
  const fetchLanes = useLanesStore((s) => s.fetchLanes);
  const fetchCards = useCardsStore((s) => s.fetchCards);

  useEffect(() => {
    initPlatform();
    initSessionListeners();
    loadSettings();
    fetchLanes();
    fetchCards();
    useRelayStore.getState().fetchRelayInfo();
    // Poll relay info every 10s for pending counts and new registrations
    const relayPoll = setInterval(() => {
      useRelayStore.getState().fetchRelayInfo();
    }, 10_000);
    return () => clearInterval(relayPoll);
  }, []);

  // Apply theme class to <html>
  useEffect(() => {
    const applyTheme = (mode: string) => {
      const isDark = mode === "dark" ||
        (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.classList.toggle("dark", isDark);
    };
    applyTheme(themeMode);
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => { if (themeMode === "system") applyTheme("system"); };
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, [themeMode]);

  useEffect(() => {
    document.title = nccName;
  }, [nccName]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const modKey = e.metaKey || e.ctrlKey;
      if (modKey && e.key === ",") {
        e.preventDefault();
        useNavStore.getState().setView("settings");
      }
      if (modKey && e.key === "w") {
        e.preventDefault();
        const { currentView, activeCardId } = useNavStore.getState();
        if (currentView === "settings") useNavStore.getState().backToBoard();
        else if (activeCardId) useNavStore.getState().deselectCard();
      }
      if (modKey && e.key === "n") {
        e.preventDefault();
        useNavStore.getState().openNewSessionModal();
      }
      if (e.metaKey && e.key === "b") {
        e.preventDefault();
        useNavStore.getState().toggleSidebar();
      }
      if (e.metaKey && e.shiftKey && e.key === "p") {
        e.preventDefault();
        usePreviewStore.getState().toggle();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  if (currentView === "settings") {
    return (
      <div className="h-full w-full bg-nx-bg text-nx-text">
        <SettingsView />
      </div>
    );
  }

  return (
    <div className="h-full w-full bg-nx-bg text-nx-text">
      <AppShell />
    </div>
  );
}
