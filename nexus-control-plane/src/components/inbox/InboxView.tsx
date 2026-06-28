import { useMemo, useState } from "react";
import { useInboxStore, groupByHorizon, type Horizon } from "../../store/inbox";
import InboxItem from "./InboxItem";

const COLUMNS: { key: Horizon; label: string; icon: string }[] = [
  // Today — sun
  { key: "today", label: "Today", icon: "M12 3v2.25m6.364.386-1.591 1.591M21 12h-2.25m-.386 6.364-1.591-1.591M12 18.75V21m-4.773-4.227-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0Z" },
  // This Week — calendar-days
  { key: "week", label: "This Week", icon: "M6.75 3v2.25M17.25 3v2.25M3 18.75V7.5a2.25 2.25 0 0 1 2.25-2.25h13.5A2.25 2.25 0 0 1 21 7.5v11.25m-18 0A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75m-18 0v-7.5A2.25 2.25 0 0 1 5.25 9h13.5A2.25 2.25 0 0 1 21 11.25v7.5" },
  // This Month — calendar
  { key: "month", label: "This Month", icon: "M12 6.042A8.967 8.967 0 0 0 6 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 0 1 6 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 0 1 6-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0 0 18 18a8.967 8.967 0 0 0-6 2.292m0-14.25v14.25" },
  // Someday — archive
  { key: "someday", label: "Someday", icon: "m20.25 7.5-.625 10.632a2.25 2.25 0 0 1-2.247 2.118H6.622a2.25 2.25 0 0 1-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125Z" },
];

export default function InboxView() {
  const items = useInboxStore((s) => s.items);
  const recentlyDone = useInboxStore((s) => s.recentlyDone);
  const loading = useInboxStore((s) => s.loading);
  const error = useInboxStore((s) => s.error);
  const expandedId = useInboxStore((s) => s.expandedId);
  const inboxPath = useInboxStore((s) => s.inboxPath);
  const toggleExpanded = useInboxStore((s) => s.toggleExpanded);
  const fetchInbox = useInboxStore((s) => s.fetchInbox);
  const dismissItem = useInboxStore((s) => s.dismissItem);
  const toggleActionItem = useInboxStore((s) => s.toggleActionItem);
  const quickAdd = useInboxStore((s) => s.quickAdd);

  const [draft, setDraft] = useState("");
  const [adding, setAdding] = useState(false);
  // Vertical accordion: exactly one horizon expanded at a time. Default = Today.
  const [focused, setFocused] = useState<Horizon>("today");

  // Derive horizon buckets from the pre-sorted items (pure selector + useMemo:
  // do NOT inline groupByHorizon in a zustand selector — fresh object each call).
  const grouped = useMemo(() => groupByHorizon(items), [items]);

  const submitQuickAdd = async () => {
    const text = draft.trim();
    if (!text || adding) return;
    setAdding(true);
    try {
      await quickAdd(text);
      setDraft("");
    } finally {
      setAdding(false);
    }
  };

  const quickAddBox = (
    <div className="shrink-0 flex items-center gap-2 px-4 pt-1 pb-2">
      <div className="flex-1 flex items-center gap-2 rounded-xl backdrop-blur-xl bg-white/10 dark:bg-white/[0.07] border border-white/20 dark:border-white/10 px-3 py-1.5 focus-within:border-white/40 transition-colors">
        <svg className="w-3.5 h-3.5 text-white/40 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
        </svg>
        <input
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submitQuickAdd();
            if (e.key === "Escape") setDraft("");
          }}
          placeholder={inboxPath ? "Quick-add an action — press Enter" : "Inbox not found"}
          disabled={!inboxPath || adding}
          className="flex-1 bg-transparent text-[12px] font-body text-white/90 placeholder:text-white/30 focus:outline-none disabled:opacity-50"
        />
        {draft.trim() && (
          <button
            onClick={submitQuickAdd}
            disabled={adding}
            className="text-[10px] font-secondary font-medium text-white/60 hover:text-white/90 disabled:opacity-50 transition-colors"
          >
            {adding ? "Adding…" : "Add"}
          </button>
        )}
      </div>
    </div>
  );

  if (loading && items.length === 0) {
    return (
      <div className="flex flex-col h-full">
        {quickAddBox}
        <div className="flex items-center justify-center flex-1">
          <span className="text-sm font-body text-white/40">Loading actions...</span>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 px-8">
        <span className="text-sm font-body text-white/40">Could not load actions</span>
        <span className="text-[10px] font-secondary text-white/25 max-w-sm text-center">{error}</span>
        <button onClick={fetchInbox} className="mt-2 px-4 py-1.5 bg-white/10 border border-white/15 rounded-lg text-xs font-body text-white/70 hover:bg-white/20 transition-colors">
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Quick-add */}
      {quickAddBox}

      {items.length === 0 ? (
        <div className="flex items-center justify-center flex-1">
          <span className="text-sm font-body text-white/30">No actions pending — quick-add one above</span>
        </div>
      ) : (
        /* Vertical accordion deck: 4 horizons stacked; one expanded (fills),
           three collapsed to thin bars. flex-grow animates the height. */
        <div className="flex-1 flex flex-col gap-2 p-3 pt-1 min-h-0">
          {COLUMNS.map((col) => {
            const isFocused = col.key === focused;
            const colItems = grouped[col.key];
            return (
              <div
                key={col.key}
                style={{ flexGrow: isFocused ? 1 : 0 }}
                className="flex flex-col min-h-[2.75rem] overflow-hidden rounded-2xl backdrop-blur-xl bg-white/10 dark:bg-white/[0.07] border border-white/20 dark:border-white/10 shadow-lg transition-[flex-grow] duration-300 ease-out"
              >
                {/* Header bar — always visible; click to focus this horizon */}
                <button
                  onClick={() => setFocused(col.key)}
                  className={`shrink-0 flex items-center gap-2.5 px-4 h-11 text-left transition-colors ${
                    isFocused ? "cursor-default" : "hover:bg-white/[0.06]"
                  }`}
                >
                  <svg className="w-4 h-4 text-white/70 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                    <path strokeLinecap="round" strokeLinejoin="round" d={col.icon} />
                  </svg>
                  <span className={`font-heading font-semibold flex-1 ${isFocused ? "text-[13px] text-white/90" : "text-[12px] text-white/70"}`}>
                    {col.label}
                  </span>
                  <span className="text-[11px] font-secondary text-white/40 tabular-nums">{colItems.length}</span>
                  <svg
                    className={`w-3.5 h-3.5 text-white/30 shrink-0 transition-transform duration-300 ${isFocused ? "rotate-180" : ""}`}
                    fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" d="m19.5 8.25-7.5 7.5-7.5-7.5" />
                  </svg>
                </button>

                {/* Items — only rendered for the focused horizon */}
                {isFocused && (
                  <div className="flex-1 min-h-0 overflow-y-auto px-2.5 pb-2.5 pt-0.5 space-y-1.5 border-t border-white/10">
                    {colItems.length === 0 ? (
                      <div className="flex items-center justify-center py-10">
                        <span className="text-[11px] font-secondary text-white/30">Nothing in {col.label.toLowerCase()}</span>
                      </div>
                    ) : (
                      colItems.map((item) => (
                        <InboxItem
                          key={item.id}
                          item={item}
                          isExpanded={expandedId === item.id}
                          onToggle={() => toggleExpanded(item.id)}
                          onDismiss={() => dismissItem(item.id)}
                          onToggleAction={(idx) => toggleActionItem(item.id, idx)}
                        />
                      ))
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Recently completed (this session) */}
      {recentlyDone.length > 0 && (
        <div className="shrink-0 mx-3 mb-3 max-h-24 overflow-y-auto rounded-2xl backdrop-blur-xl bg-white/10 dark:bg-white/[0.07] border border-white/20 dark:border-white/10 p-3">
          <span className="text-[10px] font-secondary text-white/40 uppercase tracking-wider">Completed</span>
          {recentlyDone.map((item) => (
            <div key={item.id} className="flex items-center gap-2 px-2 py-1.5">
              <svg className="w-3 h-3 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
              </svg>
              <span className="text-[11px] font-body text-white/40 line-through truncate">{item.subject}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
