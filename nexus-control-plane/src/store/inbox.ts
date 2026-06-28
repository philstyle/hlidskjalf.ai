import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface InboxItem {
  id: string;
  filename: string;
  filepath: string;
  from: string;
  date: string;
  subject: string;
  category: string;
  priority: string;
  workstream: string;
  dueDate: string;
  waitingOn: string;
  status: string;
  gmailId: string;
  // NEW (contract 2026-06-16) — all optional in frontmatter, default "" when absent
  type: string; // action | meeting | fyi | dispatch
  source: string; // email | voice | manual | dispatch
  plannedDate: string; // YYYY-MM-DD — the day Steve intends to DO it (distinct from dueDate)
  summary: string;
  actionItems: { text: string; checked: boolean }[];
  raw: string;
}

export type Horizon = "today" | "week" | "month" | "someday";

interface InboxState {
  items: InboxItem[];
  recentlyDone: InboxItem[];
  loading: boolean;
  error: string | null;
  expandedId: string | null;
  inboxPath: string | null;
  fetchInbox: () => Promise<void>;
  setInboxPath: (path: string) => void;
  toggleExpanded: (id: string) => void;
  collapseAll: () => void;
  dismissItem: (id: string) => Promise<void>;
  toggleActionItem: (itemId: string, actionIndex: number) => Promise<void>;
  quickAdd: (title: string) => Promise<void>;
}

function parseFrontmatter(content: string): { meta: Record<string, string>; body: string } {
  const match = content.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  if (!match) return { meta: {}, body: content };

  const meta: Record<string, string> = {};
  for (const line of match[1].split("\n")) {
    const colonIdx = line.indexOf(":");
    if (colonIdx === -1) continue;
    const key = line.slice(0, colonIdx).trim();
    let value = line.slice(colonIdx + 1).trim();
    // Strip surrounding quotes
    if ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    meta[key] = value;
  }

  return { meta, body: match[2] };
}

function extractActionItems(body: string): { text: string; checked: boolean }[] {
  const items: { text: string; checked: boolean }[] = [];
  for (const line of body.split("\n")) {
    const match = line.match(/^- \[([ x])\] (.+)$/);
    if (match) items.push({ text: match[2], checked: match[1] === "x" });
  }
  return items;
}

function extractSummary(body: string): string {
  const lines = body.split("\n");
  let inSummary = false;
  const summaryLines: string[] = [];

  for (const line of lines) {
    if (line.startsWith("## Summary")) {
      inSummary = true;
      continue;
    }
    if (inSummary && line.startsWith("## ")) break;
    if (inSummary && line.trim()) summaryLines.push(line.trim());
  }

  return summaryLines.join(" ");
}

/** Rewrite frontmatter field in raw file content */
function setFrontmatterField(raw: string, key: string, value: string): string {
  const match = raw.match(/^(---\n)([\s\S]*?)(\n---\n)([\s\S]*)$/);
  if (!match) return raw;

  const lines = match[2].split("\n");
  let found = false;
  for (let i = 0; i < lines.length; i++) {
    const colonIdx = lines[i].indexOf(":");
    if (colonIdx === -1) continue;
    if (lines[i].slice(0, colonIdx).trim() === key) {
      lines[i] = `${key}: ${value}`;
      found = true;
      break;
    }
  }
  if (!found) {
    lines.push(`${key}: ${value}`);
  }

  return match[1] + lines.join("\n") + match[3] + match[4];
}

/** Toggle a checkbox in the markdown body */
function toggleCheckbox(raw: string, actionIndex: number): string {
  let idx = 0;
  return raw.replace(/^- \[([ x])\] /gm, (match, state) => {
    if (idx++ === actionIndex) {
      return state === " " ? "- [x] " : "- [ ] ";
    }
    return match;
  });
}

const priorityOrder: Record<string, number> = { P0: 0, P1: 1, P2: 2, P3: 3 };

/** Local YYYY-MM-DD for a given date (default: now). */
function isoDate(d: Date = new Date()): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** Card name → file slug: lowercase, hyphens, max 50 chars (same convention as workspace slugs). */
function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 50)
    .replace(/-+$/g, "");
}

/**
 * Pure horizon bucketer. effectiveDate = plannedDate || dueDate (both ISO YYYY-MM-DD,
 * so lexicographic compare == date compare). Buckets, per the 2026-06-16 data contract:
 *   today   = effective on/before today (covers "planned today OR overdue")
 *   week    = today < effective <= end of week (Sunday)
 *   month   = end of week < effective <= end of month
 *   someday = no effective date, OR effective beyond this month's horizon
 * Input is assumed pre-sorted (fetchInbox sorts overdue→priority→due), so each
 * bucket inherits that within-group order for free.
 */
export function groupByHorizon(
  items: InboxItem[],
  now: Date = new Date(),
): Record<Horizon, InboxItem[]> {
  const today = isoDate(now);

  // End of week = upcoming Sunday (today if today is Sunday). getDay(): 0=Sun..6=Sat.
  const endOfWeekDate = new Date(now);
  endOfWeekDate.setDate(now.getDate() + ((7 - now.getDay()) % 7));
  const endOfWeek = isoDate(endOfWeekDate);

  // End of month = day 0 of next month.
  const endOfMonth = isoDate(new Date(now.getFullYear(), now.getMonth() + 1, 0));

  const out: Record<Horizon, InboxItem[]> = { today: [], week: [], month: [], someday: [] };
  for (const item of items) {
    const eff = item.plannedDate || item.dueDate;
    if (!eff) out.someday.push(item);
    else if (eff <= today) out.today.push(item);
    else if (eff <= endOfWeek) out.week.push(item);
    else if (eff <= endOfMonth) out.month.push(item);
    else out.someday.push(item); // dated beyond this month → not actionable in the cockpit horizon
  }
  return out;
}

export const useInboxStore = create<InboxState>((set, get) => ({
  items: [],
  recentlyDone: [],
  loading: false,
  error: null,
  expandedId: null,
  inboxPath: null,

  setInboxPath: (path) => set({ inboxPath: path }),

  toggleExpanded: (id) =>
    set((s) => ({ expandedId: s.expandedId === id ? null : id })),

  collapseAll: () => set({ expandedId: null }),

  dismissItem: async (id) => {
    const { items, inboxPath } = get();
    const item = items.find((i) => i.id === id);
    if (!item) return;

    // Write status: done to the file (skip for dispatch PRs — no local file)
    if (item.raw && inboxPath) {
      const updated = setFrontmatterField(item.raw, "status", "done");
      await invoke("write_file", { path: item.filepath, content: updated });
    }

    // Move to recently done, remove from items
    set((s) => ({
      items: s.items.filter((i) => i.id !== id),
      recentlyDone: [...s.recentlyDone, { ...item, status: "done" }],
      expandedId: s.expandedId === id ? null : s.expandedId,
    }));
  },

  toggleActionItem: async (itemId, actionIndex) => {
    const { items } = get();
    const item = items.find((i) => i.id === itemId);
    if (!item) return;

    // Toggle in file (skip for dispatch PRs — no local file)
    let updatedRaw = item.raw;
    if (item.raw) {
      updatedRaw = toggleCheckbox(item.raw, actionIndex);
      await invoke("write_file", { path: item.filepath, content: updatedRaw });
    }

    // Toggle in state
    const updatedActions = item.actionItems.map((a, i) =>
      i === actionIndex ? { ...a, checked: !a.checked } : a,
    );
    set((s) => ({
      items: s.items.map((i) =>
        i.id === itemId ? { ...i, actionItems: updatedActions, raw: updatedRaw } : i,
      ),
    }));
  },

  quickAdd: async (title) => {
    const trimmed = title.trim();
    const { inboxPath } = get();
    if (!trimmed || !inboxPath) return;

    const today = isoDate();
    const slug = slugify(trimmed) || "action";

    // Collision guard: never silently overwrite an existing file. If
    // <today>-<slug>.md exists, append -2, -3, … until the name is free.
    let existing = new Set<string>();
    try {
      const files: { name: string }[] = await invoke("list_directory", { path: inboxPath });
      existing = new Set(files.map((f) => f.name));
    } catch {
      // Can't list — fall through with the base name; write_file still won't traverse.
    }
    let filename = `${today}-${slug}.md`;
    for (let n = 2; existing.has(filename); n++) {
      filename = `${today}-${slug}-${n}.md`;
    }
    const filepath = `${inboxPath}/${filename}`;

    // Escape any double-quotes in the subject for safe YAML.
    const safeSubject = trimmed.replace(/"/g, '\\"');
    const content = `---
from: ""
date: ${today}
subject: "${safeSubject}"
category: action
priority: P2
workstream: ""
status: not-started
type: action
source: manual
planned_date: ${today}
---

## Summary


## Action Items
- [ ] ${trimmed}

## Context

`;

    await invoke("write_file", { path: filepath, content });
    // Re-read inbox so the new item flows through the normal parse + sort path.
    await get().fetchInbox();
  },

  fetchInbox: async () => {
    const { inboxPath } = get();
    if (!inboxPath) return;

    set({ loading: true, error: null });

    try {
      const files: { name: string; is_dir: boolean; path: string }[] =
        await invoke("list_directory", { path: inboxPath });

      const mdFiles = files.filter(
        (f) => !f.is_dir && f.name.endsWith(".md") && f.name !== ".gitkeep",
      );

      const items: InboxItem[] = [];

      for (const file of mdFiles) {
        try {
          const content: string = await invoke("read_file", {
            path: file.path,
          });
          const { meta, body } = parseFrontmatter(content);

          // Skip items marked as done
          if (meta.status === "done") continue;

          items.push({
            id: file.name,
            filename: file.name,
            filepath: file.path,
            from: meta.from || "",
            date: meta.date || "",
            subject: meta.subject || file.name.replace(/\.md$/, ""),
            category: meta.category || "fyi",
            priority: meta.priority || "P3",
            workstream: meta.workstream || "",
            dueDate: meta.due_date || "",
            waitingOn: meta.waiting_on || "",
            status: meta.status || "not-started",
            gmailId: meta.gmail_id || "",
            type: meta.type || "",
            source: meta.source || "",
            plannedDate: meta.planned_date || "",
            summary: extractSummary(body),
            actionItems: extractActionItems(body),
            raw: content,
          });
        } catch {
          // Skip files that can't be read
        }
      }

      // Sort: overdue first, then by priority, then by due date
      const now = new Date();
      items.sort((a, b) => {
        // Overdue items float to top
        const aOverdue = a.dueDate ? new Date(a.dueDate + "T00:00:00") < now : false;
        const bOverdue = b.dueDate ? new Date(b.dueDate + "T00:00:00") < now : false;
        if (aOverdue !== bOverdue) return aOverdue ? -1 : 1;

        // Then by priority
        const pa = priorityOrder[a.priority] ?? 9;
        const pb = priorityOrder[b.priority] ?? 9;
        if (pa !== pb) return pa - pb;

        // Items with unchecked actions above FYI-only items
        const aHasActions = a.actionItems.some((ai) => !ai.checked);
        const bHasActions = b.actionItems.some((ai) => !ai.checked);
        if (aHasActions !== bHasActions) return aHasActions ? -1 : 1;

        // Then by due date
        if (a.dueDate && b.dueDate) return a.dueDate.localeCompare(b.dueDate);
        if (a.dueDate) return -1;
        if (b.dueDate) return 1;
        return b.date.localeCompare(a.date);
      });

      // Dispatch PRs are intentionally NOT merged here — they live in their own
      // Dispatches tab (useDispatchStore). Actions is a pure personal-action
      // cockpit (Steve's call 2026-06-17).

      set({ items, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
}));
