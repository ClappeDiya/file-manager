/**
 * Settings Panel Component
 *
 * Provides:
 * - Full settings export/import (optionally password-encrypted)
 * - Settings sync across installations via shared directory
 * - Access to network diagnostics wizard
 *
 * Accessible from toolbar/menu. Uses progressive disclosure with
 * collapsible sections to avoid cognitive overload.
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { useAiStore } from "@/stores/ai-store";

// ── Types ──

interface SettingsSummary {
  num_connections: number;
  num_sync_pairs: number;
  num_configs: number;
}

interface SettingsImportResult {
  imported_connections: number;
  imported_connection_groups: number;
  imported_sync_pairs: number;
  imported_configs: number;
  imported_workspace: boolean;
}

interface SyncStatus {
  sync_dir: string | null;
  last_synced: string | null;
  status: string;
  auto_sync_on_startup: boolean;
}

interface NotificationPrefs {
  transfer_complete: boolean;
  transfer_failed: boolean;
  sync_complete: boolean;
  sync_failed: boolean;
  connection_lost: boolean;
  only_when_backgrounded: boolean;
}

// ── Collapsible Section ──

function Section({
  title,
  defaultOpen = false,
  children,
}: {
  title: string;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between px-4 py-3 text-sm font-medium text-left hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors"
      >
        <span>{title}</span>
        <span className="text-gray-400 text-xs">{open ? "collapse" : "expand"}</span>
      </button>
      {open && (
        <div className="px-4 pb-4 border-t border-gray-200 dark:border-gray-700">
          {children}
        </div>
      )}
    </div>
  );
}

// ── Export Section ──

function ExportSection({ summary }: { summary: SettingsSummary | null }) {
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState("");
  const [error, setError] = useState("");

  const handleExport = async () => {
    setError("");
    setExportResult("");

    if (password && password !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setExporting(true);
    try {
      const data = await tauriInvoke<string>("export_all_settings", {
        password: password || null,
      });
      setExportResult(data);
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const handleDownload = () => {
    if (!exportResult) return;
    const blob = new Blob([exportResult], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `ufop-settings-${new Date().toISOString().slice(0, 10)}.ufops`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const handleCopy = async () => {
    if (!exportResult) return;
    try {
      await navigator.clipboard.writeText(exportResult);
    } catch {
      // Fallback
    }
  };

  return (
    <div className="space-y-3 pt-3">
      {summary && (
        <div className="text-xs text-gray-500 dark:text-gray-400 space-y-1">
          <p>{summary.num_connections} connection(s), {summary.num_sync_pairs} sync pair(s), {summary.num_configs} config(s)</p>
        </div>
      )}

      <div className="space-y-2">
        <label className="block text-xs font-medium mb-1">
          Encryption Password{" "}
          <span className="text-gray-400 font-normal">(optional)</span>
        </label>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="Leave empty for unencrypted export"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        {password && (
          <input
            type="password"
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            placeholder="Confirm password"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        )}
      </div>

      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleExport}
          disabled={exporting}
          className="text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {exporting ? "Exporting..." : "Export All Settings"}
        </button>
      </div>

      {error && <p className="text-xs text-red-500">{error}</p>}

      {exportResult && (
        <div className="space-y-2">
          <p className="text-xs text-green-600 dark:text-green-400">
            Export ready ({Math.round(exportResult.length / 1024)} KB)
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={handleDownload}
              className="text-xs px-3 py-1.5 rounded bg-green-500 text-white hover:bg-green-600"
            >
              Download File
            </button>
            <button
              type="button"
              onClick={handleCopy}
              className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
            >
              Copy to Clipboard
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Import Section ──

function ImportSection({
  onImported,
}: {
  onImported: () => void;
}) {
  const [importData, setImportData] = useState("");
  const [password, setPassword] = useState("");
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<SettingsImportResult | null>(null);
  const [dragOver, setDragOver] = useState(false);

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      setImportData(reader.result as string);
      setResult(null);
      setError("");
    };
    reader.readAsText(file);
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      setImportData(reader.result as string);
      setResult(null);
      setError("");
    };
    reader.readAsText(file);
  };

  const handleImport = async () => {
    if (!importData.trim()) {
      setError("No import data provided.");
      return;
    }
    setError("");
    setResult(null);
    setImporting(true);

    try {
      const res = await tauriInvoke<SettingsImportResult>("import_all_settings", {
        data: importData.trim(),
        password: password || null,
      });
      setResult(res);
      onImported();
    } catch (err) {
      setError(String(err));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="space-y-3 pt-3">
      {/* Drag-drop zone */}
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={handleDrop}
        className={`border-2 border-dashed rounded-lg p-4 text-center transition-colors ${
          dragOver
            ? "border-blue-400 bg-blue-50 dark:bg-blue-900/20"
            : "border-gray-300 dark:border-gray-600"
        }`}
      >
        <p className="text-sm text-gray-500 dark:text-gray-400 mb-2">
          Drag a .ufops file here, or:
        </p>
        <label className="text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 cursor-pointer inline-block">
          Browse File
          <input
            type="file"
            accept=".ufops,.txt,.json"
            className="hidden"
            onChange={handleFileSelect}
          />
        </label>
      </div>

      {importData && (
        <p className="text-xs text-gray-500 dark:text-gray-400">
          Loaded {Math.round(importData.length / 1024)} KB of data
        </p>
      )}

      {/* Or paste directly */}
      <div>
        <label className="block text-xs font-medium mb-1">
          Or paste exported data directly
        </label>
        <textarea
          value={importData}
          onChange={(e) => {
            setImportData(e.target.value);
            setResult(null);
            setError("");
          }}
          rows={3}
          placeholder="Paste base64 settings data here..."
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
        />
      </div>

      <div>
        <label className="block text-xs font-medium mb-1">
          Decryption Password{" "}
          <span className="text-gray-400 font-normal">(if encrypted)</span>
        </label>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="Leave empty if unencrypted"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>

      <button
        type="button"
        onClick={handleImport}
        disabled={importing || !importData.trim()}
        className="text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
      >
        {importing ? "Importing..." : "Import Settings"}
      </button>

      {error && <p className="text-xs text-red-500">{error}</p>}

      {result && (
        <div className="text-xs text-green-600 dark:text-green-400 space-y-0.5 bg-green-50 dark:bg-green-900/20 p-2 rounded">
          <p>Import complete:</p>
          <ul className="list-disc list-inside ml-1">
            <li>{result.imported_connections} connection(s)</li>
            <li>{result.imported_connection_groups} group(s)</li>
            <li>{result.imported_sync_pairs} sync pair(s)</li>
            <li>{result.imported_configs} config(s)</li>
            {result.imported_workspace && <li>Workspace state restored</li>}
          </ul>
        </div>
      )}
    </div>
  );
}

// ── Sync Settings Section ──

function SyncSection() {
  const [syncStatus, setSyncStatus] = useState<SyncStatus | null>(null);
  const [newDir, setNewDir] = useState("");
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  const loadStatus = useCallback(async () => {
    const status = await tauriInvokeSafe<SyncStatus | null>(
      "get_settings_sync_status",
      undefined,
      null,
    );
    setSyncStatus(status);
    if (status?.sync_dir) {
      setNewDir(status.sync_dir);
    }
  }, []);

  useEffect(() => {
    loadStatus();
  }, [loadStatus]);

  const handleSetDir = async () => {
    if (!newDir.trim()) return;
    setError("");
    setMessage("");
    try {
      await tauriInvoke("set_settings_sync_dir", { path: newDir.trim() });
      setMessage("Sync directory saved.");
      loadStatus();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleSyncTo = async () => {
    setSyncing(true);
    setError("");
    setMessage("");
    try {
      await tauriInvoke("sync_settings_to_dir", {});
      setMessage("Settings synced to shared directory.");
      loadStatus();
    } catch (err) {
      setError(String(err));
    } finally {
      setSyncing(false);
    }
  };

  const handleSyncFrom = async () => {
    setSyncing(true);
    setError("");
    setMessage("");
    try {
      const result = await tauriInvoke<SettingsImportResult>("sync_settings_from_dir", {});
      const total =
        result.imported_connections +
        result.imported_sync_pairs +
        result.imported_configs;
      if (total === 0 && !result.imported_workspace) {
        setMessage("Already up to date. No new settings to import.");
      } else {
        setMessage(
          `Imported ${result.imported_connections} connections, ${result.imported_sync_pairs} sync pairs, ${result.imported_configs} configs.`,
        );
      }
      loadStatus();
    } catch (err) {
      setError(String(err));
    } finally {
      setSyncing(false);
    }
  };

  const handleToggleAutoSync = async () => {
    const next = !(syncStatus?.auto_sync_on_startup ?? false);
    try {
      await tauriInvoke("set_settings_auto_sync", { enabled: next });
      loadStatus();
    } catch (err) {
      setError(String(err));
    }
  };

  const statusLabel =
    syncStatus?.status === "synced"
      ? "Synced"
      : syncStatus?.status === "pending"
        ? "Pending changes"
        : syncStatus?.status === "never_synced"
          ? "Never synced"
          : "Not configured";

  const statusColor =
    syncStatus?.status === "synced"
      ? "text-green-500"
      : syncStatus?.status === "pending"
        ? "text-yellow-500"
        : "text-gray-400";

  return (
    <div className="space-y-3 pt-3">
      <div className="flex items-center gap-2 text-xs">
        <span className={`font-medium ${statusColor}`}>{statusLabel}</span>
        {syncStatus?.last_synced && (
          <span className="text-gray-400">
            Last: {new Date(syncStatus.last_synced).toLocaleString()}
          </span>
        )}
      </div>

      <div className="space-y-1">
        <label className="block text-xs font-medium">Shared Sync Directory</label>
        <div className="flex gap-2">
          <input
            type="text"
            value={newDir}
            onChange={(e) => setNewDir(e.target.value)}
            placeholder="/path/to/dropbox/ufop-sync"
            className="flex-1 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <button
            type="button"
            onClick={handleSetDir}
            disabled={!newDir.trim()}
            className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            Save
          </button>
        </div>
        <p className="text-xs text-gray-400">
          Choose a folder synced by Dropbox, iCloud, Google Drive, etc.
        </p>
      </div>

      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleSyncTo}
          disabled={syncing || !syncStatus?.sync_dir}
          className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {syncing ? "Syncing..." : "Push Settings"}
        </button>
        <button
          type="button"
          onClick={handleSyncFrom}
          disabled={syncing || !syncStatus?.sync_dir}
          className="text-sm px-3 py-1.5 rounded border border-blue-500 text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 disabled:opacity-50"
        >
          {syncing ? "Syncing..." : "Pull Settings"}
        </button>
      </div>

      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={syncStatus?.auto_sync_on_startup ?? false}
          onChange={handleToggleAutoSync}
          className="rounded"
        />
        Auto-sync on startup
      </label>

      {error && <p className="text-xs text-red-500">{error}</p>}
      {message && <p className="text-xs text-green-600 dark:text-green-400">{message}</p>}
    </div>
  );
}

// ── Notification Section ──

const DEFAULT_NOTIFICATION_PREFS: NotificationPrefs = {
  transfer_complete: true,
  transfer_failed: true,
  sync_complete: false,
  sync_failed: true,
  connection_lost: false,
  only_when_backgrounded: true,
};

function NotificationSection() {
  const [prefs, setPrefs] = useState<NotificationPrefs>(DEFAULT_NOTIFICATION_PREFS);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    (async () => {
      const saved = await tauriInvokeSafe<NotificationPrefs | null>(
        "get_notification_prefs",
        undefined,
        null,
      );
      if (saved) {
        setPrefs(saved);
      }
      setLoaded(true);
    })();
  }, []);

  const updatePref = useCallback(
    async (key: keyof NotificationPrefs, value: boolean) => {
      const next = { ...prefs, [key]: value };
      setPrefs(next);
      setError("");
      try {
        await tauriInvoke("set_notification_prefs", { prefs: next });
      } catch (err) {
        setError(String(err));
      }
    },
    [prefs],
  );

  if (!loaded) {
    return (
      <div className="pt-3 text-xs text-gray-400">Loading preferences...</div>
    );
  }

  const toggles: { key: keyof NotificationPrefs; label: string }[] = [
    { key: "transfer_complete", label: "Transfer Complete" },
    { key: "transfer_failed", label: "Transfer Failed" },
    { key: "sync_complete", label: "Sync Complete" },
    { key: "sync_failed", label: "Sync Failed" },
    { key: "connection_lost", label: "Connection Lost" },
    { key: "only_when_backgrounded", label: "Only when backgrounded" },
  ];

  return (
    <div className="space-y-3 pt-3">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Control when UFOP sends system notifications.
      </p>

      <div className="space-y-2">
        {toggles.map(({ key, label }) => (
          <label
            key={key}
            className="flex items-center gap-2 text-sm cursor-pointer"
          >
            <input
              type="checkbox"
              checked={prefs[key]}
              onChange={(e) => updatePref(key, e.target.checked)}
              className="rounded"
            />
            {label}
          </label>
        ))}
      </div>

      {error && <p className="text-xs text-red-500">{error}</p>}
    </div>
  );
}

// ── Privacy Section ──

function PrivacySection() {
  const [autoUpdate, setAutoUpdate] = useState(true);
  const [crashReport, setCrashReport] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");

  // Load privacy settings on mount
  useEffect(() => {
    (async () => {
      const settings = await tauriInvokeSafe<{ auto_update_check: boolean; crash_report_opt_in: boolean } | null>(
        "get_privacy_settings",
        undefined,
        null,
      );
      if (settings) {
        setAutoUpdate(settings.auto_update_check);
        setCrashReport(settings.crash_report_opt_in);
      }
      setLoaded(true);
    })();
  }, []);

  const updateSetting = useCallback(
    async (key: "autoUpdate" | "crashReport", value: boolean) => {
      const nextAutoUpdate = key === "autoUpdate" ? value : autoUpdate;
      const nextCrashReport = key === "crashReport" ? value : crashReport;
      if (key === "autoUpdate") setAutoUpdate(value);
      if (key === "crashReport") setCrashReport(value);
      setError("");
      try {
        await tauriInvoke("set_privacy_settings", {
          settings: {
            auto_update_check: nextAutoUpdate,
            crash_report_opt_in: nextCrashReport,
          },
        });
      } catch (err) {
        setError(String(err));
      }
    },
    [autoUpdate, crashReport],
  );

  if (!loaded) {
    return <div className="pt-3 text-xs text-gray-400">Loading privacy settings...</div>;
  }

  return (
    <div className="space-y-3 pt-3">
      <div className="p-3 rounded-lg bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800">
        <p className="text-xs font-medium text-green-700 dark:text-green-400">
          UFOP sends no usage data or analytics.
        </p>
        <p className="text-xs text-green-600 dark:text-green-500 mt-1">
          Your files, connections, and activity never leave your device.
          UFOP operates entirely locally with no telemetry, tracking, or data collection of any kind.
        </p>
      </div>

      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={autoUpdate}
          onChange={(e) => updateSetting("autoUpdate", e.target.checked)}
          className="rounded"
        />
        Check for updates automatically
      </label>
      <p className="text-xs text-gray-400 ml-6 -mt-2">
        When enabled, UFOP checks for new versions on launch. No personal data is sent.
      </p>

      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={crashReport}
          onChange={(e) => updateSetting("crashReport", e.target.checked)}
          className="rounded"
        />
        Send anonymous crash reports
      </label>
      <p className="text-xs text-gray-400 ml-6 -mt-2">
        Help improve UFOP by sending stack traces when crashes occur. No file contents or connection details are included.
      </p>

      {error && <p className="text-xs text-red-500">{error}</p>}
    </div>
  );
}

// ── Editor Mappings Section ──

interface EditorMappingEntry {
  id: string;
  extension: string;
  appName: string;
  appPath: string;
  args: string;
}

function EditorMappingsSection() {
  const [mappings, setMappings] = useState<EditorMappingEntry[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [extension, setExtension] = useState("");
  const [appName, setAppName] = useState("");
  const [appPath, setAppPath] = useState("");
  const [args, setArgs] = useState("");
  const [doubleClick, setDoubleClick] = useState("open");
  const [error, setError] = useState("");

  const handleAdd = () => {
    if (!extension.trim() || !appPath.trim()) {
      setError("Extension and application path are required.");
      return;
    }
    const entry: EditorMappingEntry = {
      id: `em-${Date.now()}`,
      extension: extension.trim().replace(/^\./, ""),
      appName: appName.trim() || appPath.split("/").pop() || "Editor",
      appPath: appPath.trim(),
      args: args.trim(),
    };
    setMappings([...mappings, entry]);
    setExtension("");
    setAppName("");
    setAppPath("");
    setArgs("");
    setShowForm(false);
    setError("");
  };

  const handleRemove = (id: string) => {
    setMappings(mappings.filter((m) => m.id !== id));
  };

  return (
    <div className="space-y-3 pt-3">
      <div>
        <label className="block text-xs font-medium mb-1">Double-Click Behavior</label>
        <select
          value={doubleClick}
          onChange={(e) => setDoubleClick(e.target.value)}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        >
          <option value="open">Open in system default app</option>
          <option value="edit">Open in mapped editor</option>
          <option value="preview">Show in preview pane</option>
          <option value="transfer">Start transfer (download/upload)</option>
        </select>
      </div>

      <div>
        <label className="block text-xs font-medium mb-2">File Type Mappings</label>
        {mappings.length === 0 ? (
          <p className="text-xs text-gray-400">No custom mappings. Files open with system defaults.</p>
        ) : (
          <div className="space-y-1.5">
            {mappings.map((m) => (
              <div key={m.id} className="flex items-center justify-between text-xs p-2 rounded border border-gray-200 dark:border-gray-700">
                <div className="min-w-0 flex-1">
                  <span className="font-mono font-medium">.{m.extension}</span>
                  <span className="text-gray-400 mx-2">&rarr;</span>
                  <span>{m.appName}</span>
                </div>
                <button
                  onClick={() => handleRemove(m.id)}
                  className="text-red-500 hover:text-red-700 ml-2 text-xs"
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {showForm ? (
        <div className="space-y-2 border-t border-gray-200 dark:border-gray-700 pt-2">
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs text-gray-500 mb-0.5">File Extension</label>
              <input
                type="text"
                value={extension}
                onChange={(e) => setExtension(e.target.value)}
                placeholder="py, js, txt, *"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
              />
            </div>
            <div>
              <label className="block text-xs text-gray-500 mb-0.5">App Name</label>
              <input
                type="text"
                value={appName}
                onChange={(e) => setAppName(e.target.value)}
                placeholder="VS Code"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
          </div>
          <div>
            <label className="block text-xs text-gray-500 mb-0.5">Application Path</label>
            <input
              type="text"
              value={appPath}
              onChange={(e) => setAppPath(e.target.value)}
              placeholder="/usr/local/bin/code"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
            />
          </div>
          <div>
            <label className="block text-xs text-gray-500 mb-0.5">CLI Arguments (optional)</label>
            <input
              type="text"
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder="--wait --new-window"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
            />
          </div>
          <div className="flex gap-2">
            <button
              onClick={handleAdd}
              className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600"
            >
              Add Mapping
            </button>
            <button
              onClick={() => { setShowForm(false); setError(""); }}
              className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
            >
              Cancel
            </button>
          </div>
          {error && <p className="text-xs text-red-500">{error}</p>}
        </div>
      ) : (
        <button
          onClick={() => setShowForm(true)}
          className="text-xs text-blue-500 hover:text-blue-600"
        >
          + Add File Type Mapping
        </button>
      )}
    </div>
  );
}

// ── Log Controls Section ──

type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

function LogControlsSection() {
  const [logLevel, setLogLevel] = useState<LogLevel>("info");
  const [logDir, setLogDir] = useState("");
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  useEffect(() => {
    (async () => {
      const level = await tauriInvokeSafe<LogLevel>("get_log_level", undefined, "info");
      setLogLevel(level);
      const dir = await tauriInvokeSafe<string>("get_log_directory", undefined, "");
      setLogDir(dir);
    })();
  }, []);

  const handleLevelChange = async (level: LogLevel) => {
    setError("");
    setMessage("");
    try {
      await tauriInvoke("set_log_level", { level });
      setLogLevel(level);
      setMessage(`Log level set to ${level}`);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleExportJson = async () => {
    setExporting(true);
    setError("");
    setMessage("");
    try {
      const entries = await tauriInvoke<unknown[]>("export_logs_json", { maxEntries: 1000 });
      const json = JSON.stringify(entries, null, 2);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `ufop-logs-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
      setMessage(`Exported ${entries.length} log entries`);
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const levels: LogLevel[] = ["error", "warn", "info", "debug", "trace"];

  return (
    <div className="space-y-3 pt-3">
      <div>
        <label className="block text-xs font-medium mb-1">Log Level</label>
        <div className="flex gap-1">
          {levels.map((level) => (
            <button
              key={level}
              type="button"
              onClick={() => handleLevelChange(level)}
              className={`text-xs px-2.5 py-1 rounded capitalize ${
                logLevel === level
                  ? "bg-blue-500 text-white"
                  : "border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
              }`}
            >
              {level}
            </button>
          ))}
        </div>
      </div>

      {logDir && (
        <div className="text-xs text-gray-500 dark:text-gray-400">
          Log directory: <span className="font-mono">{logDir}</span>
        </div>
      )}

      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleExportJson}
          disabled={exporting}
          className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {exporting ? "Exporting..." : "Export Logs (JSON)"}
        </button>
      </div>

      {error && <p className="text-xs text-red-500">{error}</p>}
      {message && <p className="text-xs text-green-600 dark:text-green-400">{message}</p>}
    </div>
  );
}

// ── Trusted SSH Host CAs Section ──

interface TrustedHostCA {
  id: string;
  name: string;
  public_key: string;
  host_pattern: string | null;
  added_at: string;
}

function TrustedHostCAsSection() {
  const [cas, setCas] = useState<TrustedHostCA[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [hostPattern, setHostPattern] = useState("");
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState("");

  const loadCAs = useCallback(async () => {
    const result = await tauriInvokeSafe<TrustedHostCA[]>("list_trusted_host_cas", undefined, []);
    setCas(result);
  }, []);

  useEffect(() => {
    loadCAs();
  }, [loadCAs]);

  const handleAdd = async () => {
    if (!name.trim() || !publicKey.trim()) {
      setError("Name and public key are required");
      return;
    }
    setAdding(true);
    setError("");
    try {
      await tauriInvoke("add_trusted_host_ca", {
        name: name.trim(),
        publicKey: publicKey.trim(),
        hostPattern: hostPattern.trim() || null,
      });
      setName("");
      setPublicKey("");
      setHostPattern("");
      setShowForm(false);
      loadCAs();
    } catch (err) {
      setError(String(err));
    } finally {
      setAdding(false);
    }
  };

  const handleRemove = async (caId: string) => {
    try {
      await tauriInvoke("remove_trusted_host_ca", { caId });
      loadCAs();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="space-y-3 pt-3">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Trust SSH certificate authorities for host verification.
      </p>

      {cas.length === 0 ? (
        <p className="text-xs text-gray-400">No trusted CAs configured.</p>
      ) : (
        <div className="space-y-1.5">
          {cas.map((ca) => (
            <div key={ca.id} className="flex items-center justify-between text-xs p-2 rounded border border-gray-200 dark:border-gray-700">
              <div className="min-w-0 flex-1">
                <div className="font-medium">{ca.name}</div>
                {ca.host_pattern && (
                  <div className="text-gray-400 font-mono">{ca.host_pattern}</div>
                )}
              </div>
              <button
                onClick={() => handleRemove(ca.id)}
                className="text-red-500 hover:text-red-700 ml-2 text-xs"
              >
                Remove
              </button>
            </div>
          ))}
        </div>
      )}

      {showForm ? (
        <div className="space-y-2 border-t border-gray-200 dark:border-gray-700 pt-2">
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="CA Name (e.g., Corporate CA)"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <textarea
            value={publicKey}
            onChange={(e) => setPublicKey(e.target.value)}
            placeholder="Paste CA public key (.pub file contents)"
            rows={3}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
          />
          <input
            type="text"
            value={hostPattern}
            onChange={(e) => setHostPattern(e.target.value)}
            placeholder="Host pattern (optional, e.g., *.example.com)"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <div className="flex gap-2">
            <button
              onClick={handleAdd}
              disabled={adding}
              className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
            >
              {adding ? "Adding..." : "Add CA"}
            </button>
            <button
              onClick={() => setShowForm(false)}
              className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button
          onClick={() => setShowForm(true)}
          className="text-xs text-blue-500 hover:text-blue-600"
        >
          + Add Trusted CA
        </button>
      )}

      {error && <p className="text-xs text-red-500">{error}</p>}
    </div>
  );
}

// ── GSSAPI Status Section ──

interface GssapiStatus {
  available: boolean;
  has_ticket_cache: boolean;
  krb5_config_path: string | null;
  ticket_cache_path: string | null;
}

function GssapiSection() {
  const [status, setStatus] = useState<GssapiStatus | null>(null);

  useEffect(() => {
    (async () => {
      const result = await tauriInvokeSafe<GssapiStatus | null>("check_gssapi_available", undefined, null);
      setStatus(result);
    })();
  }, []);

  if (!status) return null;

  return (
    <div className="space-y-2 pt-3">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        GSSAPI/Kerberos authentication for enterprise SSH environments.
      </p>
      <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
        <span className="text-gray-500 dark:text-gray-400">Available</span>
        <span className={status.available ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
          {status.available ? "Yes" : "No (krb5.conf not found)"}
        </span>
        <span className="text-gray-500 dark:text-gray-400">Ticket Cache</span>
        <span className={status.has_ticket_cache ? "text-green-600 dark:text-green-400" : "text-gray-400"}>
          {status.has_ticket_cache ? "Active" : "No ticket"}
        </span>
        {status.krb5_config_path && (
          <>
            <span className="text-gray-500 dark:text-gray-400">Config</span>
            <span className="font-mono truncate" title={status.krb5_config_path}>{status.krb5_config_path}</span>
          </>
        )}
        {status.ticket_cache_path && (
          <>
            <span className="text-gray-500 dark:text-gray-400">Cache</span>
            <span className="font-mono truncate" title={status.ticket_cache_path}>{status.ticket_cache_path}</span>
          </>
        )}
      </div>
      {!status.available && (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          To use GSSAPI auth, install Kerberos and create /etc/krb5.conf.
        </p>
      )}
    </div>
  );
}

// ── AI / Ollama Section ──

function OllamaSection() {
  const { ollamaAvailable, ollamaModel, probeOllama, setOllamaModel, loadOllamaModel } =
    useAiStore();
  const [modelInput, setModelInput] = useState(ollamaModel);

  useEffect(() => {
    probeOllama();
    loadOllamaModel();
  }, [probeOllama, loadOllamaModel]);

  useEffect(() => {
    setModelInput(ollamaModel);
  }, [ollamaModel]);

  const handleModelSave = useCallback(() => {
    const trimmed = modelInput.trim();
    if (trimmed && trimmed !== ollamaModel) {
      setOllamaModel(trimmed);
    }
  }, [modelInput, ollamaModel, setOllamaModel]);

  return (
    <div className="pt-3 space-y-3">
      {/* Status */}
      <div className="flex items-center justify-between">
        <span className="text-xs text-gray-500 dark:text-gray-400">Local AI (Ollama)</span>
        <span
          className={`text-xs font-medium ${
            ollamaAvailable === true
              ? "text-green-500"
              : ollamaAvailable === false
                ? "text-gray-400"
                : "text-yellow-500"
          }`}
        >
          {ollamaAvailable === true
            ? "Connected"
            : ollamaAvailable === false
              ? "Not detected"
              : "Checking..."}
        </span>
      </div>

      {/* Model selector */}
      <div className="flex items-center gap-2">
        <label className="text-xs text-gray-500 dark:text-gray-400 shrink-0">Model</label>
        <input
          type="text"
          value={modelInput}
          onChange={(e) => setModelInput(e.target.value)}
          onBlur={handleModelSave}
          onKeyDown={(e) => e.key === "Enter" && handleModelSave()}
          className="flex-1 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-transparent"
          placeholder="llama3.2"
        />
      </div>

      {/* Help text */}
      <p className="text-xs text-gray-400">
        {ollamaAvailable
          ? "The Intent Bar (Cmd+K) uses your local Ollama for natural language commands. No data leaves your machine."
          : "Install Ollama for free AI-powered natural language commands in the Intent Bar (Cmd+K). Visit ollama.com to get started."}
      </p>

      {/* Re-probe button */}
      {ollamaAvailable === false && (
        <button
          type="button"
          onClick={() => {
            // Reset cached probe and retry
            probeOllama();
          }}
          className="text-xs px-3 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
        >
          Check again
        </button>
      )}
    </div>
  );
}

// ── Main Settings Panel ──

export default function SettingsPanel({
  onOpenNetworkWizard,
}: {
  onOpenNetworkWizard?: () => void;
}) {
  const [summary, setSummary] = useState<SettingsSummary | null>(null);

  const loadSummary = useCallback(async () => {
    const s = await tauriInvokeSafe<SettingsSummary | null>(
      "get_settings_summary",
      undefined,
      null,
    );
    setSummary(s);
  }, []);

  useEffect(() => {
    loadSummary();
  }, [loadSummary]);

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <h2 className="text-sm font-semibold">Settings</h2>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {/* Export */}
        <Section title="Export Settings" defaultOpen>
          <ExportSection summary={summary} />
        </Section>

        {/* AI / Local LLM */}
        <Section title="AI &amp; Intent Bar">
          <OllamaSection />
        </Section>

        {/* Editor Mappings */}
        <Section title="External Editors">
          <EditorMappingsSection />
        </Section>

        {/* Import */}
        <Section title="Import Settings">
          <ImportSection onImported={loadSummary} />
        </Section>

        {/* Sync */}
        <Section title="Sync Settings Across Devices">
          <SyncSection />
        </Section>

        {/* Notifications */}
        <Section title="Notifications">
          <NotificationSection />
        </Section>

        {/* Privacy */}
        <Section title="Privacy">
          <PrivacySection />
        </Section>

        {/* Log Controls */}
        <Section title="Logging">
          <LogControlsSection />
        </Section>

        {/* Trusted SSH Host CAs */}
        <Section title="Trusted SSH Host CAs">
          <TrustedHostCAsSection />
        </Section>

        {/* GSSAPI/Kerberos */}
        <Section title="GSSAPI / Kerberos">
          <GssapiSection />
        </Section>

        {/* Network Diagnostics */}
        <Section title="Network">
          <div className="pt-3">
            <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">
              Run network diagnostics to troubleshoot connection issues.
            </p>
            <button
              type="button"
              onClick={onOpenNetworkWizard}
              className="text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600"
            >
              Run Diagnostics
            </button>
          </div>
        </Section>
      </div>
    </div>
  );
}
