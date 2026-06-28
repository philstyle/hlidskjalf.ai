interface RelayBadgeProps {
  count: number;
}

export default function RelayBadge({ count }: RelayBadgeProps) {
  if (count <= 0) return null;
  return (
    <span
      className="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 rounded-full bg-nx-accent text-white text-[10px] font-semibold"
      title={`${count} pending relay messages`}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
