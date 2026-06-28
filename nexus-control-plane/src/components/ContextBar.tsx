import { useSessionsStore } from "../store/sessions";

interface ContextBarProps {
  cardId: string;
}

function contextBarStyle(percent: number): React.CSSProperties {
  // Smooth gradient: green → yellow → orange → red
  // Interpolate HSL hue: 142 (green) → 45 (yellow) → 25 (orange) → 0 (red)
  let hue: number;
  let saturation = 72;
  let lightness = 45;
  if (percent <= 63) {
    hue = 142 - (percent / 63) * (142 - 45);
  } else if (percent <= 81) {
    const t = (percent - 63) / (81 - 63);
    hue = 45 - t * (45 - 25);
  } else {
    const t = Math.min(1, (percent - 81) / (100 - 81));
    hue = 25 - t * 25;
    saturation = 72 + t * 8;
    lightness = 45 + t * 5;
  }
  return {
    width: `${percent}%`,
    backgroundColor: `hsl(${hue}, ${saturation}%, ${lightness}%)`,
    animation: percent >= 95 ? "pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite" : undefined,
  };
}

function permBadge(mode?: string): { label: string; style: string } {
  switch (mode) {
    case "bypassPermissions":
      return { label: "Bypass", style: "bg-red-50 text-red-600" };
    case "plan":
      return { label: "Planning", style: "bg-amber-50 text-amber-700" };
    case "acceptEdits":
      return { label: "Accept Edits", style: "bg-nx-accent-light text-nx-accent" };
    default:
      return { label: "Confirm", style: "bg-nx-bg-alt text-nx-text-secondary" };
  }
}

export default function ContextBar({ cardId }: ContextBarProps) {
  const session = useSessionsStore((s) => s.sessions[cardId]);

  const contextPercent = session?.contextPercent ?? 0;
  const modelName = session?.model ?? "Claude";
  const perm = permBadge(session?.permissionMode);
  const cost = session?.cost ?? 0;

  return (
    <div className="flex items-center gap-4 px-4 py-2 border-t border-nx-border-light bg-nx-surface shrink-0">
      {/* Context progress bar */}
      <div className="flex items-center gap-2 flex-1">
        <span className="text-[11px] text-nx-text-secondary font-secondary font-medium">
          Context
        </span>
        <div className="flex-1 max-w-[200px] h-1.5 bg-nx-bg-alt rounded-full overflow-hidden">
          <div
            className="h-full rounded-full transition-all duration-500"
            style={contextBarStyle(contextPercent)}
          />
        </div>
        <span className="text-[11px] font-secondary text-nx-muted">
          {contextPercent}%
        </span>
      </div>

      {/* Model badge */}
      <div className="flex items-center gap-1.5 px-2 py-0.5 bg-nx-bg-alt rounded-full">
        <div className="w-1.5 h-1.5 rounded-full bg-nx-accent" />
        <span className="text-[11px] font-secondary text-nx-text-secondary font-medium">
          {modelName}
        </span>
      </div>

      {/* Permission mode badge */}
      <div className={`flex items-center gap-1.5 px-2 py-0.5 rounded-full ${perm.style}`}>
        <span className="text-[11px] font-secondary font-medium">{perm.label}</span>
      </div>

      {/* Cost */}
      {cost > 0 && (
        <span className="text-[11px] font-secondary text-nx-muted">
          ${cost.toFixed(3)}
        </span>
      )}
    </div>
  );
}
