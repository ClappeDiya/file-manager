/**
 * Simple Mode Sidebar (T-054)
 *
 * Task-based navigation for both sidebar layouts:
 * Files, Transfers, Sync, Cloud & Servers, Search, Favorites, Activity, Settings
 *
 * Larger touch targets, more whitespace, fewer controls.
 * Basic keeps everyday destinations; All features always exposes every tool.
 */
import { useUIStore } from "@/stores/ui-store";
import { cn } from "@ufop/ui-components";
import { ScrollArea } from "@ufop/ui-components";
import {
  FolderOpen,
  ArrowUpDown,
  RefreshCw,
  Cloud,
  Search,
  Star,
  Activity,
  Settings,
  DatabaseBackup,
  ChevronRight,
  LayoutGrid,
  Zap,
  Shield,
  Wrench,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export type SimpleNavSection =
  | "files"
  | "transfers"
  | "backups"
  | "sync"
  | "cloud"
  | "search"
  | "favorites"
  | "activity"
  | "settings"
  | "features"
  | "automation"
  | "security"
  | "tools";

interface SimpleSidebarProps {
  compact?: boolean;
  activeSection: SimpleNavSection;
  onSectionChange: (section: SimpleNavSection) => void;
  className?: string;
}

// ──────────────────────────────────────────────
// Navigation items
// ──────────────────────────────────────────────

const NAV_ITEMS: {
  id: SimpleNavSection;
  label: string;
  icon: React.ReactNode;
  description: string;
}[] = [
  {
    id: "features",
    label: "All features",
    icon: <LayoutGrid className="h-5 w-5" aria-hidden="true" />,
    description: "Find every platform tool by task",
  },
  {
    id: "automation",
    label: "Automation & AI",
    icon: <Zap className="h-5 w-5" aria-hidden="true" />,
    description: "Quickflows, schedules, Smart Spaces and AI",
  },
  {
    id: "security",
    label: "Security & Sharing",
    icon: <Shield className="h-5 w-5" aria-hidden="true" />,
    description: "Vault, encryption, sharing and security settings",
  },
  {
    id: "tools",
    label: "Tools & Diagnostics",
    icon: <Wrench className="h-5 w-5" aria-hidden="true" />,
    description: "Terminal, file tools, logs and diagnostics",
  },
  {
    id: "files",
    label: "Files",
    icon: <FolderOpen className="h-5 w-5" aria-hidden="true" />,
    description: "Browse and manage your files",
  },
  {
    id: "transfers",
    label: "Transfers",
    icon: <ArrowUpDown className="h-5 w-5" aria-hidden="true" />,
    description: "Send and receive files",
  },
  {
    id: "backups",
    label: "Backups & Restore",
    icon: <DatabaseBackup className="h-5 w-5" aria-hidden="true" />,
    description: "Protect files and restore a version",
  },
  {
    id: "sync",
    label: "Sync",
    icon: <RefreshCw className="h-5 w-5" aria-hidden="true" />,
    description: "Keep folders in sync",
  },
  {
    id: "cloud",
    label: "Cloud & Servers",
    icon: <Cloud className="h-5 w-5" aria-hidden="true" />,
    description: "Connect to cloud storage and servers",
  },
  {
    id: "search",
    label: "Search",
    icon: <Search className="h-5 w-5" aria-hidden="true" />,
    description: "Find files and folders",
  },
  {
    id: "favorites",
    label: "Favorites",
    icon: <Star className="h-5 w-5" aria-hidden="true" />,
    description: "Quick access to saved locations",
  },
  {
    id: "activity",
    label: "Activity",
    icon: <Activity className="h-5 w-5" aria-hidden="true" />,
    description: "See what happened recently",
  },
  {
    id: "settings",
    label: "Settings",
    icon: <Settings className="h-5 w-5" aria-hidden="true" />,
    description: "Customize your experience",
  },
];

// ──────────────────────────────────────────────
// Component
// ──────────────────────────────────────────────

export function SimpleSidebar({
  compact = false,
  activeSection,
  onSectionChange,
  className,
}: SimpleSidebarProps) {
  const appMode = useUIStore((s) => s.appMode);

  return (
    <ScrollArea
      className={cn(
        "h-full bg-[var(--color-sidebar-bg)] text-[color:var(--color-sidebar-text)]",
        className,
      )}
      data-testid="simple-sidebar"
    >
      <nav className="py-3 flex flex-col h-full" aria-label="Main navigation">
        {/* Navigation items with larger touch targets */}
        <div className="flex-1 space-y-1 px-2">
          {[
            {
              label: "Workspace",
              ids: ["files", "search", "favorites", "features"],
            },
            {
              label: "File operations",
              ids: ["transfers", "backups", "sync", "cloud"],
            },
            ...(appMode === "advanced"
              ? [
                  {
                    label: "Advanced tools",
                    ids: ["automation", "security", "tools"],
                  },
                ]
              : []),
            { label: "Manage", ids: ["activity", "settings"] },
          ].map((group) => (
            <section
              key={group.label}
              aria-label={group.label}
              className="mb-3"
            >
              {!compact && (
                <h2 className="px-3 pb-1 text-xs font-medium text-[color:var(--color-text-secondary)]">
                  {group.label}
                </h2>
              )}
              {NAV_ITEMS.filter((item) => group.ids.includes(item.id))
                .sort(
                  (a, b) => group.ids.indexOf(a.id) - group.ids.indexOf(b.id),
                )
                .map((item) => (
                  <button
                    key={item.id}
                    className={cn(
                      "flex items-center gap-3 w-full py-2.5 rounded-lg text-left",
                      compact ? "px-2 justify-center" : "px-4",
                      "transition-all duration-150",
                      "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
                      activeSection === item.id
                        ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] shadow-sm"
                        : "text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)] hover:text-[color:var(--color-text)]",
                    )}
                    onClick={() => onSectionChange(item.id)}
                    aria-current={
                      activeSection === item.id ? "page" : undefined
                    }
                    title={item.description}
                  >
                    {item.icon}
                    <div className={compact ? "sr-only" : "flex-1 min-w-0"}>
                      <span className="text-sm font-medium">{item.label}</span>
                    </div>
                    {activeSection === item.id && (
                      <ChevronRight
                        className="h-4 w-4 opacity-60"
                        aria-hidden="true"
                      />
                    )}
                  </button>
                ))}
            </section>
          ))}
        </div>

        <div className="px-2 pt-3 mt-auto border-t border-[var(--color-border)]">
          {!compact && (
            <p className="px-3 text-xs text-[color:var(--color-text-secondary)]">
              {appMode === "simple" ? "Basic" : "Advanced"} sidebar
            </p>
          )}
          <button
            type="button"
            className="flex gap-3 items-center w-full rounded-lg px-3 py-2.5 text-sm hover:bg-[var(--color-hover-bg)] focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
            onClick={() => onSectionChange("settings")}
            aria-label="Customize sidebar"
          >
            <Settings className="h-4 w-4" aria-hidden="true" />
            <span className={compact ? "sr-only" : undefined}>
              Customize sidebar
            </span>
          </button>
        </div>
      </nav>
    </ScrollArea>
  );
}
