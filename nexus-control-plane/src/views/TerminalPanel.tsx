import { useEffect, useRef } from "react";
import { Terminal, type ILinkProvider, type ILink, type IBufferLine } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSessionsStore } from "../store/sessions";
import { useSettingsStore } from "../store/settings";
import { useCardsStore } from "../store/cards";
import { useNavStore } from "../store/nav";
import { usePreviewStore } from "../store/preview";
import { isAbsolute, joinPath } from "../utils/path";
import "@xterm/xterm/css/xterm.css";

/**
 * Regex to match file paths ending in .md (absolute or relative).
 * Captures paths like: ./README.md, ../docs/SPEC.md, /Users/foo/bar.md,
 * CLAUDE.md, docs/notes.md, .planning/PLAN.md, C:\docs\README.md
 * Uses [^\s,;:!?"'`(){}\[\]<>] to stop at common delimiters while allowing
 * slashes, dots, hyphens, and underscores within paths.
 */
const MD_PATH_RE = /(?:\.{0,2}[/\\])?[A-Za-z0-9_.][A-Za-z0-9_./\\-]*\.md(?=[\s,;:!?"'`(){}\[\]<>]|$)/g;

function createMarkdownLinkProvider(
  term: Terminal,
  workspacePath: string,
): ILinkProvider {
  return {
    provideLinks(bufferLineNumber: number, callback: (links: ILink[] | undefined) => void) {
      const line: IBufferLine | undefined = term.buffer.active.getLine(bufferLineNumber - 1);
      if (!line) {
        callback(undefined);
        return;
      }

      // Don't trim — use false to preserve character positions
      const text = line.translateToString(false);
      const links: ILink[] = [];

      let match: RegExpExecArray | null;
      MD_PATH_RE.lastIndex = 0;
      while ((match = MD_PATH_RE.exec(text)) !== null) {
        const startX = match.index;
        const matchText = match[0];
        links.push({
          range: {
            start: { x: startX + 1, y: bufferLineNumber },
            end: { x: startX + matchText.length + 1, y: bufferLineNumber },
          },
          text: matchText,
          decorations: { pointerCursor: true, underline: true },
          activate(_event, linkText) {
            const fullPath = isAbsolute(linkText)
              ? linkText
              : joinPath(workspacePath, linkText);
            usePreviewStore.getState().openFile(fullPath);
          },
        });
      }

      callback(links.length > 0 ? links : undefined);
    },
  };
}

interface Session {
  id: string;
  card_id: string;
  is_alive: boolean;
  started_at: string | null;
}

interface AttachResponse {
  data: string;
  seq: number;
}

interface OutputPayload {
  seq: number;
  data: string;
}

/** Read a CSS var containing space-separated RGB channels and return #RRGGBB */
function rgbHex(name: string, fallback: string): string {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  if (!raw) return fallback;
  const parts = raw.split(/\s+/).map(Number);
  if (parts.length !== 3 || parts.some(isNaN)) return fallback;
  return "#" + parts.map((n) => n.toString(16).padStart(2, "0")).join("");
}

function getTermTheme() {
  return {
    background: rgbHex("--nx-term-bg", "#0D1B2A"),
    foreground: rgbHex("--nx-term-fg", "#E2E6EC"),
    cursor: rgbHex("--nx-term-cursor", "#217FF1"),
    selectionBackground: rgbHex("--nx-term-selection", "#1F2B38"),
    black: "#1F2B38",
    red: "#F38BA8",
    green: "#A6E3A1",
    yellow: "#F9E2AF",
    blue: "#79B9FC",
    magenta: "#F5C2E7",
    cyan: "#94E2D5",
    white: "#E2E6EC",
    brightBlack: "#3A4A5C",
    brightRed: "#F38BA8",
    brightGreen: "#A6E3A1",
    brightYellow: "#F9E2AF",
    brightBlue: "#217FF1",
    brightMagenta: "#F5C2E7",
    brightCyan: "#94E2D5",
    brightWhite: "#FFFFFF",
  };
}

export default function TerminalPanel({ cardId }: { cardId: string }) {
  const termRef = useRef<HTMLDivElement>(null);
  const termInstanceRef = useRef<Terminal | null>(null);
  const setSession = useSessionsStore((s) => s.setSession);
  const fontSize = useSettingsStore((s) => s.settings.terminal_font_size);
  const scrollback = useSettingsStore((s) => s.settings.terminal_scrollback);
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const workspacePath = useCardsStore(
    (s) => s.cards.find((c) => c.id === cardId)?.workspace_path ?? "",
  );

  // Live-update terminal theme when user toggles light/dark mode
  useEffect(() => {
    const term = termInstanceRef.current;
    if (term) term.options.theme = getTermTheme();
  }, [themeMode]);

  useEffect(() => {
    const container = termRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize,
      scrollback,
      fontFamily: "Menlo, Consolas, 'DejaVu Sans Mono', 'Liberation Mono', monospace",
      theme: getTermTheme(),
    });
    termInstanceRef.current = term;

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(new WebLinksAddon());
    term.open(container);

    // Register clickable .md file path detection
    if (workspacePath) {
      term.registerLinkProvider(createMarkdownLinkProvider(term, workspacePath));
    }

    let disposed = false;
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;

    requestAnimationFrame(async () => {
      if (disposed) return;
      fitAddon.fit();

      try {
        // 1. Create or get existing session (pass actual dimensions so PTY starts at correct size)
        const session = await invoke<Session>("create_session", {
          cardId,
          cols: term.cols,
          rows: term.rows,
        });
        if (disposed) return;

        const sessionId = session.id;
        setSession(cardId, sessionId, session.started_at);

        // 2. Subscribe FIRST — queue events for gap-free handoff
        const queue: OutputPayload[] = [];
        let liveMode = false;

        unlistenOutput = await listen<OutputPayload>(
          `pty-output-${sessionId}`,
          (event) => {
            if (liveMode) {
              // Only force scroll-to-bottom if user was already at bottom.
              // Otherwise respect their scroll position so they can read history
              // while the agent is responding.
              const buf = term.buffer.active;
              const wasAtBottom = buf.viewportY >= buf.baseY;
              term.write(event.payload.data);
              if (wasAtBottom) {
                term.scrollToBottom();
              }
            } else {
              queue.push(event.payload);
            }
          },
        );
        if (disposed) return;

        // 3. Snapshot buffer
        const attach = await invoke<AttachResponse>("attach_session", {
          sessionId,
        });
        if (disposed) return;

        // 4. Write buffer to terminal
        if (attach.data) {
          term.write(attach.data);
        }

        // 5. Drain queued events, skip duplicates via seq
        for (const ev of queue) {
          if (ev.seq > attach.seq) {
            term.write(ev.data);
          }
        }
        queue.length = 0;
        liveMode = true;

        // Ensure scroll is at the bottom after buffer replay
        term.scrollToBottom();

        // 6. Listen for session exit
        unlistenExit = await listen<string>("session:exit", (event) => {
          if (event.payload === sessionId) {
            term.write("\r\n[Session ended]\r\n");
          }
        });
        if (disposed) return;

        // 7. Send initial resize
        invoke("resize_pty", {
          sessionId,
          cols: term.cols,
          rows: term.rows,
        });

        // 7.5. Send initial command if pending (e.g. "claude --dangerously-skip-permissions")
        const initialCmd = useNavStore.getState().pendingInitialCommand;
        if (initialCmd) {
          useNavStore.getState().clearPendingCommand();
          const isResume = initialCmd.includes("--resume");
          setTimeout(() => {
            if (!disposed) {
              invoke("send_input", { sessionId, data: initialCmd + "\n" });
              // Resume shows a picker — send Enter after delay to select
              if (isResume) {
                setTimeout(() => {
                  if (!disposed) {
                    invoke("send_input", { sessionId, data: "\r\n" });
                  }
                }, 2500);
              }
            }
          }, 500);
        }

        // 8. Forward keystrokes
        term.onData((data) => {
          if (!disposed) {
            invoke("send_input", { sessionId, data });
          }
        });

        // 9. Resize observer
        let resizeTimeout: ReturnType<typeof setTimeout>;
        const resizeObserver = new ResizeObserver(() => {
          clearTimeout(resizeTimeout);
          resizeTimeout = setTimeout(() => {
            if (disposed) return;
            fitAddon.fit();
            invoke("resize_pty", {
              sessionId,
              cols: term.cols,
              rows: term.rows,
            });
          }, 150);
        });
        resizeObserver.observe(container);

        // Store cleanup ref for resize observer
        const origCleanup = term.dispose.bind(term);
        term.dispose = () => {
          resizeObserver.disconnect();
          origCleanup();
        };
      } catch (err) {
        if (!disposed) {
          term.write(`\r\n[ERROR: ${err}]\r\n`);
        }
      }
    });

    return () => {
      disposed = true;
      unlistenOutput?.();
      unlistenExit?.();
      termInstanceRef.current = null;
      term.dispose();
      // Detach, don't kill — PTY keeps running
      invoke("detach_session", { sessionId: "" }).catch(() => {});
    };
  }, [cardId, workspacePath]);

  return (
    <div
      ref={termRef}
      className="h-full w-full"
      style={{ padding: "4px" }}
    />
  );
}
