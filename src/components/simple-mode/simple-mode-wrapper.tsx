/**
 * Simple Mode Wrapper (T-054)
 *
 * This component wraps the entire app content in Simple mode.
 * When appMode === "simple", it:
 * - Uses SimpleSidebar navigation
 * - Hides advanced features (regex rename, checksums, detailed connections,
 *   automation, naming inspector, scripting, logs)
 * - Shows larger touch targets and more whitespace
 * - Provides "More options" for discoverability of advanced features
 *
 * When appMode === "advanced", renders children as-is.
 * Mode switch is instant -- no restart required.
 */
import { useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { cn } from "@ufop/ui-components";
import { Button } from "@ufop/ui-components";
import {
  SimpleSidebar,
  type SimpleNavSection,
} from "./simple-sidebar";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { ActivityFeed } from "@/components/activity-feed";
import { NotificationBell } from "@/components/notification-bell";
import {
  FolderOpen,
  PanelLeft,
  HelpCircle,
} from "lucide-react";

interface SimpleModeWrapperProps {
  /** The full file manager component (rendered when section = "files" or in advanced mode) */
  fileManager: React.ReactNode;
  /** Transfer panel component */
  transferPanel?: React.ReactNode;
  /** Sync panel component */
  syncPanel?: React.ReactNode;
  /** Connection panel (Cloud & Servers) */
  connectionPanel?: React.ReactNode;
  /** Search view */
  searchView?: React.ReactNode;
  /** Favorites view */
  favoritesView?: React.ReactNode;
  /** Settings panel */
  settingsPanel?: React.ReactNode;
  /** Callback to open onboarding wizard */
  onOpenOnboarding?: () => void;
  /** Callback to open a guided flow */
  onOpenGuidedFlow?: (flowId: string) => void;
}

export function SimpleModeWrapper({
  fileManager,
  transferPanel,
  syncPanel,
  connectionPanel,
  searchView,
  favoritesView,
  settingsPanel,
  onOpenOnboarding,
  onOpenGuidedFlow,
}: SimpleModeWrapperProps) {
  const appMode = useUIStore((s) => s.appMode);
  const sidebarOpen = useUIStore((s) => s.sidebarOpen);
  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const [activeSection, setActiveSection] = useState<SimpleNavSection>("files");

  // In advanced mode, just render the file manager directly. FileManager
  // mounts its own NotificationBell, so structured errors stay visible.
  if (appMode === "advanced") {
    return <>{fileManager}</>;
  }

  const renderSection = () => {
    switch (activeSection) {
      case "files":
        return fileManager;
      case "transfers":
        return transferPanel || (
          <SimplePlaceholder
            title="Transfers"
            description="Send files between locations. Drag files here or use the guided wizard to start a transfer."
            actionLabel="Start a Transfer"
            onAction={() => onOpenGuidedFlow?.("start-transfer")}
          />
        );
      case "sync":
        return syncPanel || (
          <SimplePlaceholder
            title="Sync"
            description="Keep two folders in sync automatically. Changes in one folder appear in the other."
            actionLabel="Set Up Sync"
            onAction={() => onOpenGuidedFlow?.("create-sync")}
          />
        );
      case "cloud":
        return connectionPanel || (
          <SimplePlaceholder
            title="Cloud & Servers"
            description="Connect to OneDrive, Google Drive, Dropbox, or remote servers to manage your files in one place."
            actionLabel="Connect a Service"
            onAction={() => onOpenGuidedFlow?.("connect-onedrive")}
          />
        );
      case "search":
        return searchView || (
          <SimplePlaceholder
            title="Search"
            description="Find files and folders across all your connected locations."
            actionLabel="Search Files"
            onAction={() => {}}
          />
        );
      case "favorites":
        return favoritesView || (
          <SimplePlaceholder
            title="Favorites"
            description="Your saved locations appear here. Drag any folder to add it to favorites."
            actionLabel="Browse Files"
            onAction={() => setActiveSection("files")}
          />
        );
      case "activity":
        return <ActivityFeed />;
      case "settings":
        return settingsPanel || (
          <SimpleSettingsView onOpenOnboarding={onOpenOnboarding} />
        );
      default:
        return fileManager;
    }
  };

  return (
    <div className="flex h-full flex-col" data-testid="simple-mode-wrapper">
      {/* Simple mode toolbar */}
      <header
        className="flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-toolbar-bg)] px-4"
        style={{ height: "var(--toolbar-height)" }}
        role="toolbar"
        aria-label="Main toolbar"
      >
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={toggleSidebar}
            aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"}
            title="Toggle sidebar"
          >
            <PanelLeft className="h-4 w-4" aria-hidden="true" />
          </Button>
          <FolderOpen className="h-5 w-5 text-[color:var(--color-primary)]" aria-hidden="true" />
          <h1 className="text-[length:var(--font-size-md)] font-semibold text-[color:var(--color-text)]">
            File Manager
          </h1>
        </div>

        <div className="flex items-center gap-2">
          {/* Structured-error notifications. Shared with Advanced mode, which
              mounts the same component in the FileManager toolbar. */}
          <NotificationBell />

          <Button
            variant="ghost"
            size="icon"
            onClick={onOpenOnboarding}
            aria-label="Help and getting started"
            title="Help"
          >
            <HelpCircle className="h-4 w-4" aria-hidden="true" />
          </Button>
          <ThemeSwitcher />
        </div>
      </header>

      {/* Main content area */}
      <div className="flex flex-1 overflow-hidden">
        {/* Simple sidebar */}
        {sidebarOpen && (
          <aside
            className="border-r border-[var(--color-border)] shrink-0 overflow-hidden"
            style={{ width: sidebarWidth }}
            role="navigation"
            aria-label="Main navigation"
          >
            <SimpleSidebar
              activeSection={activeSection}
              onSectionChange={setActiveSection}
            />
          </aside>
        )}

        {/* Content area */}
        <main className="flex-1 overflow-hidden">
          {renderSection()}
        </main>
      </div>

      {/* Simple status bar */}
      <footer
        className="flex items-center justify-between border-t border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-4"
        style={{ height: "var(--status-bar-height)" }}
        role="contentinfo"
        aria-label="Status bar"
      >
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          Ready
        </span>
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          Simple Mode
        </span>
      </footer>
    </div>
  );
}

// ──────────────────────────────────────────────
// Placeholder for sections not yet wired up
// ──────────────────────────────────────────────

function SimplePlaceholder({
  title,
  description,
  actionLabel,
  onAction,
}: {
  title: string;
  description: string;
  actionLabel: string;
  onAction: () => void;
}) {
  return (
    <div className="flex flex-col items-center justify-center h-full p-8 text-center">
      <h2 className="text-xl font-semibold text-[color:var(--color-text)] mb-3">
        {title}
      </h2>
      <p className="text-sm text-[color:var(--color-text-secondary)] max-w-md mb-6 leading-relaxed">
        {description}
      </p>
      <Button variant="default" onClick={onAction}>
        {actionLabel}
      </Button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Simple Settings View (T-054)
// ──────────────────────────────────────────────

function SimpleSettingsView({
  onOpenOnboarding,
}: {
  onOpenOnboarding?: () => void;
}) {
  const appMode = useUIStore((s) => s.appMode);
  const setAppMode = useUIStore((s) => s.setAppMode);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);

  return (
    <div className="max-w-lg mx-auto p-8">
      <h2 className="text-xl font-semibold text-[color:var(--color-text)] mb-6">
        Settings
      </h2>

      <div className="space-y-6">
        {/* User Mode */}
        <div className="space-y-2">
          <label className="block text-sm font-medium text-[color:var(--color-text)]">
            User Mode
          </label>
          <p className="text-xs text-[color:var(--color-text-secondary)]">
            Simple mode shows a clean interface with essential features.
            Advanced mode unlocks regex renaming, checksums, scripting, and more.
          </p>
          <div className="flex gap-3 mt-2">
            <button
              className={cn(
                "flex-1 px-4 py-3 rounded-lg border-2 text-center transition-all",
                appMode === "simple"
                  ? "border-[var(--color-primary)] bg-[var(--color-primary)]/10 text-[color:var(--color-primary)]"
                  : "border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:border-[var(--color-text-tertiary)]",
              )}
              onClick={() => setAppMode("simple")}
            >
              <span className="text-sm font-medium block">Simple</span>
              <span className="text-xs opacity-80 block mt-1">
                Clean and easy
              </span>
            </button>
            <button
              className={cn(
                "flex-1 px-4 py-3 rounded-lg border-2 text-center transition-all",
                appMode === "advanced"
                  ? "border-[var(--color-primary)] bg-[var(--color-primary)]/10 text-[color:var(--color-primary)]"
                  : "border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:border-[var(--color-text-tertiary)]",
              )}
              onClick={() => setAppMode("advanced")}
            >
              <span className="text-sm font-medium block">Advanced</span>
              <span className="text-xs opacity-80 block mt-1">
                All features
              </span>
            </button>
          </div>
        </div>

        {/* Theme */}
        <div className="space-y-2">
          <label className="block text-sm font-medium text-[color:var(--color-text)]">
            Appearance
          </label>
          <div className="flex gap-2">
            {(["system", "light", "dark", "high-contrast"] as const).map((t) => (
              <button
                key={t}
                className={cn(
                  "px-3 py-2 rounded-lg border text-xs transition-all capitalize",
                  theme === t
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/10 text-[color:var(--color-primary)]"
                    : "border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setTheme(t)}
              >
                {t === "high-contrast" ? "High Contrast" : t}
              </button>
            ))}
          </div>
        </div>

        {/* Getting Started */}
        {onOpenOnboarding && (
          <div className="space-y-2">
            <label className="block text-sm font-medium text-[color:var(--color-text)]">
              Getting Started
            </label>
            <button
              className="text-sm text-[color:var(--color-primary)] hover:underline"
              onClick={onOpenOnboarding}
            >
              Re-run the setup wizard
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
