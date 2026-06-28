import { useState, useMemo } from "react";

export interface RepoSummary {
  name: string;
  full_name: string;
  description: string | null;
  default_branch: string;
  updated_at: string;
  is_private: boolean;
}

interface RepoPickerProps {
  repos: RepoSummary[];
  loading: boolean;
  selected: RepoSummary | null;
  onSelect: (repo: RepoSummary) => void;
  org: string;
}

function relativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  return `${months}mo ago`;
}

export default function RepoPicker({
  repos,
  loading,
  selected,
  onSelect,
  org,
}: RepoPickerProps) {
  const [search, setSearch] = useState("");

  const filtered = useMemo(() => {
    if (!search.trim()) return repos;
    const q = search.toLowerCase();
    return repos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        (r.description && r.description.toLowerCase().includes(q))
    );
  }, [repos, search]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="w-5 h-5 border-2 border-nx-accent border-t-transparent rounded-full animate-spin" />
        <span className="ml-2 text-xs text-nx-muted">Loading repos...</span>
      </div>
    );
  }

  return (
    <div>
      <input
        type="text"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder="Search repos..."
        className="w-full px-3 py-1.5 text-sm bg-nx-bg border border-nx-border rounded text-nx-text
          placeholder:text-nx-dim focus:outline-none focus:border-nx-accent/50 mb-2"
      />

      <div className="max-h-64 overflow-y-auto border border-nx-border-light rounded">
        {filtered.length === 0 && repos.length === 0 && (
          <p className="text-xs text-nx-muted text-center py-6">
            No repositories found in {org}
          </p>
        )}
        {filtered.length === 0 && repos.length > 0 && (
          <p className="text-xs text-nx-muted text-center py-6">
            No repos match
          </p>
        )}
        {filtered.map((repo) => {
          const isSelected = selected?.full_name === repo.full_name;
          return (
            <button
              key={repo.full_name}
              onClick={() => onSelect(repo)}
              className={`w-full text-left px-3 py-2 transition-colors border-l-2 ${
                isSelected
                  ? "border-l-nx-accent bg-nx-accent/10"
                  : "border-l-transparent hover:bg-nx-surface-hover"
              }`}
            >
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium text-nx-text truncate">
                  {repo.name}
                </span>
                {repo.is_private && (
                  <span className="text-[9px] px-1 py-0.5 rounded bg-nx-bg text-nx-dim leading-none">
                    private
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2 mt-0.5">
                {repo.description && (
                  <span className="text-[10px] text-nx-dim truncate flex-1">
                    {repo.description}
                  </span>
                )}
                <span className="text-[10px] text-nx-dim whitespace-nowrap">
                  {relativeTime(repo.updated_at)}
                </span>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
