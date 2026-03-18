/**
 * Sync Store (T-039..T-042)
 *
 * Zustand store for sync engine state management.
 * Provides reactive state for sync pairs, previews, reports,
 * conflicts, and quarantine.
 */
import { create } from "zustand";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

export interface SyncPair {
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
}

export interface SyncFilter {
  include_patterns: string[];
  exclude_patterns: string[];
  include_regex: string[];
  exclude_regex: string[];
  min_size: number;
  max_size: number;
  file_types: string[];
  exclude_file_types: string[];
}

export interface SyncPreview {
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

export interface SyncDiffEntry {
  relative_path: string;
  action: string;
  source_size: number | null;
  dest_size: number | null;
  source_modified: string | null;
  dest_modified: string | null;
  reason: string;
  path_length_warning: string | null;
}

export interface SyncReport {
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

export interface SyncConflictItem {
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

export interface QuarantineEntry {
  id: string;
  pair_id: string;
  original_path: string;
  quarantine_path: string;
  quarantined_at: string;
  resolved: boolean;
}

export type SyncHealth = "green" | "yellow" | "red" | "gray";

// ── Store ──

interface SyncState {
  // Data
  pairs: SyncPair[];
  healthMap: Record<string, SyncHealth>;
  currentPreview: SyncPreview | null;
  currentReports: SyncReport[];
  pendingConflicts: SyncConflictItem[];
  quarantineEntries: QuarantineEntry[];

  // UI state
  selectedPairId: string | null;
  loading: boolean;
  syncRunning: boolean;
  error: string | null;

  // Actions
  loadPairs: () => Promise<void>;
  createPair: (params: {
    name: string;
    sourcePath: string;
    destPath: string;
    mode: string;
    trigger: string;
    cronExpr?: string;
    filterJson?: string;
    conflictPolicy?: string;
    verifyMode?: string;
    checksumEnabled?: boolean;
  }) => Promise<SyncPair>;
  deletePair: (pairId: string) => Promise<void>;
  updatePair: (pairJson: string) => Promise<SyncPair>;
  runSync: (pairId: string) => Promise<SyncReport>;
  dryRun: (pairId: string) => Promise<SyncPreview>;
  exportPreviewCsv: (pairId: string) => Promise<string>;
  loadReports: (pairId: string) => Promise<void>;
  loadHealth: (pairId: string) => Promise<SyncHealth>;
  rollback: (pairId: string) => Promise<number>;
  resolveConflict: (conflictId: string, resolution: string) => Promise<void>;
  loadConflicts: () => Promise<void>;
  loadQuarantine: (pairId: string) => Promise<void>;
  startWatcher: (pairId: string) => Promise<void>;
  stopWatcher: (pairId: string) => Promise<void>;
  clearError: () => void;
  setSelectedPairId: (id: string | null) => void;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  // Initial state
  pairs: [],
  healthMap: {},
  currentPreview: null,
  currentReports: [],
  pendingConflicts: [],
  quarantineEntries: [],
  selectedPairId: null,
  loading: false,
  syncRunning: false,
  error: null,

  // Actions
  loadPairs: async () => {
    set({ loading: true, error: null });
    try {
      const pairs = await tauriInvoke<SyncPair[]>("list_sync_pairs");
      set({ pairs });

      // Load health for each pair
      const healthMap: Record<string, SyncHealth> = {};
      for (const pair of pairs) {
        try {
          healthMap[pair.id] = await tauriInvoke<SyncHealth>("get_sync_health", {
            pairId: pair.id,
          });
        } catch {
          healthMap[pair.id] = "gray";
        }
      }
      set({ healthMap });
    } catch (e: any) {
      set({ error: e?.message || "Failed to load sync pairs" });
    } finally {
      set({ loading: false });
    }
  },

  createPair: async (params) => {
    set({ loading: true, error: null });
    try {
      const pair = await tauriInvoke<SyncPair>("create_sync_pair", {
        name: params.name,
        sourcePath: params.sourcePath,
        destPath: params.destPath,
        mode: params.mode,
        trigger: params.trigger,
        cronExpr: params.cronExpr || null,
        filterJson: params.filterJson || null,
        conflictPolicy: params.conflictPolicy || null,
        verifyMode: params.verifyMode || null,
        checksumEnabled: params.checksumEnabled || false,
      });
      await get().loadPairs();
      return pair;
    } catch (e: any) {
      set({ error: e?.message || "Failed to create sync pair" });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  deletePair: async (pairId) => {
    try {
      await tauriInvoke("delete_sync_pair", { pairId });
      await get().loadPairs();
    } catch (e: any) {
      set({ error: e?.message || "Failed to delete sync pair" });
      throw e;
    }
  },

  updatePair: async (pairJson) => {
    try {
      const pair = await tauriInvoke<SyncPair>("update_sync_pair", { pairJson });
      await get().loadPairs();
      return pair;
    } catch (e: any) {
      set({ error: e?.message || "Failed to update sync pair" });
      throw e;
    }
  },

  runSync: async (pairId) => {
    set({ syncRunning: true, error: null });
    try {
      const report = await tauriInvoke<SyncReport>("run_sync", { pairId });
      set({ currentReports: [report, ...get().currentReports] });
      await get().loadPairs();
      return report;
    } catch (e: any) {
      set({ error: e?.message || "Sync failed" });
      throw e;
    } finally {
      set({ syncRunning: false });
    }
  },

  dryRun: async (pairId) => {
    set({ loading: true, error: null });
    try {
      const preview = await tauriInvoke<SyncPreview>("sync_dry_run", { pairId });
      set({ currentPreview: preview });
      return preview;
    } catch (e: any) {
      set({ error: e?.message || "Dry run failed" });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  exportPreviewCsv: async (pairId) => {
    try {
      return await tauriInvoke<string>("export_sync_preview_csv", { pairId });
    } catch (e: any) {
      set({ error: e?.message || "Export failed" });
      throw e;
    }
  },

  loadReports: async (pairId) => {
    try {
      const reports = await tauriInvoke<SyncReport[]>("get_sync_reports", { pairId });
      set({ currentReports: reports });
    } catch (e: any) {
      set({ error: e?.message || "Failed to load reports" });
    }
  },

  loadHealth: async (pairId) => {
    try {
      const health = await tauriInvoke<SyncHealth>("get_sync_health", { pairId });
      set({
        healthMap: { ...get().healthMap, [pairId]: health },
      });
      return health;
    } catch {
      return "gray";
    }
  },

  rollback: async (pairId) => {
    set({ loading: true, error: null });
    try {
      const restored = await tauriInvoke<number>("rollback_sync", { pairId });
      await get().loadPairs();
      return restored;
    } catch (e: any) {
      set({ error: e?.message || "Rollback failed" });
      throw e;
    } finally {
      set({ loading: false });
    }
  },

  resolveConflict: async (conflictId, resolution) => {
    try {
      await tauriInvoke("resolve_sync_conflict", { conflictId, resolution });
      await get().loadConflicts();
    } catch (e: any) {
      set({ error: e?.message || "Failed to resolve conflict" });
      throw e;
    }
  },

  loadConflicts: async () => {
    try {
      const conflicts = await tauriInvoke<SyncConflictItem[]>("get_sync_conflicts");
      set({ pendingConflicts: conflicts });
    } catch (e: any) {
      set({ error: e?.message || "Failed to load conflicts" });
    }
  },

  loadQuarantine: async (pairId) => {
    try {
      const entries = await tauriInvoke<QuarantineEntry[]>("get_quarantine_entries", {
        pairId,
      });
      set({ quarantineEntries: entries });
    } catch (e: any) {
      set({ error: e?.message || "Failed to load quarantine" });
    }
  },

  startWatcher: async (pairId) => {
    try {
      await tauriInvoke("start_sync_watcher", { pairId });
    } catch (e: any) {
      set({ error: e?.message || "Failed to start watcher" });
      throw e;
    }
  },

  stopWatcher: async (pairId) => {
    try {
      await tauriInvoke("stop_sync_watcher", { pairId });
    } catch (e: any) {
      set({ error: e?.message || "Failed to stop watcher" });
      throw e;
    }
  },

  clearError: () => set({ error: null }),
  setSelectedPairId: (id) => set({ selectedPairId: id }),
}));
