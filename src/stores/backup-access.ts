/**
 * Reading backups without restoring them (Kopia-parity WP5): pick a version
 * by time, read / preview / export one file, the local volume cache and
 * "Open as drive" (a read-only WebDAV drive of a version). Wire types mirror
 * `src-tauri/src/backup_engine/{access,drives}.rs` and `ufop_core::backup::cache`.
 */
import { create } from "zustand";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import type { BackupOperation, Fileset, OperationResult, RestoreOptions, RestoreOverrides, RestoreReport, VersionSelector } from "./backup-store";

export interface ResolvedVersion {
  index: number;
  fileset: Fileset;
}

export interface FileChunk {
  path: string;
  version: number;
  timestamp: string;
  size: number;
  offset: number;
  length: number;
  /** base64 */
  data: string;
  truncated: boolean;
  mime_type: string;
}

export interface ExportedFile {
  path: string;
  local_path: string;
  bytes: number;
  version: number;
  timestamp: string;
}

export interface CacheSettings {
  /** null: the app's cache folder. */
  dir: string | null;
  /** 0 = cache off. */
  max_bytes: number;
  use_for_restore: boolean;
}

export interface CacheStats {
  dir: string;
  max_bytes: number;
  entries: number;
  bytes: number;
  hits: number;
  misses: number;
  corrupt: number;
  evictions: number;
  hit_rate: number;
}

export interface DriveRequest {
  version: number | null;
  selector: VersionSelector | null;
  all_versions: boolean;
  source: string | null;
  /** Mount with the OS's WebDAV client (default true). */
  mount: boolean | null;
  port: number | null;
  idle_timeout_minutes: number | null;
}

export interface DriveInfo {
  id: string;
  job_id: string;
  job_name: string;
  url: string;
  username: string;
  password: string;
  version: number | null;
  timestamp: string | null;
  all_versions: boolean;
  mount_point: string | null;
  mounted_with: string | null;
  mount_error: string | null;
  started_at: string;
  idle_timeout_minutes: number;
}

export const DEFAULT_CACHE_SETTINGS: CacheSettings = { dir: null, max_bytes: 2 * 1024 ** 3, use_for_restore: false };

/** Operation name of the confirmation token a mirror restore needs (Rust: MIRROR_RESTORE_CONFIRMATION). */
export const MIRROR_RESTORE_CONFIRMATION = "backup_mirror_restore";

/** Which args a version reference sends: a selector, or a plain number. */
export function versionArgs(v: number | VersionSelector): { version: number | null; selector: VersionSelector | null } {
  return typeof v === "number" ? { version: v, selector: null } : { version: null, selector: v };
}

export const resolveVersion = (id: string, selector: VersionSelector, source: string | null = null) =>
  tauriInvoke<ResolvedVersion>("backup_resolve_version", { id, selector, source });

export const readBackupFile = (id: string, path: string, version: number | VersionSelector, offset = 0, length: number | null = null, source: string | null = null) =>
  tauriInvoke<FileChunk>("backup_read_file", { id, path, ...versionArgs(version), offset, length, source });

export const exportBackupFile = (id: string, path: string, dest: string, version: number | VersionSelector, source: string | null = null) =>
  tauriInvoke<ExportedFile>("backup_export_file", { id, path, dest, ...versionArgs(version), source });

export const previewBackupFile = (id: string, path: string, version: number | VersionSelector, source: string | null = null) =>
  tauriInvoke<ExportedFile>("backup_preview_file", { id, path, ...versionArgs(version), source });

const EMPTY_STATS: CacheStats = { dir: "", max_bytes: 0, entries: 0, bytes: 0, hits: 0, misses: 0, corrupt: 0, evictions: 0, hit_rate: 0 };

export const cacheStats = () => (isBackendAvailable() ? tauriInvoke<CacheStats>("backup_cache_stats", {}) : Promise.resolve(EMPTY_STATS));
export const clearCache = () => tauriInvoke<number>("backup_clear_cache", {});
export const getCacheSettings = () =>
  isBackendAvailable() ? tauriInvoke<CacheSettings>("backup_get_cache_settings", {}) : Promise.resolve(DEFAULT_CACHE_SETTINGS);
export const setCacheSettings = (settings: CacheSettings) => tauriInvoke<CacheStats>("backup_set_cache_settings", { settings });

/** Run a restore; `confirmationToken` is required when a mirror restore deletes (`overrides.confirm_delete`). */
export function executeRestore(id: string, options: RestoreOptions, overrides: RestoreOverrides, confirmationToken: string | null = null) {
  const operation: BackupOperation = { type: "restore", options, overrides };
  return tauriInvoke<OperationResult>("backup_execute", { id, operation, confirmationToken });
}

/** The report of a restore result. */
export const restoreReport = (r: OperationResult) => r.report as unknown as RestoreReport;

/** Ask the backend for a single-use confirmation token for a mirror restore. */
export async function mirrorConfirmationToken(details: string): Promise<string> {
  const t = await tauriInvoke<{ token: string }>("request_destructive_confirmation", { operation: MIRROR_RESTORE_CONFIRMATION, details });
  return t.token;
}

/** "12 files and 2 folders" for a mirror deletion list (folders end in "/"). */
export function deletionSummary(paths: string[]): string {
  const folders = paths.filter((p) => p.endsWith("/") || p.endsWith("\\")).length;
  const files = paths.length - folders;
  const part = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  if (folders === 0) return part(files, "file", "files");
  if (files === 0) return part(folders, "folder", "folders");
  return `${part(files, "file", "files")} and ${part(folders, "folder", "folders")}`;
}

/** File name for an archive export of a version. */
export function archiveFileName(jobName: string, timestamp: string, ext = "zip"): string {
  const safe = jobName.replace(/[^\w.-]+/g, "_").replace(/^_+|_+$/g, "") || "backup";
  const stamp = timestamp.replace(/[:]/g, "-").replace(/\+.*$|Z$/, "").replace("T", "_");
  return `${safe}-${stamp}.${ext}`;
}

// ── Open drives ──

interface DrivesState {
  drives: DriveInfo[];
  load: () => Promise<void>;
  open: (jobId: string, request: Partial<DriveRequest>) => Promise<DriveInfo>;
  close: (driveId: string) => Promise<void>;
}

export const useBackupDrives = create<DrivesState>((set, get) => ({
  drives: [],
  load: async () => {
    if (!isBackendAvailable()) return;
    // The list is a convenience: a backend without drives (ufop-server) has none.
    const drives = await tauriInvoke<DriveInfo[]>("backup_list_drives", {}).catch(() => [] as DriveInfo[]);
    set({ drives: drives ?? [] });
  },
  open: async (jobId, request) => {
    const full: DriveRequest = { version: null, selector: null, all_versions: false, source: null, mount: null, port: null, idle_timeout_minutes: null, ...request };
    const d = await tauriInvoke<DriveInfo>("backup_open_drive", { id: jobId, request: full });
    await get().load();
    return d;
  },
  close: async (driveId) => {
    await tauriInvoke<boolean>("backup_close_drive", { driveId });
    await get().load();
  },
}));
