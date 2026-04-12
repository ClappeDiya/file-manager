/**
 * Sync Panel Component (T-039..T-042)
 *
 * Provides:
 * - Sync pair list with health indicators (green/yellow/red/gray)
 * - Create pair wizard (Simple and Advanced modes)
 * - Dry-run preview with category counts
 * - Sync execution with progress
 * - Conflict resolution UI
 * - Reports and rollback
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";

// ── Types ──

interface SyncPair {
  id: string;
  name: string;
  source_path: string;
  dest_path: string;
  mode: string;
  enabled: boolean;
  last_run: string | null;
  trigger: string | { scheduled: { cron_expr: string } };
  filter: SyncFilter;
  conflict_policy: string;
  verify_mode: string;
  checksum_enabled: boolean;
  created_at: string;
  time_offset_secs: number | null;
}

interface SyncFilter {
  include_patterns: string[];
  exclude_patterns: string[];
  include_regex: string[];
  exclude_regex: string[];
  min_size: number;
  max_size: number;
  file_types: string[];
  exclude_file_types: string[];
}

interface SyncPreview {
  pair_id: string;
  pair_name: string;
  computed_at: string;
  duration_ms: number;
  additions: SyncDiffEntry[];
  modifications: SyncDiffEntry[];
  deletions: SyncDiffEntry[];
  skipped: SyncDiffEntry[];
  conflicts: SyncDiffEntry[];
  path_warnings: SyncDiffEntry[];
  total_additions: number;
  total_modifications: number;
  total_deletions: number;
  total_skipped: number;
  total_conflicts: number;
  total_bytes: number;
}

interface SyncDiffEntry {
  relative_path: string;
  action: string;
  source_size: number | null;
  dest_size: number | null;
  source_modified: string | null;
  dest_modified: string | null;
  reason: string;
  path_length_warning: string | null;
}

interface SyncReport {
  id: string;
  pair_id: string;
  pair_name: string;
  started_at: string;
  ended_at: string;
  duration_ms: number;
  files_added: number;
  files_modified: number;
  files_deleted: number;
  files_skipped: number;
  conflicts_resolved: number;
  errors: number;
  bytes_transferred: number;
  status: string;
  health: string;
  error_messages: string[];
  resumed: boolean;
}

interface SyncConflictItem {
  id: string;
  pair_id: string;
  relative_path: string;
  source_size: number;
  dest_size: number;
  source_modified: string | null;
  dest_modified: string | null;
  resolution: string | null;
  created_at: string;
}

interface QuarantineEntry {
  id: string;
  pair_id: string;
  original_path: string;
  quarantine_path: string;
  quarantined_at: string;
  resolved: boolean;
}

type SyncPanelView = "list" | "create" | "preview" | "conflicts" | "reports" | "quarantine";
type SyncHealth = "green" | "yellow" | "red" | "gray";

// ── Utility functions ──

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const mins = Math.floor(ms / 60000);
  const secs = Math.floor((ms % 60000) / 1000);
  return `${mins}m ${secs}s`;
}

function healthColor(health: SyncHealth): string {
  switch (health) {
    case "green": return "text-green-500";
    case "yellow": return "text-yellow-500";
    case "red": return "text-red-500";
    case "gray": return "text-gray-400";
  }
}

function healthIcon(health: SyncHealth): string {
  switch (health) {
    case "green": return "●";
    case "yellow": return "●";
    case "red": return "●";
    case "gray": return "○";
  }
}

function modeName(mode: string): string {
  switch (mode) {
    case "one_way": return "One-Way";
    case "two_way": return "Two-Way";
    case "mirror": return "Mirror";
    case "versioned_backup": return "Versioned Backup";
    default: return mode;
  }
}

function triggerName(trigger: string | { scheduled: { cron_expr: string } }): string {
  if (typeof trigger === "string") {
    switch (trigger) {
      case "manual": return "Manual";
      case "watch": return "Watch";
      case "api": return "API";
      default: return trigger;
    }
  }
  if ("scheduled" in trigger) {
    return `Scheduled (${trigger.scheduled.cron_expr})`;
  }
  return "Unknown";
}

// ── Main Component ──

export function SyncPanel() {
  const [view, setView] = useState<SyncPanelView>("list");
  const [pairs, setPairs] = useState<SyncPair[]>([]);
  const [healthMap, setHealthMap] = useState<Record<string, SyncHealth>>({});
  const [_selectedPair, _setSelectedPair] = useState<SyncPair | null>(null);
  const [preview, setPreview] = useState<SyncPreview | null>(null);
  const [reports, setReports] = useState<SyncReport[]>([]);
  const [conflicts, setConflicts] = useState<SyncConflictItem[]>([]);
  const [quarantine, _setQuarantine] = useState<QuarantineEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [syncRunning, setSyncRunning] = useState(false);

  // Load pairs on mount
  const loadPairs = useCallback(async () => {
    try {
      const result = await tauriInvoke<SyncPair[]>("list_sync_pairs", undefined, []);
      setPairs(result);

      // Load health for each pair
      const healthResults: Record<string, SyncHealth> = {};
      for (const pair of result) {
        try {
          const h = await tauriInvoke<SyncHealth>("get_sync_health", { pairId: pair.id });
          healthResults[pair.id] = h;
        } catch {
          healthResults[pair.id] = "gray";
        }
      }
      setHealthMap(healthResults);
    } catch (e: any) {
      setError(e?.message || "Failed to load sync pairs");
    }
  }, []);

  useEffect(() => {
    loadPairs();
  }, [loadPairs]);

  // ── Create Pair ──

  const [createForm, setCreateForm] = useState({
    name: "",
    sourcePath: "",
    destPath: "",
    mode: "one_way",
    trigger: "manual",
    cronExpr: "0 0 */6 * * * *",
    conflictPolicy: "create_conflict_copy",
    verifyMode: "fast",
    checksumEnabled: false,
    advancedMode: false,
    excludePatterns: "",
    includePatterns: "",
    fileTypes: "",
    maxSize: 0,
    minSize: 0,
    timeOffsetSecs: null as number | null,
  });

  // Time offset auto-detection state (used for inline display)
  const [_detectedOffset, _setDetectedOffset] = useState<number | null>(null);
  const [_detectingOffset, _setDetectingOffset] = useState(false);

  const handleCreatePair = async () => {
    setLoading(true);
    setError(null);
    try {
      const filter: SyncFilter = {
        include_patterns: createForm.includePatterns
          ? createForm.includePatterns.split(",").map((s) => s.trim())
          : [],
        exclude_patterns: createForm.excludePatterns
          ? createForm.excludePatterns.split(",").map((s) => s.trim())
          : [],
        include_regex: [],
        exclude_regex: [],
        min_size: createForm.minSize,
        max_size: createForm.maxSize,
        file_types: createForm.fileTypes
          ? createForm.fileTypes.split(",").map((s) => s.trim())
          : [],
        exclude_file_types: [],
      };

      await tauriInvoke("create_sync_pair", {
        name: createForm.name,
        sourcePath: createForm.sourcePath,
        destPath: createForm.destPath,
        mode: createForm.mode,
        trigger: createForm.trigger,
        cronExpr: createForm.trigger === "scheduled" ? createForm.cronExpr : null,
        filterJson: JSON.stringify(filter),
        conflictPolicy: createForm.conflictPolicy,
        verifyMode: createForm.verifyMode,
        checksumEnabled: createForm.checksumEnabled,
        timeOffsetSecs: createForm.timeOffsetSecs,
      });

      setView("list");
      await loadPairs();
      setCreateForm({
        name: "",
        sourcePath: "",
        destPath: "",
        mode: "one_way",
        trigger: "manual",
        cronExpr: "0 0 */6 * * * *",
        conflictPolicy: "create_conflict_copy",
        verifyMode: "fast",
        checksumEnabled: false,
        advancedMode: false,
        excludePatterns: "",
        includePatterns: "",
        fileTypes: "",
        maxSize: 0,
        minSize: 0,
        timeOffsetSecs: null,
      });
      _setDetectedOffset(null);
    } catch (e: any) {
      setError(e?.message || "Failed to create sync pair");
    } finally {
      setLoading(false);
    }
  };

  // ── Dry Run ──

  const handleDryRun = async (pairId: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await tauriInvoke<SyncPreview>("sync_dry_run", { pairId });
      setPreview(result);
      setView("preview");
    } catch (e: any) {
      setError(e?.message || "Dry run failed");
    } finally {
      setLoading(false);
    }
  };

  // ── Execute Sync ──

  const handleRunSync = async (pairId: string) => {
    setSyncRunning(true);
    setError(null);
    try {
      const report = await tauriInvoke<SyncReport>("run_sync", { pairId });
      setReports([report]);
      setView("reports");
      await loadPairs();
    } catch (e: any) {
      setError(e?.message || "Sync failed");
    } finally {
      setSyncRunning(false);
    }
  };

  // ── Delete Pair ──

  const handleDeletePair = async (pairId: string) => {
    try {
      await tauriInvoke("delete_sync_pair", { pairId });
      await loadPairs();
    } catch (e: any) {
      setError(e?.message || "Failed to delete sync pair");
    }
  };

  // ── Rollback ──

  const handleRollback = async (pairId: string) => {
    setLoading(true);
    try {
      const restored = await tauriInvoke<number>("rollback_sync", { pairId });
      setError(null);
      alert(`Rollback complete: ${restored} file(s) restored.`);
      await loadPairs();
    } catch (e: any) {
      setError(e?.message || "Rollback failed");
    } finally {
      setLoading(false);
    }
  };

  // ── Load Reports ──

  const handleViewReports = async (pairId: string) => {
    try {
      const result = await tauriInvoke<SyncReport[]>("get_sync_reports", { pairId });
      setReports(result);
      setView("reports");
    } catch (e: any) {
      setError(e?.message || "Failed to load reports");
    }
  };

  // ── Export Preview ──

  const handleExportPreview = async () => {
    if (!preview) return;
    try {
      const csv = await tauriInvoke<string>("export_sync_preview_csv", {
        pairId: preview.pair_id,
      });
      // Create download
      const blob = new Blob([csv], { type: "text/csv" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `sync-preview-${preview.pair_name}.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e: any) {
      setError(e?.message || "Export failed");
    }
  };

  // ── Resolve Conflict ──

  const handleResolveConflict = async (
    conflictId: string,
    resolution: string
  ) => {
    try {
      await tauriInvoke("resolve_sync_conflict", { conflictId, resolution });
      const updated = await tauriInvoke<SyncConflictItem[]>("get_sync_conflicts", undefined, []);
      setConflicts(updated);
    } catch (e: any) {
      setError(e?.message || "Failed to resolve conflict");
    }
  };

  // ── Render ──

  return (
    <div
      className="flex flex-col h-full bg-white dark:bg-zinc-900 border-l border-zinc-200 dark:border-zinc-700"
      role="region"
      aria-label="Sync Panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between p-3 border-b border-zinc-200 dark:border-zinc-700">
        <h2 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Sync Pairs
        </h2>
        <div className="flex gap-1">
          <button
            onClick={() => setView("list")}
            className={`px-2 py-1 text-xs rounded ${
              view === "list"
                ? "bg-blue-500 text-white"
                : "text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"
            }`}
            aria-label="Pair list view"
          >
            List
          </button>
          <button
            onClick={() => setView("create")}
            className="px-2 py-1 text-xs rounded bg-green-500 text-white hover:bg-green-600"
            aria-label="Create new sync pair"
          >
            + New
          </button>
        </div>
      </div>

      {/* Error Banner */}
      {error && (
        <div
          className="p-2 text-xs text-red-700 bg-red-50 dark:bg-red-900/20 dark:text-red-300 border-b border-red-200"
          role="alert"
        >
          {error}
          <button
            onClick={() => setError(null)}
            className="ml-2 underline"
            aria-label="Dismiss error"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-2">
        {view === "list" && (
          <PairListView
            pairs={pairs}
            healthMap={healthMap}
            onDryRun={handleDryRun}
            onRunSync={handleRunSync}
            onDelete={handleDeletePair}
            onRollback={handleRollback}
            onViewReports={handleViewReports}
            syncRunning={syncRunning}
          />
        )}

        {view === "create" && (
          <CreatePairView
            form={createForm}
            setForm={setCreateForm}
            onSubmit={handleCreatePair}
            onCancel={() => setView("list")}
            loading={loading}
          />
        )}

        {view === "preview" && preview && (
          <PreviewView
            preview={preview}
            onProceed={() => {
              handleRunSync(preview.pair_id);
            }}
            onCancel={() => setView("list")}
            onExport={handleExportPreview}
            loading={syncRunning}
          />
        )}

        {view === "reports" && (
          <ReportsView
            reports={reports}
            onBack={() => setView("list")}
          />
        )}

        {view === "conflicts" && (
          <ConflictsView
            conflicts={conflicts}
            onResolve={handleResolveConflict}
            onBack={() => setView("list")}
          />
        )}

        {view === "quarantine" && (
          <QuarantineView
            entries={quarantine}
            onBack={() => setView("list")}
          />
        )}
      </div>
    </div>
  );
}

// ── Sub-Components ──

function PairListView({
  pairs,
  healthMap,
  onDryRun,
  onRunSync,
  onDelete,
  onRollback,
  onViewReports,
  syncRunning,
}: {
  pairs: SyncPair[];
  healthMap: Record<string, SyncHealth>;
  onDryRun: (id: string) => void;
  onRunSync: (id: string) => void;
  onDelete: (id: string) => void;
  onRollback: (id: string) => void;
  onViewReports: (id: string) => void;
  syncRunning: boolean;
}) {
  if (pairs.length === 0) {
    return (
      <div className="text-center text-sm text-zinc-500 py-8">
        No sync pairs configured.
        <br />
        Click "+ New" to create one.
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {pairs.map((pair) => {
        const health = healthMap[pair.id] || "gray";
        return (
          <div
            key={pair.id}
            className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3"
          >
            <div className="flex items-center justify-between mb-1">
              <div className="flex items-center gap-2">
                <span className={`text-lg ${healthColor(health)}`} title={`Health: ${health}`}>
                  {healthIcon(health)}
                </span>
                <span className="text-sm font-medium text-zinc-900 dark:text-zinc-100">
                  {pair.name}
                </span>
              </div>
              <span className="text-xs text-zinc-500">
                {modeName(pair.mode)}
              </span>
            </div>

            <div className="text-xs text-zinc-500 dark:text-zinc-400 space-y-0.5">
              <div className="truncate" title={pair.source_path}>
                Source: {pair.source_path}
              </div>
              <div className="truncate" title={pair.dest_path}>
                Dest: {pair.dest_path}
              </div>
              <div>
                Trigger: {triggerName(pair.trigger)}
                {pair.last_run && (
                  <span className="ml-2">
                    Last run: {new Date(pair.last_run).toLocaleString()}
                  </span>
                )}
              </div>
            </div>

            <div className="flex gap-1 mt-2 flex-wrap">
              <button
                onClick={() => onDryRun(pair.id)}
                className="px-2 py-1 text-xs rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700"
                aria-label={`Preview sync for ${pair.name}`}
              >
                Preview
              </button>
              <button
                onClick={() => onRunSync(pair.id)}
                disabled={syncRunning || !pair.enabled}
                className="px-2 py-1 text-xs rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
                aria-label={`Run sync for ${pair.name}`}
              >
                {syncRunning ? "Running..." : "Sync Now"}
              </button>
              <button
                onClick={() => onViewReports(pair.id)}
                className="px-2 py-1 text-xs rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200"
                aria-label={`View reports for ${pair.name}`}
              >
                Reports
              </button>
              <button
                onClick={() => onRollback(pair.id)}
                className="px-2 py-1 text-xs rounded bg-yellow-100 text-yellow-800 hover:bg-yellow-200"
                aria-label={`Rollback last sync for ${pair.name}`}
              >
                Rollback
              </button>
              <button
                onClick={() => {
                  if (confirm(`Delete sync pair "${pair.name}"?`)) {
                    onDelete(pair.id);
                  }
                }}
                className="px-2 py-1 text-xs rounded bg-red-100 text-red-800 hover:bg-red-200"
                aria-label={`Delete ${pair.name}`}
              >
                Delete
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function CreatePairView({
  form,
  setForm,
  onSubmit,
  onCancel,
  loading,
}: {
  form: any;
  setForm: (f: any) => void;
  onSubmit: () => void;
  onCancel: () => void;
  loading: boolean;
}) {
  const update = (field: string, value: any) =>
    setForm({ ...form, [field]: value });

  const [cronValidation, setCronValidation] = useState<{
    valid: boolean;
    next_runs: string[];
    error: string | null;
  } | null>(null);
  const [cronValidating, setCronValidating] = useState(false);

  // Validate cron expression when it changes
  useEffect(() => {
    if (form.trigger !== "scheduled" || !form.cronExpr.trim()) {
      setCronValidation(null);
      return;
    }
    const timeout = setTimeout(async () => {
      setCronValidating(true);
      try {
        const result = await tauriInvoke<{ valid: boolean; next_runs: string[]; error: string | null }>(
          "validate_sync_cron",
          { cronExpr: form.cronExpr }
        );
        setCronValidation(result);
      } catch (e: any) {
        setCronValidation({ valid: false, next_runs: [], error: e?.message || "Validation failed" });
      } finally {
        setCronValidating(false);
      }
    }, 400); // debounce 400ms
    return () => clearTimeout(timeout);
  }, [form.cronExpr, form.trigger]);

  return (
    <div className="space-y-3">
      <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
        {form.advancedMode ? "Advanced Setup" : "Simple Setup"}
      </h3>

      <div className="flex items-center gap-2 text-xs">
        <button
          onClick={() => update("advancedMode", false)}
          className={`px-2 py-1 rounded ${
            !form.advancedMode ? "bg-blue-500 text-white" : "bg-zinc-100 dark:bg-zinc-800"
          }`}
        >
          Simple
        </button>
        <button
          onClick={() => update("advancedMode", true)}
          className={`px-2 py-1 rounded ${
            form.advancedMode ? "bg-blue-500 text-white" : "bg-zinc-100 dark:bg-zinc-800"
          }`}
        >
          Advanced
        </button>
      </div>

      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Name
        <input
          type="text"
          value={form.name}
          onChange={(e) => update("name", e.target.value)}
          className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
          placeholder="My Backup"
        />
      </label>

      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Source Path
        <input
          type="text"
          value={form.sourcePath}
          onChange={(e) => update("sourcePath", e.target.value)}
          className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
          placeholder="/home/user/documents"
        />
      </label>

      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Destination Path
        <input
          type="text"
          value={form.destPath}
          onChange={(e) => update("destPath", e.target.value)}
          className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
          placeholder="/backup/documents"
        />
      </label>

      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Sync Mode
        <select
          value={form.mode}
          onChange={(e) => update("mode", e.target.value)}
          className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
        >
          <option value="one_way">One-Way (Source to Destination)</option>
          <option value="two_way">Two-Way (Bidirectional)</option>
          <option value="mirror">Mirror (Exact Copy)</option>
          <option value="versioned_backup">Versioned Backup</option>
        </select>
      </label>

      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Trigger
        <select
          value={form.trigger}
          onChange={(e) => update("trigger", e.target.value)}
          className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
        >
          <option value="manual">Manual</option>
          <option value="scheduled">Scheduled (Cron)</option>
          <option value="watch">Watch (Filesystem)</option>
          <option value="api">API/CLI</option>
        </select>
      </label>

      {form.trigger === "scheduled" && (
        <div className="space-y-1">
          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            Cron Expression
            <input
              type="text"
              value={form.cronExpr}
              onChange={(e) => update("cronExpr", e.target.value)}
              className={`mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600 ${
                cronValidation && !cronValidation.valid
                  ? "border-red-400 dark:border-red-500"
                  : cronValidation?.valid
                  ? "border-green-400 dark:border-green-500"
                  : ""
              }`}
              placeholder="0 0 */6 * * * *"
            />
            <span className="text-[10px] text-zinc-400">
              sec min hour day month day_of_week year
            </span>
          </label>
          {cronValidating && (
            <div className="text-[10px] text-zinc-400">Validating...</div>
          )}
          {cronValidation && !cronValidating && (
            <>
              {cronValidation.valid ? (
                <div className="p-1.5 bg-green-50 dark:bg-green-900/20 rounded text-[11px] text-green-700 dark:text-green-400">
                  <div className="font-medium">Valid - Next 3 runs:</div>
                  <ul className="mt-0.5 space-y-0.5">
                    {cronValidation.next_runs.slice(0, 3).map((run, i) => (
                      <li key={i}>{new Date(run).toLocaleString()}</li>
                    ))}
                  </ul>
                </div>
              ) : (
                <div className="p-1.5 bg-red-50 dark:bg-red-900/20 rounded text-[11px] text-red-700 dark:text-red-400">
                  {cronValidation.error || "Invalid cron expression"}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {form.advancedMode && (
        <>
          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            Conflict Policy
            <select
              value={form.conflictPolicy}
              onChange={(e) => update("conflictPolicy", e.target.value)}
              className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
            >
              <option value="create_conflict_copy">Create Conflict Copy</option>
              <option value="ask">Ask Each Time</option>
              <option value="source_wins">Source Wins</option>
              <option value="dest_wins">Destination Wins</option>
              <option value="newest_wins">Newest Wins</option>
              <option value="skip">Skip</option>
              <option value="quarantine">Quarantine</option>
            </select>
          </label>

          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            Verification Mode
            <select
              value={form.verifyMode}
              onChange={(e) => update("verifyMode", e.target.value)}
              className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
            >
              <option value="none">None</option>
              <option value="fast">Fast (Size + Timestamp)</option>
              <option value="full">Full (Checksum)</option>
            </select>
          </label>

          <label className="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-400">
            <input
              type="checkbox"
              checked={form.checksumEnabled}
              onChange={(e) => update("checksumEnabled", e.target.checked)}
            />
            Enable Checksum Verification
          </label>

          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            Exclude Patterns (comma-separated globs)
            <input
              type="text"
              value={form.excludePatterns}
              onChange={(e) => update("excludePatterns", e.target.value)}
              className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
              placeholder="*.tmp, .DS_Store, Thumbs.db"
            />
          </label>

          {/* Quick Filter Presets */}
          <div className="space-y-2">
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400">Quick Filter Presets</label>
            <div className="flex flex-wrap gap-1.5">
              {[
                { id: "macos_metadata", label: "macOS (.DS_Store, ._*)", patterns: [".DS_Store", "._*", ".Spotlight-V100", ".fseventsd", ".Trashes"] },
                { id: "windows_metadata", label: "Windows (Thumbs.db)", patterns: ["Thumbs.db", "desktop.ini", "System Volume Information", "$RECYCLE.BIN"] },
                { id: "version_control", label: "Git/SVN (.git, .svn)", patterns: [".git", ".svn", ".hg", ".bzr"] },
                { id: "ide_files", label: "IDE (.idea, .vscode)", patterns: [".idea", ".vscode", "*.swp", "*~"] },
                { id: "temp_files", label: "Temp (*.tmp, *.log)", patterns: ["*.tmp", "*.temp", "*.bak", "*.log"] },
                { id: "node_modules", label: "Node (node_modules)", patterns: ["node_modules", ".npm", ".yarn"] },
              ].map((preset) => {
                const currentExcludePatterns = form.excludePatterns
                  ? form.excludePatterns.split(",").map((s: string) => s.trim()).filter(Boolean)
                  : [];
                const isActive = preset.patterns.every((p) => currentExcludePatterns.includes(p));
                return (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => {
                      if (isActive) {
                        // Remove preset patterns
                        const remaining = currentExcludePatterns.filter(
                          (p: string) => !preset.patterns.includes(p)
                        );
                        update("excludePatterns", remaining.join(", "));
                      } else {
                        // Add preset patterns (avoid duplicates)
                        const merged = [
                          ...currentExcludePatterns,
                          ...preset.patterns.filter((p) => !currentExcludePatterns.includes(p)),
                        ];
                        update("excludePatterns", merged.join(", "));
                      }
                    }}
                    className={`text-xs px-2.5 py-1 rounded-full border transition-colors ${
                      isActive
                        ? "bg-blue-500 text-white border-blue-500"
                        : "border-zinc-300 dark:border-zinc-600 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                    }`}
                  >
                    {preset.label}
                  </button>
                );
              })}
            </div>
          </div>

          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            Include Patterns (comma-separated, empty = all)
            <input
              type="text"
              value={form.includePatterns}
              onChange={(e) => update("includePatterns", e.target.value)}
              className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
              placeholder="*.doc, *.pdf"
            />
          </label>

          <label className="block text-xs text-zinc-600 dark:text-zinc-400">
            File Types (comma-separated extensions)
            <input
              type="text"
              value={form.fileTypes}
              onChange={(e) => update("fileTypes", e.target.value)}
              className="mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
              placeholder="pdf, docx, xlsx"
            />
          </label>

          {/* Server Time Offset Compensation */}
          <div className="space-y-1.5">
            <label className="block text-xs text-zinc-600 dark:text-zinc-400">
              Server Time Offset
              <span className="text-zinc-400 dark:text-zinc-500 font-normal"> (optional)</span>
            </label>
            <div className="flex items-center gap-2">
              <input
                type="number"
                value={form.timeOffsetSecs ?? ""}
                onChange={(e) =>
                  update(
                    "timeOffsetSecs",
                    e.target.value === "" ? null : parseInt(e.target.value, 10)
                  )
                }
                className="flex-1 px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600"
                placeholder="0"
              />
              <span className="text-xs text-zinc-500">seconds</span>
            </div>
            {form.timeOffsetSecs != null && form.timeOffsetSecs !== 0 && (
              <p className="text-[11px] text-blue-500 dark:text-blue-400">
                Offset: {form.timeOffsetSecs > 0 ? "+" : ""}
                {Math.abs(form.timeOffsetSecs) >= 3600
                  ? `${(form.timeOffsetSecs / 3600).toFixed(1)} hour(s)`
                  : Math.abs(form.timeOffsetSecs) >= 60
                  ? `${(form.timeOffsetSecs / 60).toFixed(0)} minute(s)`
                  : `${form.timeOffsetSecs} second(s)`}
              </p>
            )}
            {form.sourcePath && form.destPath && (
              <button
                type="button"
                onClick={async () => {
                  try {
                    const offset = await tauriInvoke<number>("detect_time_offset", {
                      sourcePath: form.sourcePath,
                      destPath: form.destPath,
                    });
                    update("timeOffsetSecs", offset);
                  } catch (e: any) {
                    console.error("Time offset detection failed:", e);
                  }
                }}
                className="text-xs px-2 py-1 rounded border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-600 dark:text-zinc-400"
              >
                Auto-detect offset
              </button>
            )}
            <p className="text-[10px] text-zinc-400">
              Compensates for clock differences between local and remote. Prevents false change detection.
            </p>
          </div>
        </>
      )}

      <div className="flex gap-2 pt-2">
        <button
          onClick={onSubmit}
          disabled={loading || !form.name || !form.sourcePath || !form.destPath}
          className="flex-1 px-3 py-1.5 text-xs rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {loading ? "Creating..." : "Create Sync Pair"}
        </button>
        <button
          onClick={onCancel}
          className="px-3 py-1.5 text-xs rounded border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-800"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

function PreviewView({
  preview,
  onProceed,
  onCancel,
  onExport,
  loading,
}: {
  preview: SyncPreview;
  onProceed: () => void;
  onCancel: () => void;
  onExport: () => void;
  loading: boolean;
}) {
  const [expandedCategory, setExpandedCategory] = useState<string | null>(null);

  const categories = [
    {
      key: "additions",
      label: "New Files",
      count: preview.total_additions,
      items: preview.additions,
      color: "text-green-600",
    },
    {
      key: "modifications",
      label: "Modified",
      count: preview.total_modifications,
      items: preview.modifications,
      color: "text-blue-600",
    },
    {
      key: "deletions",
      label: "Deletions",
      count: preview.total_deletions,
      items: preview.deletions,
      color: "text-red-600",
    },
    {
      key: "conflicts",
      label: "Conflicts",
      count: preview.total_conflicts,
      items: preview.conflicts,
      color: "text-yellow-600",
    },
    {
      key: "skipped",
      label: "Skipped",
      count: preview.total_skipped,
      items: preview.skipped,
      color: "text-zinc-500",
    },
  ];

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Sync Preview: {preview.pair_name}
        </h3>
        <span className="text-[10px] text-zinc-400">
          Computed in {formatDuration(preview.duration_ms)}
        </span>
      </div>

      <div className="text-xs text-zinc-500">
        Total: {formatBytes(preview.total_bytes)} to transfer
      </div>

      {/* Category Summaries */}
      <div className="space-y-1">
        {categories.map((cat) => (
          <div key={cat.key}>
            <button
              onClick={() =>
                setExpandedCategory(
                  expandedCategory === cat.key ? null : cat.key
                )
              }
              className="flex items-center justify-between w-full px-2 py-1.5 text-xs rounded hover:bg-zinc-100 dark:hover:bg-zinc-800"
            >
              <span className={cat.color}>
                {expandedCategory === cat.key ? "v" : ">"} {cat.label}
              </span>
              <span className="font-mono">{cat.count}</span>
            </button>

            {expandedCategory === cat.key && cat.items.length > 0 && (
              <div className="ml-4 space-y-0.5 max-h-48 overflow-y-auto">
                {cat.items.map((item, idx) => (
                  <div
                    key={idx}
                    className="text-[11px] text-zinc-600 dark:text-zinc-400 flex items-center gap-2 py-0.5"
                  >
                    <span className="truncate flex-1" title={item.relative_path}>
                      {item.relative_path}
                    </span>
                    {item.source_size !== null && (
                      <span className="text-zinc-400 whitespace-nowrap">
                        {formatBytes(item.source_size)}
                      </span>
                    )}
                    {item.path_length_warning && (
                      <span className="text-yellow-500" title={item.path_length_warning}>
                        !
                      </span>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Conflict Details */}
      {preview.total_conflicts > 0 && (
        <div className="p-2 bg-yellow-50 dark:bg-yellow-900/20 rounded text-xs">
          <strong>{preview.total_conflicts} conflict(s)</strong> detected. These
          will be resolved using the configured conflict policy.
        </div>
      )}

      {/* Path Warnings */}
      {preview.path_warnings.length > 0 && (
        <div className="p-2 bg-orange-50 dark:bg-orange-900/20 rounded text-xs">
          <strong>{preview.path_warnings.length} path warning(s)</strong>: Some
          paths may exceed platform limits.
        </div>
      )}

      {/* Action Buttons */}
      <div className="flex gap-2 pt-2">
        <button
          onClick={onProceed}
          disabled={loading}
          className="flex-1 px-3 py-1.5 text-xs rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {loading ? "Syncing..." : "Proceed"}
        </button>
        <button
          onClick={onExport}
          className="px-3 py-1.5 text-xs rounded border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-800"
        >
          Export CSV
        </button>
        <button
          onClick={onCancel}
          className="px-3 py-1.5 text-xs rounded border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-800"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

function ReportsView({
  reports,
  onBack,
}: {
  reports: SyncReport[];
  onBack: () => void;
}) {
  const [exportError, setExportError] = useState<string | null>(null);

  const handleExportReportCsv = async (pairId: string, reportId: string, pairName: string) => {
    setExportError(null);
    try {
      const csv = await tauriInvoke<string>("export_sync_report_csv", { pairId, reportId });
      const blob = new Blob([csv], { type: "text/csv" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `sync-report-${pairName}-${reportId}.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e: any) {
      setExportError(e?.message || "CSV export failed");
    }
  };

  const handleExportReportJson = async (pairId: string, reportId: string, pairName: string) => {
    setExportError(null);
    try {
      const json = await tauriInvoke<string>("export_sync_report_json", { pairId, reportId });
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `sync-report-${pairName}-${reportId}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e: any) {
      setExportError(e?.message || "JSON export failed");
    }
  };
  if (reports.length === 0) {
    return (
      <div className="space-y-3">
        <button
          onClick={onBack}
          className="text-xs text-blue-500 hover:underline"
        >
          Back to list
        </button>
        <div className="text-center text-sm text-zinc-500 py-4">
          No sync reports available.
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Sync Reports
        </h3>
        <button
          onClick={onBack}
          className="text-xs text-blue-500 hover:underline"
        >
          Back
        </button>
      </div>

      {reports.map((report) => (
        <div
          key={report.id}
          className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3"
        >
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <span className={healthColor(report.health as SyncHealth)}>
                {healthIcon(report.health as SyncHealth)}
              </span>
              <span className="text-sm font-medium text-zinc-900 dark:text-zinc-100">
                {report.pair_name}
              </span>
            </div>
            <span className="text-xs text-zinc-500">
              {formatDuration(report.duration_ms)}
            </span>
          </div>

          <div className="grid grid-cols-2 gap-1 text-xs text-zinc-600 dark:text-zinc-400">
            <div>Added: {report.files_added}</div>
            <div>Modified: {report.files_modified}</div>
            <div>Deleted: {report.files_deleted}</div>
            <div>Skipped: {report.files_skipped}</div>
            <div>Conflicts: {report.conflicts_resolved}</div>
            <div>Errors: {report.errors}</div>
            <div className="col-span-2">
              Transferred: {formatBytes(report.bytes_transferred)}
            </div>
            <div>Status: {report.status}</div>
            {report.resumed && <div>Resumed from interruption</div>}
          </div>

          {report.error_messages.length > 0 && (
            <div className="mt-2 p-2 bg-red-50 dark:bg-red-900/20 rounded text-xs text-red-700 dark:text-red-300">
              {report.error_messages.map((msg, i) => (
                <div key={i}>{msg}</div>
              ))}
            </div>
          )}

          <div className="text-[10px] text-zinc-400 mt-2">
            {new Date(report.started_at).toLocaleString()} -{" "}
            {new Date(report.ended_at).toLocaleString()}
          </div>

          {/* Export buttons */}
          <div className="flex gap-1 mt-2">
            <button
              onClick={() => handleExportReportCsv(report.pair_id, report.id, report.pair_name)}
              className="px-2 py-1 text-[11px] rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700"
              aria-label={`Export CSV for report ${report.id}`}
            >
              Export CSV
            </button>
            <button
              onClick={() => handleExportReportJson(report.pair_id, report.id, report.pair_name)}
              className="px-2 py-1 text-[11px] rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700"
              aria-label={`Export JSON for report ${report.id}`}
            >
              Export JSON
            </button>
          </div>
        </div>
      ))}

      {exportError && (
        <div className="p-2 text-xs text-red-700 bg-red-50 dark:bg-red-900/20 dark:text-red-300 rounded" role="alert">
          {exportError}
        </div>
      )}
    </div>
  );
}

function ConflictsView({
  conflicts,
  onResolve,
  onBack,
}: {
  conflicts: SyncConflictItem[];
  onResolve: (id: string, resolution: string) => void;
  onBack: () => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Pending Conflicts ({conflicts.length})
        </h3>
        <button
          onClick={onBack}
          className="text-xs text-blue-500 hover:underline"
        >
          Back
        </button>
      </div>

      {conflicts.length === 0 && (
        <div className="text-center text-sm text-zinc-500 py-4">
          No pending conflicts.
        </div>
      )}

      {conflicts.map((item) => (
        <div
          key={item.id}
          className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3"
        >
          <div className="text-sm font-medium text-zinc-900 dark:text-zinc-100 mb-1">
            {item.relative_path}
          </div>

          <div className="grid grid-cols-2 gap-2 text-xs text-zinc-600 dark:text-zinc-400 mb-2">
            <div>
              <div className="font-semibold">Source</div>
              <div>Size: {formatBytes(item.source_size)}</div>
              {item.source_modified && (
                <div>Modified: {new Date(item.source_modified).toLocaleString()}</div>
              )}
            </div>
            <div>
              <div className="font-semibold">Destination</div>
              <div>Size: {formatBytes(item.dest_size)}</div>
              {item.dest_modified && (
                <div>Modified: {new Date(item.dest_modified).toLocaleString()}</div>
              )}
            </div>
          </div>

          <div className="flex gap-1 flex-wrap">
            {[
              { value: "source_wins", label: "Source Wins" },
              { value: "dest_wins", label: "Dest Wins" },
              { value: "newest_wins", label: "Newest Wins" },
              { value: "create_conflict_copy", label: "Conflict Copy" },
              { value: "skip", label: "Skip" },
            ].map((opt) => (
              <button
                key={opt.value}
                onClick={() => onResolve(item.id, opt.value)}
                className="px-2 py-1 text-[11px] rounded bg-zinc-100 dark:bg-zinc-800 hover:bg-zinc-200 dark:hover:bg-zinc-700"
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function QuarantineView({
  entries,
  onBack,
}: {
  entries: QuarantineEntry[];
  onBack: () => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
          Quarantine Queue ({entries.length})
        </h3>
        <button
          onClick={onBack}
          className="text-xs text-blue-500 hover:underline"
        >
          Back
        </button>
      </div>

      {entries.length === 0 && (
        <div className="text-center text-sm text-zinc-500 py-4">
          No quarantined files.
        </div>
      )}

      {entries.map((entry) => (
        <div
          key={entry.id}
          className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3 text-xs"
        >
          <div className="font-medium text-zinc-900 dark:text-zinc-100">
            {entry.original_path}
          </div>
          <div className="text-zinc-500 mt-1">
            Quarantined: {new Date(entry.quarantined_at).toLocaleString()}
          </div>
          <div className="text-zinc-500">
            Status: {entry.resolved ? "Resolved" : "Pending Review"}
          </div>
        </div>
      ))}
    </div>
  );
}

export default SyncPanel;
