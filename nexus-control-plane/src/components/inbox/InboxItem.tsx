import type { InboxItem as InboxItemType } from "../../store/inbox";

const priorityConfig: Record<string, { color: string; bg: string; label: string }> = {
  P0: { color: "#f87171", bg: "rgba(248,113,113,0.15)", label: "P0" },
  P1: { color: "#fbbf24", bg: "rgba(251,191,36,0.15)", label: "P1" },
  P2: { color: "#60a5fa", bg: "rgba(96,165,250,0.15)", label: "P2" },
  P3: { color: "rgba(255,255,255,0.4)", bg: "rgba(255,255,255,0.08)", label: "P3" },
};

function formatDueDate(due: string): string {
  if (!due) return "";
  const today = new Date();
  const dueDate = new Date(due + "T00:00:00");
  const diffDays = Math.ceil((dueDate.getTime() - today.getTime()) / 86400000);

  if (diffDays < 0) return "overdue";
  if (diffDays === 0) return "today";
  if (diffDays === 1) return "tomorrow";
  if (diffDays <= 7) return dueDate.toLocaleDateString("en-US", { weekday: "short" });
  return due.slice(5);
}

interface Props {
  item: InboxItemType;
  isExpanded: boolean;
  onToggle: () => void;
  onDismiss: () => void;
  onToggleAction: (index: number) => void;
}

export default function InboxItem({ item, isExpanded, onToggle, onDismiss, onToggleAction }: Props) {
  const priority = priorityConfig[item.priority] || priorityConfig.P3;
  const dueDateStr = formatDueDate(item.dueDate);
  const isOverdue = dueDateStr === "overdue";
  const isDueToday = dueDateStr === "today";

  const totalActions = item.actionItems.length;
  const checkedActions = item.actionItems.filter((a) => a.checked).length;
  const allDone = totalActions > 0 && checkedActions === totalActions;

  return (
    <div
      className={`rounded-xl transition-all ${
        allDone
          ? "bg-green-500/10 border border-green-400/20"
          : isExpanded
            ? "bg-white/15 dark:bg-white/10 border border-white/25 shadow-lg"
            : "bg-white/5 border border-transparent hover:bg-white/10 hover:border-white/15"
      }`}
    >
      {/* Collapsed row */}
      <button onClick={onToggle} className="w-full text-left px-3 py-2.5">
        {/* Title line */}
        <div className="flex items-start gap-2 mb-1">
          <span
            className="text-[10px] font-secondary font-medium px-1.5 py-0.5 rounded shrink-0 mt-px"
            style={{ backgroundColor: priority.bg, color: priority.color }}
          >
            {priority.label}
          </span>
          <span className={`text-xs font-body leading-snug flex-1 min-w-0 ${
            allDone ? "text-white/30 line-through" : "text-white/90"
          }`}>
            {item.subject}
          </span>
          <svg
            className={`w-3 h-3 text-white/30 shrink-0 mt-0.5 transition-transform ${
              isExpanded ? "rotate-180" : ""
            }`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="m19 9-7 7-7-7" />
          </svg>
        </div>

        {/* Tags line */}
        <div className="flex items-center gap-1.5 pl-7 flex-wrap">
          {item.workstream && (
            <span className="text-[9px] font-secondary text-white/40 bg-white/10 px-1.5 py-0.5 rounded">
              {item.workstream}
            </span>
          )}
          {item.waitingOn && (
            <span className="text-[9px] font-secondary text-amber-300/80 bg-amber-400/15 px-1.5 py-0.5 rounded">
              waiting
            </span>
          )}
          {dueDateStr && (
            <span
              className={`text-[9px] font-secondary px-1.5 py-0.5 rounded ${
                isOverdue
                  ? "text-red-300 bg-red-400/15 font-medium"
                  : isDueToday
                    ? "text-amber-300 bg-amber-400/15 font-medium"
                    : "text-white/40 bg-white/10"
              }`}
            >
              {dueDateStr}
            </span>
          )}
          {totalActions > 0 && (
            <span className={`text-[9px] font-secondary px-1.5 py-0.5 rounded ${
              allDone ? "text-green-300 bg-green-400/15 font-medium" : "text-white/40 bg-white/10"
            }`}>
              {checkedActions}/{totalActions}
            </span>
          )}
        </div>
      </button>

      {/* Expanded detail */}
      {isExpanded && (
        <div className="px-4 pb-3 space-y-3">
          {/* Meta row */}
          <div className="flex items-center gap-3 text-[10px] font-secondary text-white/40 flex-wrap">
            {item.from && <span>From: {item.from}</span>}
            <span>{item.date}</span>
            <span className="px-1.5 py-0.5 rounded bg-white/10 text-white/50">
              {item.category}
            </span>
            {item.waitingOn && (
              <span className="text-amber-300/70">Waiting on: {item.waitingOn}</span>
            )}
          </div>

          {/* Summary */}
          {item.summary && (
            <p className="text-xs font-body text-white/60 leading-relaxed">
              {item.summary}
            </p>
          )}

          {/* Action items */}
          {totalActions > 0 && (
            <div className="space-y-1">
              <span className="text-[10px] font-secondary font-medium text-white/50 uppercase tracking-wider">
                Action Items
              </span>
              {item.actionItems.map((action, i) => (
                <button
                  key={i}
                  onClick={() => onToggleAction(i)}
                  className="w-full flex items-start gap-2 text-xs font-body text-left group py-0.5"
                >
                  <span className={`mt-0.5 shrink-0 transition-colors ${
                    action.checked
                      ? "text-green-400"
                      : "text-white/30 group-hover:text-white/60"
                  }`}>
                    {action.checked ? "\u2611" : "\u2610"}
                  </span>
                  <span className={`leading-relaxed ${
                    action.checked
                      ? "text-white/25 line-through"
                      : "text-white/70"
                  }`}>
                    {action.text}
                  </span>
                </button>
              ))}
            </div>
          )}

          {/* Dismiss */}
          <div className="flex gap-2 pt-1">
            <button
              onClick={onDismiss}
              className={`px-4 py-1.5 rounded-lg text-[11px] font-body font-medium transition-colors ${
                allDone
                  ? "bg-green-500/80 text-white hover:bg-green-500"
                  : "bg-white/10 border border-white/15 text-white/70 hover:bg-white/20"
              }`}
            >
              {allDone ? "Done" : "Dismiss"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
