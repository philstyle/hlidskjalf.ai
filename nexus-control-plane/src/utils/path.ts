import { platform } from "@tauri-apps/plugin-os";

// Initialize synchronously — platform() returns immediately after first call
let _isWindows: boolean | null = null;

/** Must be called once at app startup (e.g., in App.tsx) before any path functions are used */
export async function initPlatform(): Promise<void> {
  _isWindows = (await platform()) === "windows";
}

function isWindows(): boolean {
  if (_isWindows === null) {
    // Fallback if initPlatform hasn't been called yet — detect from path separators
    // in data we've already received from the backend (paths always use native separators)
    _isWindows = false; // safe default, updated by initPlatform
  }
  return _isWindows;
}

const SEP_RE = /[/\\]/;

/** Split a path into segments, handling both / and \ */
export function splitPath(p: string): string[] {
  return p.split(SEP_RE).filter(Boolean);
}

/** Join path segments with the platform separator */
export function joinPath(...parts: string[]): string {
  const sep = isWindows() ? "\\" : "/";
  return parts.join(sep);
}

/** Get the filename from a full path */
export function basename(p: string): string {
  const segments = splitPath(p);
  return segments[segments.length - 1] || p;
}

/** Get the parent directory */
export function parentPath(p: string): string | null {
  const segments = splitPath(p);
  if (segments.length <= 1) return null;
  if (isWindows()) {
    // Preserve drive prefix: "C:\Users\foo" → "C:\Users"
    const parent = segments.slice(0, -1);
    if (parent[0]?.includes(":")) {
      return parent[0] + "\\" + parent.slice(1).join("\\");
    }
    return parent.join("\\");
  }
  return "/" + segments.slice(0, -1).join("/");
}

/** Check if a path is a filesystem root */
export function isRoot(p: string): boolean {
  if (!p) return false;
  if (p === "/") return true;
  // Windows: "C:\", "D:\", "C:", etc.
  return /^[A-Za-z]:[\\]?$/.test(p);
}

/** Check if a path is absolute */
export function isAbsolute(p: string): boolean {
  if (p.startsWith("/")) return true;
  // Windows: "C:\...", "C:/..."
  return /^[A-Za-z]:[\\/]/.test(p);
}

/** Reconstruct a full path from breadcrumb segments up to index i */
export function reconstructPath(segments: string[], upTo: number): string {
  const sep = isWindows() ? "\\" : "/";
  const joined = segments.slice(0, upTo + 1).join(sep);
  if (isWindows() && segments[0]?.includes(":")) {
    return joined; // "C:\Users\foo" — drive letter already present
  }
  return "/" + joined; // "/Users/foo"
}

/** Platform-aware default root for file browser "home" navigation.
 *  Uses the user's home directory (resolved by Rust backend) rather than
 *  hardcoding "/" or "C:\" — avoids wrong-drive issues on Windows. */
export function defaultRootPath(): string {
  // The file browser's goHome() already calls get_home_dir() from Rust.
  // This is only used as a last-resort fallback.
  return isWindows() ? "C:\\" : "/";
}
