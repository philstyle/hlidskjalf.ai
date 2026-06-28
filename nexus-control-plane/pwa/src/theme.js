// Theme management — light/dark/system

const THEME_KEY = "nexuslink_theme";

let themeMode = localStorage.getItem(THEME_KEY) || "system";

export function getThemeMode() {
  return themeMode;
}

export function applyTheme() {
  const isDark =
    themeMode === "dark" ||
    (themeMode === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.setAttribute("data-theme", isDark ? "dark" : "light");
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = isDark ? "#0D1117" : "#F3F6FB";
}

export function setTheme(mode) {
  themeMode = mode;
  localStorage.setItem(THEME_KEY, mode);
  applyTheme();
}

export function isDarkActive() {
  return themeMode === "dark" ||
    (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
}

// Initialize theme and listen for system preference changes
applyTheme();
window
  .matchMedia("(prefers-color-scheme: dark)")
  .addEventListener("change", () => {
    if (themeMode === "system") applyTheme();
  });
