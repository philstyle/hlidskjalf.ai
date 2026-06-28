import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface UsageData {
  five_hour: {
    used_percentage: number;
    resets_at: number;
  } | null;
  seven_day: {
    used_percentage: number;
    resets_at: number;
    expected_percentage: number;
    headroom: number;
  } | null;
}

function barColor(pct: number, headroom?: number): string {
  if (headroom !== undefined && headroom < 0) return "bg-red-500";
  if (pct < 50) return "bg-green-500";
  if (pct < 75) return "bg-yellow-500";
  return "bg-orange-500";
}

export default function UsageBars() {
  const [usage, setUsage] = useState<UsageData | null>(null);

  useEffect(() => {
    const fetchUsage = async () => {
      try {
        const resp = await invoke<string>("fetch_url", {
          url: "http://localhost:4242/usage",
        });
        setUsage(JSON.parse(resp));
      } catch {
        // /usage may not exist on older NCC or fetch_url may not be a command
        // Fall back to direct fetch if available
        try {
          const r = await fetch("http://localhost:4242/usage");
          if (r.ok) setUsage(await r.json());
        } catch {
          // silently ignore
        }
      }
    };

    fetchUsage();
    const id = setInterval(fetchUsage, 60_000);
    return () => clearInterval(id);
  }, []);

  if (!usage) return null;

  const fh = usage.five_hour;
  const sd = usage.seven_day;

  if (!fh && !sd) return null;

  return (
    <div className="px-3 py-1.5 border-t border-nx-border space-y-1">
      {fh && (
        <div className="flex items-center gap-1.5">
          <span className="text-[9px] text-nx-muted w-3 text-right shrink-0">5h</span>
          <div className="flex-1 h-[3px] bg-nx-border/30 rounded-full overflow-hidden relative">
            <div
              className={`h-full rounded-full transition-all duration-500 ${barColor(fh.used_percentage)}`}
              style={{ width: `${Math.min(100, fh.used_percentage)}%` }}
            />
          </div>
          <span className="text-[9px] text-nx-muted w-6 text-right shrink-0">
            {Math.round(fh.used_percentage)}%
          </span>
        </div>
      )}
      {sd && (
        <div className="flex items-center gap-1.5">
          <span className="text-[9px] text-nx-muted w-3 text-right shrink-0">7d</span>
          <div className="flex-1 h-[3px] bg-nx-border/30 rounded-full overflow-visible relative">
            <div
              className={`h-full rounded-full transition-all duration-500 ${barColor(sd.used_percentage, sd.headroom)}`}
              style={{ width: `${Math.min(100, sd.used_percentage)}%` }}
            />
            {/* Pace marker */}
            <div
              className="absolute top-[-2px] w-px h-[7px] bg-white/30"
              style={{ left: `${Math.min(100, sd.expected_percentage)}%` }}
              title={`Pace: ${Math.round(sd.expected_percentage)}%`}
            />
          </div>
          <span className="text-[9px] text-nx-muted w-6 text-right shrink-0">
            {Math.round(sd.used_percentage)}%
          </span>
        </div>
      )}
    </div>
  );
}
