/**
 * Settings → Reset, web-view side. The interface preferences live in the
 * persisted UI and i18n stores; the desktop and transfer preferences are
 * reset by `reset_settings` in Rust (`src-tauri/src/reset.rs`). Data is never
 * reset: favorites, tabs, connections, sync pairs, backups, history.
 */
import { tauriInvoke } from "@/hooks/use-tauri";
import { useUIStore } from "@/stores/ui-store";
import { useI18n } from "@/lib/i18n";
import { useFeaturePeekStore } from "@/stores/feature-peek-store";

export type ResetScope = "interface" | "desktop" | "transfers" | "tips";

/** What each scope covers, for the Reset section. */
export const RESET_SCOPES: readonly { id: ResetScope; label: string; covers: string; desktopOnly?: boolean }[] = [
  {
    id: "interface",
    label: "Appearance and layout",
    covers: "Theme, accent colour, language, text size, units, Simple/Advanced mode, toolbar, sidebar, panels, table columns, drag-and-drop confirmation, default download and upload folders",
  },
  { id: "desktop", label: "Startup, tray and notifications", covers: "Start at login, start minimised, tray behaviour, desktop notifications", desktopOnly: true },
  { id: "transfers", label: "Sync and transfer defaults", covers: "Global sync defaults, bandwidth timetable, how long failed transfers are kept", desktopOnly: true },
  { id: "tips", label: "Welcome tour and tips", covers: "Shows the welcome wizard and the feature tips again" },
];

/** Kept whatever is chosen. */
export const RESET_KEEPS =
  "Connections, sync pairs, backup jobs, mounts, favorites, history, saved passwords and keys, and the local web server, chat control, metrics, rclone, proxy and console settings.";

/** Interface preferences back to the defaults a new install has. */
export function resetInterfacePreferences() {
  const d = useUIStore.getInitialState();
  const ui = useUIStore.getState();
  // Through the setters, so the page updates (theme, text size, accent, units).
  ui.setTheme(d.theme);
  ui.setFontScale(d.fontScale);
  ui.setAccentColor(d.accentColor);
  ui.setByteUnits(d.byteUnits);
  ui.setTimeDisplay(d.timeDisplay);
  useUIStore.setState({
    tableViews: d.tableViews,
    sidebarOpen: d.sidebarOpen,
    sidebarWidth: d.sidebarWidth,
    appMode: d.appMode,
    backupAdvanced: d.backupAdvanced,
    viewMode: d.viewMode,
    statusBarVisible: d.statusBarVisible,
    previewPanelOpen: d.previewPanelOpen,
    previewPanelWidth: d.previewPanelWidth,
    toolbarItems: [...d.toolbarItems],
    editorAutosave: d.editorAutosave,
    confirmDragDrop: d.confirmDragDrop,
    defaultDownloadDir: d.defaultDownloadDir,
    defaultUploadDir: d.defaultUploadDir,
  });
  void useI18n.getState().setPreference("system");
}

/** The welcome wizard and the feature tips show again. */
export function resetTips() {
  const d = useUIStore.getInitialState();
  useUIStore.setState({ onboardingComplete: false, onboardingStep: d.onboardingStep, userStyle: d.userStyle });
  useFeaturePeekStore.getState().reset();
}

/** Reset the chosen scopes; returns how many stored desktop settings were removed. */
export async function resetSettings(scopes: readonly ResetScope[]): Promise<number> {
  if (scopes.includes("interface")) resetInterfacePreferences();
  if (scopes.includes("tips")) resetTips();
  const backend = scopes.filter((s): s is "desktop" | "transfers" => s === "desktop" || s === "transfers");
  if (backend.length === 0) return 0;
  const report = await tauriInvoke<{ removed: string[] }>("reset_settings", { scopes: backend });
  return report.removed.length;
}
