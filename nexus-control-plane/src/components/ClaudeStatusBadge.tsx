interface ClaudeStatusBadgeProps {
  state: string;
  tool?: string | null;
  agentCount?: number;
  compact?: boolean;
}

const STATE_CONFIG: Record<string, { dot: string; label: string; pulse?: boolean }> = {
  idle: { dot: "bg-gray-400", label: "Idle" },
  thinking: { dot: "bg-amber-400", label: "Thinking", pulse: true },
  working: { dot: "bg-blue-400", label: "Working" },
  running_command: { dot: "bg-green-400", label: "Running", pulse: true },
  reading_code: { dot: "bg-sky-400", label: "Reading" },
  writing_code: { dot: "bg-indigo-400", label: "Writing" },
  spawning_agents: { dot: "bg-purple-400", label: "Agents", pulse: true },
  waiting_for_approval: { dot: "bg-orange-400", label: "Waiting", pulse: true },
  operator_active: { dot: "bg-white", label: "Active" },
};

export default function ClaudeStatusBadge({
  state,
  tool,
  agentCount,
  compact = false,
}: ClaudeStatusBadgeProps) {
  const config = STATE_CONFIG[state] ?? STATE_CONFIG["idle"];

  const dotClass = `inline-block w-1.5 h-1.5 rounded-full shrink-0 ${config.dot}${config.pulse ? " animate-pulse-soft" : ""}`;

  if (compact) {
    return <span className={dotClass} title={config.label} />;
  }

  let label = config.label;
  if (state === "working" && tool) {
    label = tool;
  } else if (state === "spawning_agents" && agentCount && agentCount > 1) {
    label = `${agentCount} agents`;
  }

  return (
    <span className="inline-flex items-center gap-1">
      <span className={dotClass} />
      <span className="text-[10px] font-medium">{label}</span>
    </span>
  );
}
