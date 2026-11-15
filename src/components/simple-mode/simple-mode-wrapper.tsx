import SettingsPanel from "@/components/settings-panel";
/** Shared Files-first shell. Layout changes the navigation density, never access. */
import { useEffect, useRef, useState } from "react";
import { useMediaQuery } from "@/hooks/use-media-query";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { BackupPanel } from "@/components/backup/backup-panel";
import { AllFeatures } from "./all-features";
import type { FeatureTarget } from "./feature-catalog";
import { useFeatureNavigation } from "@/stores/feature-navigation-store";
import { SimpleSearchView } from "./simple-search-view";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";
import { cn } from "@ufop/ui-components";
import { Button } from "@ufop/ui-components";
import { SimpleSidebar, type SimpleNavSection } from "./simple-sidebar";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { ActivityFeed } from "@/components/activity-feed";
import { NotificationBell } from "@/components/notification-bell";
import { FolderOpen, PanelLeft, HelpCircle, X } from "lucide-react";

interface SimpleModeWrapperProps {
  /** The full file manager component (rendered when section = "files" or in advanced mode) */
  fileManager: React.ReactNode;
  transferWorkspace?: React.ReactNode;
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
  transferWorkspace,
  transferPanel,
  syncPanel,
  connectionPanel,
  searchView,
  favoritesView,
  settingsPanel,
  onOpenOnboarding,
  onOpenGuidedFlow,
}: SimpleModeWrapperProps) {
  const sidebarOpen = useUIStore((s) => s.sidebarOpen);
  const narrow = useMediaQuery("(max-width: 639px)");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const showSidebar = narrow ? mobileNavOpen : sidebarOpen;
  const closeNavigation = () => setMobileNavOpen(false);
  const mobileNavRef = useFocusTrap<HTMLElement>({
    active: narrow && mobileNavOpen,
    onEscape: closeNavigation,
  });
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const [activeSection, setActiveSection] = useState<SimpleNavSection>("files");
  const [settingsRequest, setSettingsRequest] = useState(0);
  const contentRef = useRef<HTMLElement>(null);
  useEffect(() => {
    contentRef.current?.scrollTo?.({ top: 0, left: 0 });
    const frame = requestAnimationFrame(() => {
      if (!document.querySelector('[role="dialog"], [role="alertdialog"]'))
        contentRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [activeSection]);

  useEffect(() => {
    const show = () => setActiveSection("files");
    window.addEventListener("ufop:show-workspace", show);
    return () => window.removeEventListener("ufop:show-workspace", show);
  }, []);

  useEffect(() => {
    const openCommands = (event: KeyboardEvent) => {
      if (
        activeSection === "files" ||
        document.querySelector('[role="dialog"], [role="alertdialog"]')
      )
        return;
      if (
        (event.ctrlKey || event.metaKey) &&
        !event.shiftKey &&
        event.key === ","
      ) {
        event.preventDefault();
        setSettingsRequest((value) => value + 1);
        setActiveSection("settings");
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setActiveSection("files");
        useFileManagerStore.getState().setCommandPaletteOpen(true);
      }
    };
    window.addEventListener("keydown", openCommands);
    return () => window.removeEventListener("keydown", openCommands);
  }, [activeSection]);

  const openFeature = (target: FeatureTarget) => {
    if ("section" in target) setActiveSection(target.section);
    else {
      useFeatureNavigation.getState().request(target);
      setActiveSection("files");
    }
  };

  const renderSection = () => {
    switch (activeSection) {
      case "features":
      case "automation":
      case "security":
      case "tools":
        return (
          <AllFeatures
            key={activeSection}
            group={
              activeSection === "automation"
                ? "Automation & AI"
                : activeSection === "security"
                  ? "Security & sharing"
                  : activeSection === "tools"
                    ? "Tools & diagnostics"
                    : undefined
            }
            onOpen={openFeature}
          />
        );
      case "files":
        return fileManager;
      case "transfers":
        return (
          transferPanel || (
            <SimplePlaceholder
              title="Transfers"
              description="Send files between locations. Drag files here or use the guided wizard to start a transfer."
              actionLabel="Start a Transfer"
              onAction={() => onOpenGuidedFlow?.("start-transfer")}
            />
          )
        );
      case "backups":
        return <BackupPanel />;
      case "sync":
        return (
          syncPanel || (
            <SimplePlaceholder
              title="Sync"
              description="Keep two folders in sync automatically. Changes in one folder appear in the other."
              actionLabel="Set Up Sync"
              onAction={() => onOpenGuidedFlow?.("create-sync")}
            />
          )
        );
      case "cloud":
        return (
          connectionPanel || (
            <SimplePlaceholder
              title="Cloud & Servers"
              description="Connect to OneDrive, Google Drive, Dropbox, or remote servers to manage your files in one place."
              actionLabel="Connect a Service"
              onAction={() => onOpenGuidedFlow?.("connect-onedrive")}
            />
          )
        );
      case "search":
        return (
          searchView || (
            <SimpleSearchView
              onOpen={(path, isDir) => {
                useFileManagerStore
                  .getState()
                  .revealPath(
                    useFileManagerStore.getState().activePaneIndex,
                    path,
                    isDir,
                  );
                setActiveSection("files");
              }}
            />
          )
        );
      case "favorites":
        return (
          favoritesView || (
            <SavedLocations onOpen={() => setActiveSection("files")} />
          )
        );
      case "activity":
        return <ActivityFeed />;
      case "settings":
        return (
          settingsPanel || (
            <SimpleSettingsView
              settingsRequest={settingsRequest}
              onOpenOnboarding={onOpenOnboarding}
            />
          )
        );
      default:
        return fileManager;
    }
  };

  return (
    <div
      className="flex h-full min-h-0 flex-col"
      data-testid="simple-mode-wrapper"
    >
      {/* Simple mode toolbar */}
      <header
        className="flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-toolbar-bg)] px-4"
        style={{ height: "var(--toolbar-height)" }}
        aria-label="Application header"
      >
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() =>
              narrow ? setMobileNavOpen(!mobileNavOpen) : toggleSidebar()
            }
            aria-expanded={showSidebar}
            aria-controls="workspace-navigation"
            aria-label={showSidebar ? "Hide sidebar" : "Show sidebar"}
            title="Toggle sidebar"
          >
            <PanelLeft className="h-4 w-4" aria-hidden="true" />
          </Button>
          <FolderOpen
            className="h-5 w-5 text-[color:var(--color-primary)]"
            aria-hidden="true"
          />
          <h1 className="whitespace-nowrap text-[length:var(--font-size-md)] font-semibold text-[color:var(--color-text)]">
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
          <ThemeSwitcher compact />
        </div>
      </header>

      {/* Main content area */}
      <div className="relative flex flex-1 min-h-0 overflow-hidden">
        {/* Simple sidebar */}
        {showSidebar && (
          <aside
            ref={mobileNavRef}
            id="workspace-navigation"
            role={narrow ? "dialog" : undefined}
            aria-modal={narrow || undefined}
            className={cn(
              "border-r border-[var(--color-border)] shrink-0 overflow-hidden bg-[var(--color-sidebar-bg)]",
              narrow &&
                "absolute inset-y-0 left-0 z-40 shadow-xl flex flex-col",
            )}
            style={{ width: "clamp(160px, 19vw, 208px)" }}
            aria-label="Application sections"
          >
            {narrow && (
              <Button
                variant="ghost"
                onClick={closeNavigation}
                className="shrink-0 justify-between"
              >
                Navigation
                <X className="h-4 w-4" aria-hidden="true" />
              </Button>
            )}
            <SimpleSidebar
              className={narrow ? "flex-1 min-h-0" : undefined}
              activeSection={activeSection}
              onSectionChange={(section) => {
                setActiveSection(section);
                closeNavigation();
              }}
            />
          </aside>
        )}

        {narrow && showSidebar && (
          <div
            className="absolute inset-0 z-30 bg-[var(--color-bg-overlay)]"
            aria-hidden="true"
            onClick={closeNavigation}
          />
        )}
        {/* Content area */}
        <main
          ref={contentRef}
          id="main-content"
          tabIndex={-1}
          className="flex-1 min-w-0 overflow-auto"
        >
          {renderSection()}
        </main>
      </div>

      <div>{transferWorkspace}</div>
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
  settingsRequest = 0,
  onOpenOnboarding,
}: {
  settingsRequest?: number;
  onOpenOnboarding?: () => void;
}) {
  const [allSettings, setAllSettings] = useState(settingsRequest > 0);
  useEffect(() => {
    if (settingsRequest > 0) setAllSettings(true);
  }, [settingsRequest]);
  const fullSettingsRef = useRef<HTMLButtonElement>(null);
  const openSettingsRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (allSettings) fullSettingsRef.current?.focus();
    else openSettingsRef.current?.focus();
  }, [allSettings]);
  const appMode = useUIStore((s) => s.appMode);
  const setAppMode = useUIStore((s) => s.setAppMode);
  const [draftMode, setDraftMode] = useState(appMode);
  const [layoutNotice, setLayoutNotice] = useState("");
  useEffect(() => {
    setDraftMode(appMode);
  }, [appMode]);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);

  if (allSettings)
    return (
      <section
        className="h-full min-h-0 flex flex-col"
        aria-label="All settings"
      >
        <div className="p-3 border-b border-[var(--color-border)]">
          <Button
            ref={fullSettingsRef}
            variant="ghost"
            onClick={() => setAllSettings(false)}
          >
            Back to settings overview
          </Button>
        </div>
        <div className="flex-1 min-h-0">
          <SettingsPanel />
        </div>
      </section>
    );
  return (
    <div className="max-w-lg mx-auto p-8">
      <h2 className="text-xl font-semibold text-[color:var(--color-text)] mb-6">
        Settings
      </h2>

      <div className="space-y-6">
        <div className="space-y-2">
          <Button ref={openSettingsRef} onClick={() => setAllSettings(true)}>
            All settings
          </Button>
          <p className="text-xs text-[color:var(--color-text-secondary)]">
            Connections, console enrollment, privacy, startup, notifications and
            diagnostics.
          </p>
        </div>
        <fieldset className="space-y-3">
          <legend className="text-sm font-medium">Sidebar layout</legend>
          <p
            id="sidebar-layout-help"
            className="text-sm text-[color:var(--color-text-secondary)]"
          >
            Choose how many task groups appear in the sidebar. All features
            stays available in either layout.
          </p>
          <div
            className="grid grid-cols-2 gap-3"
            aria-describedby="sidebar-layout-help"
          >
            {(["simple", "advanced"] as const).map((mode) => (
              <label
                key={mode}
                className={cn(
                  "cursor-pointer rounded-lg border-2 p-3 text-sm",
                  draftMode === mode
                    ? "border-[var(--color-primary)] bg-[var(--color-selected-bg)]"
                    : "border-[var(--color-border)]",
                )}
              >
                <span className="flex items-center gap-2">
                  <input
                    type="radio"
                    name="sidebar-layout"
                    value={mode}
                    checked={draftMode === mode}
                    onChange={() => {
                      setDraftMode(mode);
                      setLayoutNotice("");
                    }}
                  />
                  <span className="font-medium">
                    {mode === "simple" ? "Basic" : "Advanced"}
                  </span>
                </span>
                <span className="block mt-2 text-xs text-[color:var(--color-text-secondary)]">
                  {mode === "simple"
                    ? "Everyday tasks, fewer choices"
                    : "Everyday tasks plus advanced groups"}
                </span>
              </label>
            ))}
          </div>
          <div
            className="rounded-md border border-[var(--color-border)] p-3 text-xs text-[color:var(--color-text-secondary)]"
            aria-label="Sidebar preview"
          >
            <p className="font-medium text-[color:var(--color-text)] mb-1">
              {draftMode === "simple" ? "Basic" : "Advanced"} preview
            </p>
            <p>Files · Search · Favorites · All features</p>
            <p>Transfers · Backups & Restore · Sync · Cloud & Servers</p>
            {draftMode === "advanced" && (
              <p>Automation & AI · Security & Sharing · Tools & Diagnostics</p>
            )}
            <p>Activity · Settings</p>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              disabled={draftMode === appMode}
              onClick={() => {
                setAppMode(draftMode);
                setLayoutNotice(
                  `${draftMode === "simple" ? "Basic" : "Advanced"} sidebar applied. Saved on this device.`,
                );
              }}
            >
              Apply sidebar layout
            </Button>
            {draftMode !== appMode && (
              <Button
                variant="ghost"
                onClick={() => {
                  setDraftMode(appMode);
                  setLayoutNotice("");
                }}
              >
                Cancel
              </Button>
            )}
          </div>
          <p
            role="status"
            className="text-xs text-[color:var(--color-text-secondary)]"
          >
            {layoutNotice ||
              (draftMode === appMode
                ? `Current layout: ${appMode === "simple" ? "Basic" : "Advanced"}.`
                : "Preview only. Apply to update the sidebar.")}
          </p>
        </fieldset>

        {/* Theme */}
        <div className="space-y-2">
          <label className="block text-sm font-medium text-[color:var(--color-text)]">
            Appearance
          </label>
          <p className="text-xs text-[color:var(--color-text-secondary)]">
            Appearance changes are saved immediately.
          </p>
          <div className="flex flex-wrap gap-2">
            {(["system", "light", "dark", "high-contrast"] as const).map(
              (t) => (
                <button
                  key={t}
                  className={cn(
                    "px-3 py-2 rounded-lg border text-xs transition-all capitalize",
                    theme === t
                      ? "border-[var(--color-primary)] bg-[var(--color-primary)]/10 text-[color:var(--color-primary)]"
                      : "border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:border-[var(--color-text-tertiary)]",
                  )}
                  aria-pressed={theme === t}
                  onClick={() => setTheme(t)}
                >
                  {t === "high-contrast" ? "High Contrast" : t}
                </button>
              ),
            )}
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

function SavedLocations({ onOpen }: { onOpen: () => void }) {
  const favorites = useFileManagerStore((s) => s.favorites);
  const recent = useFileManagerStore((s) => s.recentLocations);
  const open = (path: string) => {
    const store = useFileManagerStore.getState();
    const index = store.activePaneIndex;
    store.revealPath(index, path, true);
    onOpen();
  };
  return (
    <section className="p-5 space-y-5" aria-label="Saved locations">
      <h2 className="text-lg font-semibold">Favorites</h2>
      {favorites.length ? (
        <ul className="space-y-2">
          {favorites.map((f) => (
            <li key={f.id}>
              <Button variant="ghost" onClick={() => open(f.path)}>
                <FolderOpen className="h-4 w-4 mr-2" />
                {f.name}
              </Button>
              <p className="text-xs break-all text-[color:var(--color-text-secondary)]">
                {f.path}
              </p>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-sm text-[color:var(--color-text-secondary)]">
          No favorites yet. Save a folder from the file browser to find it here.
        </p>
      )}
      <h3 className="font-medium">Recent folders</h3>
      {recent.length ? (
        <ul>
          {recent.map(({ path }) => (
            <li key={path}>
              <button
                className="py-2 text-sm text-left break-all hover:underline"
                onClick={() => open(path)}
              >
                {path}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-sm text-[color:var(--color-text-secondary)]">
          Folders you visit will appear here.
        </p>
      )}
    </section>
  );
}
