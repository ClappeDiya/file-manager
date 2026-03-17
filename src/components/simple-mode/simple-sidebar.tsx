/**
 * Simple Mode Sidebar (T-054)
 *
 * Streamlined navigation for Simple mode:
 * Files, Transfers, Sync, Cloud & Servers, Search, Favorites, Activity, Settings
 *
 * Larger touch targets, more whitespace, fewer controls.
 * Advanced features hidden but discoverable via "More options".
 */
import { useCallback, useState } from "react";
import { useUIStore, type AppMode } from "@/stores/ui-store";
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
  ChevronRight,
  Sparkles,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export type SimpleNavSection =
  | "files"
  | "transfers"
  | "sync"
  | "cloud"
  | "search"
  | "favorites"
  | "activity"
  | "settings";

interface SimpleSidebarProps {
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
  activeSection,
  onSectionChange,
  className,
}: SimpleSidebarProps) {
  const appMode = useUIStore((s) => s.appMode);
  const setAppMode = useUIStore((s) => s.setAppMode);
  const [showModeHint, setShowModeHint] = useState(false);

  const handleModeSwitch = useCallback(() => {
    const newMode: AppMode = appMode === "simple" ? "advanced" : "simple";
    setAppMode(newMode);
  }, [appMode, setAppMode]);

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
          {NAV_ITEMS.map((item) => (
            <button
              key={item.id}
              className={cn(
                "flex items-center gap-3 w-full px-4 py-3 rounded-lg text-left",
                "transition-all duration-150",
                "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
                activeSection === item.id
                  ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] shadow-sm"
                  : "text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)] hover:text-[color:var(--color-text)]",
              )}
              onClick={() => onSectionChange(item.id)}
              aria-current={activeSection === item.id ? "page" : undefined}
              title={item.description}
            >
              {item.icon}
              <div className="flex-1 min-w-0">
                <span className="text-sm font-medium">{item.label}</span>
              </div>
              {activeSection === item.id && (
                <ChevronRight className="h-4 w-4 opacity-60" aria-hidden="true" />
              )}
            </button>
          ))}
        </div>

        {/* Mode switcher at bottom */}
        <div className="px-2 pt-3 mt-auto border-t border-[var(--color-border)]">
          <button
            className={cn(
              "flex items-center gap-3 w-full px-4 py-3 rounded-lg text-left",
              "text-[color:var(--color-text-tertiary)] hover:bg-[var(--color-hover-bg)] hover:text-[color:var(--color-text)]",
              "transition-all duration-150",
              "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
            )}
            onClick={handleModeSwitch}
            onMouseEnter={() => setShowModeHint(true)}
            onMouseLeave={() => setShowModeHint(false)}
            aria-label={
              appMode === "simple"
                ? "Switch to Advanced mode for more features"
                : "Switch to Simple mode for a cleaner interface"
            }
          >
            <Sparkles className="h-5 w-5" aria-hidden="true" />
            <div className="flex-1 min-w-0">
              <span className="text-sm font-medium">
                {appMode === "simple" ? "More Options" : "Simple Mode"}
              </span>
              {showModeHint && (
                <p className="text-xs text-[color:var(--color-text-tertiary)] mt-0.5">
                  {appMode === "simple"
                    ? "Enable advanced features like regex, checksums, and scripting"
                    : "Switch to a cleaner interface with fewer controls"}
                </p>
              )}
            </div>
          </button>
        </div>
      </nav>
    </ScrollArea>
  );
}
