import { useEffect, useState } from "react";
import { useNavStore, type MissionControlTab } from "../store/nav";
import { useInboxStore } from "../store/inbox";
import { useSettingsStore } from "../store/settings";
import InboxView from "./inbox/InboxView";
import inboxBg from "../assets/brand/inbox-bg.jpg";

const TABS: { key: MissionControlTab; label: string; icon: string }[] = [
  {
    key: "actions",
    label: "Actions",
    icon: "M9 12h3.75M9 15h3.75M9 18h3.75m3 .75H18a2.25 2.25 0 0 0 2.25-2.25V6.108c0-1.135-.845-2.098-1.976-2.192a48.424 48.424 0 0 0-1.123-.08m-5.801 0c-.065.21-.1.433-.1.664 0 .414.336.75.75.75h4.5a.75.75 0 0 0 .75-.75 2.25 2.25 0 0 0-.1-.664m-5.8 0A2.251 2.251 0 0 1 13.5 2.25H15c1.012 0 1.867.668 2.15 1.586m-5.8 0c-.376.023-.75.05-1.124.08C9.095 4.01 8.25 4.973 8.25 6.108V8.25m0 0H4.875c-.621 0-1.125.504-1.125 1.125v11.25c0 .621.504 1.125 1.125 1.125h9.75c.621 0 1.125-.504 1.125-1.125V9.375c0-.621-.504-1.125-1.125-1.125H8.25ZM6.75 12h.008v.008H6.75V12Zm0 3h.008v.008H6.75V15Zm0 3h.008v.008H6.75V18Z",
  },
  {
    key: "calendar",
    label: "Calendar",
    icon: "M6.75 3v2.25M17.25 3v2.25M3 18.75V7.5a2.25 2.25 0 0 1 2.25-2.25h13.5A2.25 2.25 0 0 1 21 7.5v11.25m-18 0A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75m-18 0v-7.5A2.25 2.25 0 0 1 5.25 9h13.5A2.25 2.25 0 0 1 21 11.25v7.5",
  },
  {
    key: "tools",
    label: "Tools",
    icon: "M11.42 15.17 17.25 21A2.652 2.652 0 0 0 21 17.25l-5.877-5.877M11.42 15.17l2.496-3.03c.317-.384.74-.626 1.208-.766M11.42 15.17l-4.655 5.653a2.548 2.548 0 1 1-3.586-3.586l5.653-4.655m5.976-.579a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Z",
  },
];

export default function MissionControl() {
  const activeTab = useNavStore((s) => s.missionControlTab);
  const setTab = useNavStore((s) => s.setMissionControlTab);
  const inboxPath = useInboxStore((s) => s.inboxPath);
  const setInboxPath = useInboxStore((s) => s.setInboxPath);
  const fetchInbox = useInboxStore((s) => s.fetchInbox);
  const inboxPathSetting = useSettingsStore((s) => s.settings.inbox_path);

  // Resolve inbox path on mount
  useEffect(() => {
    if (inboxPath) return;
    const tryPaths = async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      const candidates = [
        inboxPathSetting,
        `/Users/stephen/Git/stephen-work/inbox`,
      ].filter(Boolean);
      for (const path of candidates) {
        try {
          const files: { name: string }[] = await invoke("list_directory", { path });
          if (files.some((f) => f.name.endsWith(".md"))) {
            setInboxPath(path);
            return;
          }
        } catch { /* next */ }
      }
    };
    tryPaths();
  }, [inboxPath, setInboxPath, inboxPathSetting]);

  // Fetch inbox once path resolves
  useEffect(() => {
    if (inboxPath) fetchInbox();
  }, [inboxPath, fetchInbox]);

  return (
    <div className="relative flex flex-col h-full overflow-hidden">
      {/* Full-bleed background */}
      <img src={inboxBg} alt="" className="absolute inset-0 w-full h-full object-cover pointer-events-none" />
      <div className="absolute inset-0 bg-black/40 dark:bg-black/50 pointer-events-none" />

      {/* Title + Tab bar */}
      <div className="relative shrink-0 px-5 pt-4 pb-0">
        <h1 className="text-lg font-heading font-semibold text-white drop-shadow-md mb-2">Mission Control</h1>
      </div>
      <div className="relative shrink-0 flex items-center gap-1 px-4 pb-0">
        {TABS.map((tab) => (
          <button
            key={tab.key}
            onClick={() => setTab(tab.key)}
            className={`flex items-center gap-1.5 px-3 py-2 rounded-t-lg text-[11px] font-secondary font-medium transition-all ${
              activeTab === tab.key
                ? "bg-white/15 text-white border-b-2 border-white/40 backdrop-blur-md"
                : "text-white/40 hover:text-white/70 hover:bg-white/5"
            }`}
          >
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d={tab.icon} />
            </svg>
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="relative flex-1 min-h-0">
        {activeTab === "actions" && <InboxView />}
        {activeTab === "calendar" && <CalendarPlaceholder />}
        {activeTab === "tools" && <ToolsTab />}
      </div>
    </div>
  );
}

function CalendarPlaceholder() {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-3">
      <svg className="w-12 h-12 text-white/20" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M6.75 3v2.25M17.25 3v2.25M3 18.75V7.5a2.25 2.25 0 0 1 2.25-2.25h13.5A2.25 2.25 0 0 1 21 7.5v11.25m-18 0A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75m-18 0v-7.5A2.25 2.25 0 0 1 5.25 9h13.5A2.25 2.25 0 0 1 21 11.25v7.5" />
      </svg>
      <span className="text-sm font-body text-white/30">Calendar coming soon</span>
      <span className="text-[10px] font-secondary text-white/20">Google Calendar integration in progress</span>
    </div>
  );
}

interface ToolDef {
  id: string;
  name: string;
  description: string;
  icon: string;
  gradient: string;
  type: "external" | "internal";
  url?: string;
  badge?: string;
}

const TOOLS: ToolDef[] = [
  {
    id: "uplink",
    name: "Nexus Uplink",
    description: "View files uploaded by customers",
    icon: "M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75V16.5m-13.5-9L12 3m0 0 4.5 4.5M12 3v13.5",
    gradient: "from-cyan-500/30 to-blue-500/30",
    type: "external",
    url: "https://uplink.example.com/login",
  },
  {
    id: "meridian",
    name: "Nexus Meridian",
    description: "CRM — manage clients and deals",
    icon: "M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 0 1 3 12c0-1.605.42-3.113 1.157-4.418",
    gradient: "from-blue-500/30 to-purple-500/30",
    type: "external",
    url: "https://meridian.example.com",
  },
  {
    id: "priorities",
    name: "Priorities",
    description: "Org & team priority setting",
    icon: "M3 13.125C3 12.504 3.504 12 4.125 12h2.25c.621 0 1.125.504 1.125 1.125v6.75C7.5 20.496 6.996 21 6.375 21h-2.25A1.125 1.125 0 0 1 3 19.875v-6.75ZM9.75 8.625c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125v11.25c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 1.125 0 0 1-1.125-1.125V8.625ZM16.5 4.125c0-.621.504-1.125 1.125-1.125h2.25C20.496 3 21 3.504 21 4.125v15.75c0 .621-.504 1.125-1.125 1.125h-2.25a1.125 1.125 0 0 1-1.125-1.125V4.125Z",
    gradient: "from-amber-500/30 to-orange-500/30",
    type: "external",
    url: "https://priorities.example.com",
    badge: "coming soon",
  },
  {
    id: "dispatch",
    name: "Dispatch",
    description: "Issue and track team tasks",
    icon: "M6 12 3.269 3.125A59.769 59.769 0 0 1 21.485 12 59.768 59.768 0 0 1 3.27 20.875L5.999 12Zm0 0h7.5",
    gradient: "from-green-500/30 to-emerald-500/30",
    type: "external",
    url: "https://github.com/SkyNexus-AI/dispatch",
  },
  {
    id: "knowledge",
    name: "Nexus Knowledge",
    description: "Semantic search across all repos",
    icon: "M12 6.042A8.967 8.967 0 0 0 6 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 0 1 6 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 0 1 6-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0 0 18 18a8.967 8.967 0 0 0-6 2.292m0-14.25v14.25",
    gradient: "from-violet-500/30 to-indigo-500/30",
    type: "internal",
    badge: "prototype",
  },
];

function ToolsTab() {
  const [activeTool, setActiveTool] = useState<string | null>(null);

  if (activeTool) {
    const tool = TOOLS.find((t) => t.id === activeTool);
    return (
      <div className="flex flex-col h-full">
        {/* Tool header with back button */}
        <div className="shrink-0 flex items-center gap-3 px-4 py-2.5 border-b border-white/10">
          <button
            onClick={() => setActiveTool(null)}
            className="text-white/50 hover:text-white/80 transition-colors"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10.5 19.5 3 12m0 0 7.5-7.5M3 12h18" />
            </svg>
          </button>
          <span className="text-[13px] font-heading font-semibold text-white/90">{tool?.name}</span>
        </div>
        {/* Tool content */}
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <svg className="w-12 h-12 text-white/15 mx-auto mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
              <path strokeLinecap="round" strokeLinejoin="round" d={tool?.icon || ""} />
            </svg>
            <span className="text-sm font-body text-white/30">{tool?.name} integration coming soon</span>
            <p className="text-[10px] font-secondary text-white/20 mt-1 max-w-xs mx-auto">{tool?.description}</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="p-4">
      <div className="grid grid-cols-3 gap-3">
        {TOOLS.map((tool) => (
          <button
            key={tool.id}
            onClick={() => {
              if (tool.type === "external" && tool.url) {
                window.open(tool.url, "_blank");
              } else {
                setActiveTool(tool.id);
              }
            }}
            className="group relative flex flex-col items-center gap-3 p-5 rounded-2xl backdrop-blur-xl bg-white/[0.07] border border-white/15 hover:bg-white/[0.12] hover:border-white/25 transition-all text-center"
          >
            {/* Badge */}
            {tool.badge && (
              <span className="absolute top-2 right-2 text-[8px] font-secondary font-medium text-white/30 bg-white/[0.08] px-1.5 py-0.5 rounded-full">
                {tool.badge}
              </span>
            )}

            {/* External indicator */}
            {tool.type === "external" && (
              <svg className="absolute top-2.5 left-2.5 w-2.5 h-2.5 text-white/15" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M13.5 6H5.25A2.25 2.25 0 0 0 3 8.25v10.5A2.25 2.25 0 0 0 5.25 21h10.5A2.25 2.25 0 0 0 18 18.75V10.5m-10.5 6L21 3m0 0h-5.25M21 3v5.25" />
              </svg>
            )}

            {/* Icon */}
            <div className={`w-12 h-12 rounded-xl bg-gradient-to-br ${tool.gradient} flex items-center justify-center border border-white/10 group-hover:border-white/20 transition-colors`}>
              <svg className="w-6 h-6 text-white/70 group-hover:text-white/90 transition-colors" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d={tool.icon} />
              </svg>
            </div>

            {/* Name */}
            <span className="text-[12px] font-body font-medium text-white/85 group-hover:text-white transition-colors">
              {tool.name}
            </span>

            {/* Description */}
            <span className="text-[10px] font-secondary text-white/30 leading-snug">
              {tool.description}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
