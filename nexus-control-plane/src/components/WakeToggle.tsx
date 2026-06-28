import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface WakeStatus {
  enabled: boolean;
  poll_interval_secs: number;
  last_poll: string | null;
  next_poll: string | null;
  queued_agent_total: number;
  queued_person_total: number;
  consecutive_errors: number;
  operator_branch: string | null;
  backoff_active: boolean;
}

export default function WakeToggle() {
  const [status, setStatus] = useState<WakeStatus | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const s = await invoke<WakeStatus>("wake_status");
      setStatus(s);
    } catch {
      // wake commands not available — relay not wired
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 5_000);
    return () => clearInterval(id);
  }, [refresh]);

  const toggle = async () => {
    setLoading(true);
    try {
      if (status?.enabled) {
        const s = await invoke<WakeStatus>("wake_disable");
        setStatus(s);
      } else {
        const s = await invoke<WakeStatus>("wake_enable");
        setStatus(s);
      }
    } catch (e) {
      console.error("[wake] toggle failed:", e);
    } finally {
      setLoading(false);
    }
  };

  if (!status) return null;

  const totalQueued = status.queued_agent_total + status.queued_person_total;
  const hasErrors = status.consecutive_errors > 0;

  return (
    <button
      onClick={toggle}
      disabled={loading}
      title={
        status.enabled
          ? `Wake active${totalQueued > 0 ? ` (${totalQueued} queued)` : ""}${hasErrors ? ` — ${status.consecutive_errors} errors` : ""} — click to disable`
          : "Wake off — click to enable"
      }
      className={`relative p-1 rounded-lg transition-colors ${
        loading
          ? "opacity-50 cursor-wait"
          : status.enabled
            ? "text-green-500 hover:text-green-400 hover:bg-nx-surface-hover"
            : "text-nx-muted hover:text-nx-text hover:bg-nx-surface-hover"
      }`}
    >
      {/* Antenna / broadcast icon */}
      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M9.348 14.652a3.75 3.75 0 0 1 0-5.304m5.304 0a3.75 3.75 0 0 1 0 5.304m-7.425 2.121a6.75 6.75 0 0 1 0-9.546m9.546 0a6.75 6.75 0 0 1 0 9.546M5.106 18.894c-3.808-3.807-3.808-9.98 0-13.788m13.788 0c3.808 3.807 3.808 9.98 0 13.788M12 12h.008v.008H12V12Zm.375 0a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Z" />
      </svg>

      {/* Badge for queued count */}
      {status.enabled && totalQueued > 0 && (
        <span className="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] flex items-center justify-center rounded-full bg-nx-accent text-white text-[8px] font-bold leading-none px-0.5">
          {totalQueued}
        </span>
      )}

      {/* Error indicator */}
      {status.enabled && hasErrors && (
        <span className="absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full bg-amber-400" />
      )}
    </button>
  );
}
