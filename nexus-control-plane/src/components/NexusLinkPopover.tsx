import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface NexusLinkStatus {
  running: boolean;
  bind_address: string;
  tailscale_ip: string | null;
  tailscale_error: string | null;
  qr_svg: string | null;
  paired_device_count: number;
}

interface NexusLinkPopoverProps {
  onClose: () => void;
}

export default function NexusLinkPopover({ onClose }: NexusLinkPopoverProps) {
  const [status, setStatus] = useState<NexusLinkStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<NexusLinkStatus>("get_nexuslink_status")
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, []);

  // Click outside to dismiss
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onClose();
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="absolute left-0 bottom-full mb-2 z-50 w-72 bg-nx-surface border border-nx-border rounded-xl shadow-nx-xl p-4"
    >
      <div className="text-sm font-semibold text-nx-text mb-3">NexusLink</div>

      {error && (
        <div className="text-xs text-red-400 mb-2">Error: {error}</div>
      )}

      {!status && !error && (
        <div className="text-xs text-nx-muted">Loading...</div>
      )}

      {status && (
        <>
          {/* Status */}
          <div className="flex items-center gap-2 mb-3">
            <div
              className={`w-2 h-2 rounded-full ${
                status.running ? "bg-green-400" : "bg-nx-muted"
              }`}
            />
            <span className="text-xs text-nx-muted">
              {status.running ? "Running" : "Stopped"} &middot;{" "}
              {status.bind_address}
            </span>
          </div>

          {/* QR Code */}
          {status.qr_svg ? (
            <div
              className="flex justify-center mb-3"
              dangerouslySetInnerHTML={{ __html: status.qr_svg }}
              style={{ width: 200, height: 200, margin: "0 auto" }}
            />
          ) : (
            <div className="text-xs text-nx-muted mb-3 text-center py-8 bg-nx-bg rounded">
              {status.tailscale_error
                ? `Tailscale: ${status.tailscale_error}`
                : "No Tailscale connection — QR unavailable"}
            </div>
          )}

          {/* Paired devices */}
          <div className="text-xs text-nx-muted mb-2">
            {status.paired_device_count} paired device
            {status.paired_device_count !== 1 ? "s" : ""}
          </div>

          {/* Tailscale mismatch warning */}
          {status.tailscale_ip && !status.bind_address.startsWith(status.tailscale_ip) && (
            <div className="text-xs text-yellow-400 mt-1">
              Tailscale IP changed since launch. Restart app to update server bind address.
            </div>
          )}
        </>
      )}
    </div>
  );
}
