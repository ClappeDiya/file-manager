/**
 * Backup Store
 *
 * Zustand store for versioned (deduplicated, encrypted) backup jobs.
 * Types mirror the Rust serde output of `backup_engine` and
 * `ufop_core::backup` exactly (snake_case, `T | null` for Option<T>,
 * `type`-tagged unions for tagged enums).
 */
import { create } from "zustand";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

export { errorText };

// ── Types (mirror Rust) ──

export type FilterAction = "include" | "exclude";
export type FilterKind = "exact" | "wildcard" | "regex" | "group";

export interface FilterRule {
  action: FilterAction;
  kind: FilterKind;
  pattern: string;
}

export interface SourceFilter {
  rules: FilterRule[];
  exclude_hidden: boolean;
  exclude_system: boolean;
  exclude_temporary: boolean;
  max_file_size: number | null;
  exclusion_markers: string[];
  exclude_empty_folders: boolean;
  exclude_cloud_placeholders: boolean;
}

export type BackupDestination =
  | { type: "local"; path: string; alternate_paths: string[]; marker_file: string | null }
  | { type: "connection"; connection_id: string; remote_path: string }
  | { type: "library"; destination_id: string; remote_path: string };

/** Named destination shared by several jobs. */
export interface SavedDestination {
  id: string;
  name: string;
  destination: Exclude<BackupDestination, { type: "library" }>;
  created_at: string;
}

/** Defaults applied to every option a job leaves at the built-in value. */
export interface GlobalDefaults {
  options: Partial<BackupOptions>;
  throttle: Throttle | null;
  retry: RetryPolicy | null;
}

export type SymlinkPolicy = "store" | "follow" | "ignore";
export type HardlinkPolicy = "all" | "first" | "none";
export type HashAlgorithm = "sha256" | "sha512";

export interface BackupOptions {
  prefix: string;
  block_size: number;
  volume_size: number;
  compression_level: number;
  no_compress_extensions: string[];
  block_hash: HashAlgorithm;
  file_hash: HashAlgorithm;
  symlink_policy: SymlinkPolicy;
  hardlink_policy: HardlinkPolicy;
  missing_source: "fail" | "warn";
  allow_empty_source: boolean;
  disable_timestamp_check: boolean;
  timestamps_only: boolean;
  skip_metadata: boolean;
  upload_unchanged_backups: boolean;
  verify_samples: number;
  auto_compact: boolean;
  compact_threshold_percent: number;
  upload_verification_file: boolean;
  dry_run: boolean;
  task_config: unknown | null;
  allow_full_removal: boolean;
  ignore_exclusion_xattrs: boolean;
  respect_advisory_locks: boolean;
  changed_paths: string[] | null;
  deleted_paths: string[];
  usn_policy: UsnPolicy;
  backup_xattrs: boolean;
  backup_acls: boolean;
  backup_alternate_streams: boolean;
  volume_format: VolumeFormat;
  zip_method: ZipMethod;
  control_files: string[];
  par2: Par2Options;
  hash_threads: number;
  upload_concurrency: number;
  cpu_intensity: number;
  temp_dir: string | null;
  read_buffer_size: number;
  background_io: boolean;
  quota_size: number | null;
  quota_warning_percent: number;
  quota_disable: boolean;
  auto_compact_interval_days: number | null;
  auto_vacuum_interval_days: number | null;
  remote_case_insensitive: boolean;
  snapshot_policy: SnapshotPolicy;
  snapshot: SnapshotOptions;
}

export type SnapshotPolicy = "off" | "auto" | "on" | "required";

export interface SnapshotOptions {
  lvm_size: string;
  vss_provider: string | null;
  vss_exclude_writers: string[];
  mount_root: string | null;
}

export interface SnapshotDiagnostics {
  method: string;
  ok: boolean;
  mappings: [string, string][];
  steps: string[];
  error: string | null;
}

export type UsnPolicy = "off" | "auto" | "on" | "required";
export type VolumeFormat = "ufop" | "zip";
export type ZipMethod = "deflate" | "stored" | "bzip2" | "zstd";

export interface Par2Options {
  enabled: boolean;
  redundancy_percent: number;
  binary: string;
}

export type RetentionPolicy =
  | { type: "keep_all" }
  | { type: "keep_days"; days: number }
  | { type: "keep_count"; count: number }
  | { type: "smart" }
  | { type: "custom"; rules: string };

export interface BackupSchedule {
  enabled: boolean;
  start_time: string;
  interval_minutes: number;
  weekdays: number[];
  skip_on_battery: boolean;
}

export interface ScriptConfig {
  run_before: string | null;
  run_before_required: boolean;
  run_after: string | null;
  run_on_source_release: string | null;
  timeout_secs: number;
  result_format: string;
}

export interface ReportConfig {
  http_urls: string[];
  http_method: string;
  format: "json" | "form";
  send_on: string[];
  retries: number;
  accept_invalid_certs: boolean;
  message_template: string;
  extra_fields: Record<string, string>;
  machine_name: string | null;
  backup_id: string | null;
  max_log_lines: number;
}

export interface Throttle {
  upload_bps: number;
  download_bps: number;
}

export interface RetryPolicy {
  retries: number;
  delay_ms: number;
  exponential: boolean;
  max_delay_ms: number;
}

export interface JobStats {
  last_run_at: string | null;
  last_result: string | null;
  last_duration_ms: number | null;
  last_error: string | null;
  last_scheduled_at: string | null;
  version_count: number;
  source_size: number;
  backend_size: number;
}

export interface BackupJob {
  id: string;
  name: string;
  description: string;
  sources: string[];
  filter: SourceFilter;
  destination: BackupDestination;
  encrypted: boolean;
  options: BackupOptions;
  retention: RetentionPolicy;
  schedule: BackupSchedule;
  scripts: ScriptConfig;
  reports: ReportConfig;
  throttle: Throttle;
  retry: RetryPolicy;
  soft_delete_prefix: string | null;
  store_config_remotely: boolean;
  remote_config_include_secrets: boolean;
  network_timeout_secs: number;
  prevent_sleep: boolean;
  notify_on: string[];
  created_at: string;
  updated_at: string;
  stats: JobStats;
}

export interface Fileset {
  id: number;
  timestamp: string;
  volume_name: string;
  partial: boolean;
  file_count: number;
  total_size: number;
}

export type EntryKind = "file" | "dir" | "symlink";

export interface BackupFileEntry {
  path: string;
  kind: EntryKind;
  size: number;
  mtime: number | null;
  mode: number | null;
  hash: string | null;
  blocks: string[];
  symlink_target: string | null;
}

export interface FoundFile {
  version: number;
  timestamp: string;
  path: string;
  kind: EntryKind;
  size: number;
  mtime: number | null;
  hash: string | null;
}

export interface VersionComparison {
  base: string;
  compare: string;
  added: string[];
  deleted: string[];
  modified: string[];
}

export type ConflictPolicy = "overwrite" | "rename" | "skip";

export interface RestoreOptions {
  version: number;
  versions: number[];
  unique_versions: boolean;
  paths: string[];
  target: string | null;
  preserve_full_paths: boolean;
  conflict: ConflictPolicy;
  restore_permissions: boolean;
  restore_symlinks: boolean;
  restore_metadata: boolean;
  skip_verify: boolean;
  dry_run: boolean;
  use_local_blocks: boolean;
  download_threads: number;
  file_threads: number;
  legacy: boolean;
  temp_dir: string | null;
  skip_free_space_check: boolean;
}

export interface AutoTuneResult {
  download_threads: number;
  file_threads: number;
  measurements: [number, number][];
  sample_bytes: number;
}

export interface RestoreReport {
  version: string;
  files_restored: number;
  folders_created: number;
  files_skipped_matching: number;
  files_skipped_conflict: number;
  files_skipped_duplicate: number;
  bytes_restored: number;
  volumes_downloaded: number;
  blocks_from_local: number;
  planned: string[];
  warnings: string[];
  errors: string[];
}

export type Phase =
  | "preparing"
  | "scanning"
  | "uploading"
  | "finalizing"
  | "retention"
  | "compacting"
  | "verifying"
  | "downloading"
  | "restoring"
  | "completed";

export interface Progress {
  phase: Phase;
  files_processed: number;
  files_total: number;
  bytes_processed: number;
  current_path: string | null;
}

export interface TaskInfo {
  task_id: string;
  job_id: string;
  job_name: string;
  operation: string;
  state: "queued" | "running";
  queued_at: string;
  started_at: string | null;
  progress: Progress | null;
}

export interface NextRun {
  job_id: string;
  job_name: string;
  next_run_at: string;
}

export interface SchedulerStatus {
  running: boolean;
  paused: boolean;
  paused_until: string | null;
  tasks: TaskInfo[];
  next_runs: NextRun[];
}

export type LogLevel = "info" | "warning" | "error";

export interface LogLine {
  level: LogLevel;
  message: string;
}

export interface BackupRun {
  id: string;
  job_id: string;
  operation: string;
  status: "success" | "warning" | "error" | "cancelled";
  started_at: string;
  finished_at: string | null;
  summary: string;
  report: Record<string, unknown>;
  log: LogLine[];
}

export interface OperationResult {
  run_id: string;
  status: string;
  summary: string;
  report: Record<string, unknown>;
  log: LogLine[];
}

/** One-off settings for a single restore. */
export interface RestoreOverrides {
  download_bps: number | null;
  retries: number | null;
  network_timeout_secs: number | null;
}

export interface DbInfo {
  path: string;
  schema_version: number;
  latest_schema: number;
  versions: number;
  volumes: number;
  blocks: number;
  files: number;
  size_bytes: number;
  integrity_ok: boolean;
  integrity: string;
}

export interface DatabaseEntry {
  job_id: string;
  job_name: string | null;
  info: DbInfo | null;
  error: string | null;
}

export interface DatabaseCleanupReport {
  orphans_deleted: string[];
  vacuumed: number;
  migrated: number;
  bytes_before: number;
  bytes_after: number;
  errors: string[];
}

export interface RecoveryDownloadReport {
  downloaded: number;
  skipped_existing: number;
  bytes: number;
  failed: string[];
}

export interface RecoveryIndexReport {
  block_volumes: number;
  blocks: number;
  damaged: string[];
}

export interface RecoveryVersion {
  version: number;
  timestamp: string;
  partial: boolean;
  files: number;
  volume: string;
}

export interface RecoveryRestoreReport {
  version: string;
  files_restored: number;
  files_partial: string[];
  folders_created: number;
  bytes_written: number;
  errors: string[];
}

export interface RecoveryConvertReport {
  block_volumes: number;
  file_lists: number;
  bytes_uploaded: number;
  skipped: string[];
}

export type BackupOperation =
  | { type: "restore"; options: RestoreOptions; overrides?: RestoreOverrides }
  | { type: "verify"; full: boolean }
  | { type: "compact" }
  | { type: "repair" }
  | { type: "recreate_database" }
  | { type: "list_broken" }
  | { type: "purge_broken"; dry_run: boolean }
  | { type: "purge"; paths: string[]; versions: number[] | null; dry_run: boolean }
  | { type: "delete_versions"; versions: number[]; allow_full_removal: boolean }
  | { type: "consistency" }
  | { type: "rebuild_missing" }
  | { type: "extract_control_files"; version: number; target: string }
  | { type: "auto_tune"; sample_volumes: number }
  | { type: "snapshot_test" };

export interface PassphraseStrength {
  score: number;
  label: string;
  feedback: string[];
}

export interface FilterGroupInfo {
  name: string;
  description: string;
  patterns: string[];
}

export interface BackupCapabilities {
  filter_groups: FilterGroupInfo[];
  connection_protocols: string[];
  default_no_compress_extensions: string[];
}

export interface SourcePreview {
  files: number;
  folders: number;
  total_size: number;
  sample: string[];
  warnings: string[];
}

export interface DesktopSettings {
  run_in_background: boolean;
  start_at_login: boolean;
}

export interface ImportPreview {
  job: BackupJob;
  has_passphrase: boolean;
}

// ── Defaults (mirror Rust `Default` impls) ──

export const DEFAULT_NO_COMPRESS = [
  "7z", "zip", "gz", "tgz", "bz2", "xz", "zst", "rar", "jpg", "jpeg", "png", "gif", "webp",
  "heic", "mp3", "mp4", "m4a", "m4v", "mkv", "mov", "avi", "aac", "ogg", "flac", "pdf",
  "docx", "xlsx", "pptx", "odt", "jar", "apk", "dmg", "iso",
];

export function newBackupJob(): BackupJob {
  const now = new Date().toISOString();
  return {
    id: "",
    name: "",
    description: "",
    sources: [],
    filter: {
      rules: [],
      exclude_hidden: false,
      exclude_system: false,
      exclude_temporary: false,
      max_file_size: null,
      exclusion_markers: [],
      exclude_empty_folders: false,
      exclude_cloud_placeholders: false,
    },
    destination: { type: "local", path: "", alternate_paths: [], marker_file: null },
    encrypted: true,
    options: {
      prefix: "ufop",
      block_size: 100 * 1024,
      volume_size: 50 * 1024 * 1024,
      compression_level: 6,
      no_compress_extensions: [...DEFAULT_NO_COMPRESS],
      block_hash: "sha256",
      file_hash: "sha256",
      symlink_policy: "store",
      hardlink_policy: "all",
      missing_source: "fail",
      allow_empty_source: false,
      disable_timestamp_check: false,
      timestamps_only: false,
      skip_metadata: false,
      upload_unchanged_backups: false,
      verify_samples: 1,
      auto_compact: true,
      compact_threshold_percent: 25,
      upload_verification_file: false,
      dry_run: false,
      task_config: null,
      allow_full_removal: false,
      ignore_exclusion_xattrs: false,
      respect_advisory_locks: false,
      changed_paths: null,
      deleted_paths: [],
      usn_policy: "off",
      backup_xattrs: true,
      backup_acls: true,
      backup_alternate_streams: false,
      volume_format: "ufop",
      zip_method: "deflate",
      control_files: [],
      par2: { enabled: false, redundancy_percent: 10, binary: "par2" },
      hash_threads: 0,
      upload_concurrency: 2,
      cpu_intensity: 10,
      temp_dir: null,
      read_buffer_size: 0,
      background_io: false,
      quota_size: null,
      quota_warning_percent: 10,
      quota_disable: false,
      auto_compact_interval_days: null,
      auto_vacuum_interval_days: null,
      remote_case_insensitive: false,
      snapshot_policy: "off",
      snapshot: { lvm_size: "10%ORIGIN", vss_provider: null, vss_exclude_writers: [], mount_root: null },
    },
    retention: { type: "keep_all" },
    schedule: { enabled: false, start_time: "02:00", interval_minutes: 1440, weekdays: [], skip_on_battery: false },
    scripts: { run_before: null, run_before_required: false, run_after: null, run_on_source_release: null, timeout_secs: 60, result_format: "json" },
    reports: {
      http_urls: [],
      http_method: "POST",
      format: "json",
      send_on: ["warning", "error"],
      retries: 3,
      accept_invalid_certs: false,
      message_template: "{{operation}} of {{name}} on {{machine}}: {{result}}",
      extra_fields: {},
      machine_name: null,
      backup_id: null,
      max_log_lines: 100,
    },
    throttle: { upload_bps: 0, download_bps: 0 },
    retry: { retries: 3, delay_ms: 2000, exponential: true, max_delay_ms: 60000 },
    soft_delete_prefix: null,
    store_config_remotely: false,
    remote_config_include_secrets: false,
    network_timeout_secs: 0,
    prevent_sleep: true,
    notify_on: ["warning", "error"],
    created_at: now,
    updated_at: now,
    stats: {
      last_run_at: null,
      last_result: null,
      last_duration_ms: null,
      last_error: null,
      last_scheduled_at: null,
      version_count: 0,
      source_size: 0,
      backend_size: 0,
    },
  };
}

export const DEFAULT_RESTORE_OPTIONS: RestoreOptions = {
  version: 0,
  versions: [],
  unique_versions: false,
  paths: [],
  target: null,
  preserve_full_paths: false,
  conflict: "overwrite",
  restore_permissions: false,
  restore_symlinks: true,
  restore_metadata: true,
  skip_verify: false,
  dry_run: false,
  use_local_blocks: false,
  download_threads: 0,
  file_threads: 0,
  legacy: false,
  temp_dir: null,
  skip_free_space_check: false,
};


const EMPTY_STATUS: SchedulerStatus = { running: false, paused: false, paused_until: null, tasks: [], next_runs: [] };

/**
 * IPC call that uses `fallback` only without a backend (browser dev mode). With
 * Tauri or `ufop-server`, backend errors propagate so the panel can show them instead of an
 * empty list that looks like "no data".
 */
function ipc<T>(command: string, args: Record<string, unknown>, fallback: T): Promise<T> {
  return isBackendAvailable() ? tauriInvoke<T>(command, args) : Promise.resolve(fallback);
}

// ── Store ──

interface BackupState {
  jobs: BackupJob[];
  status: SchedulerStatus;
  history: BackupRun[];
  capabilities: BackupCapabilities | null;
  loading: boolean;
  error: string | null;

  loadJobs: () => Promise<void>;
  loadStatus: () => Promise<void>;
  loadHistory: (jobId?: string | null, limit?: number) => Promise<void>;
  loadCapabilities: () => Promise<void>;
  saveJob: (job: BackupJob, passphrase?: string | null) => Promise<BackupJob>;
  deleteJob: (id: string, deleteLocalDb: boolean, deleteRemote: boolean) => Promise<void>;
  runJob: (id: string, dryRun?: boolean) => Promise<string>;
  cancel: (id: string) => Promise<void>;
  execute: (id: string, operation: BackupOperation) => Promise<OperationResult>;
  pause: (minutes: number | null) => Promise<void>;
  resume: () => Promise<void>;
  clearError: () => void;
}

export const useBackupStore = create<BackupState>((set, get) => ({
  jobs: [],
  status: EMPTY_STATUS,
  history: [],
  capabilities: null,
  loading: false,
  error: null,

  loadJobs: async () => {
    set({ loading: true });
    try {
      const jobs = await ipc<BackupJob[]>("backup_list_jobs", {}, []);
      set({ jobs, loading: false, error: null });
    } catch (e) {
      set({ loading: false, error: errorText(e) });
    }
  },

  loadStatus: async () => {
    try {
      const status = await ipc<SchedulerStatus>("backup_status", {}, EMPTY_STATUS);
      set({ status });
    } catch (e) {
      set({ error: errorText(e) });
    }
  },

  loadHistory: async (jobId = null, limit = 50) => {
    try {
      const history = await ipc<BackupRun[]>("backup_run_history", { jobId, limit }, []);
      set({ history });
    } catch (e) {
      set({ error: errorText(e) });
    }
  },

  loadCapabilities: async () => {
    if (get().capabilities) return;
    const capabilities = await ipc<BackupCapabilities>(
      "backup_capabilities",
      {},
      { filter_groups: [], connection_protocols: [], default_no_compress_extensions: DEFAULT_NO_COMPRESS },
    );
    set({ capabilities });
  },

  saveJob: async (job, passphrase) => {
    const saved = await tauriInvoke<BackupJob>("backup_save_job", {
      job,
      passphrase: passphrase === undefined ? null : passphrase,
    });
    await get().loadJobs();
    return saved;
  },

  deleteJob: async (id, deleteLocalDb, deleteRemote) => {
    await tauriInvoke<void>("backup_delete_job", { id, deleteLocalDb, deleteRemote });
    await get().loadJobs();
  },

  runJob: async (id, dryRun = false) => {
    const taskId = await tauriInvoke<string>("backup_run", { id, dryRun });
    await get().loadStatus();
    return taskId;
  },

  cancel: async (id) => {
    await tauriInvoke<boolean>("backup_cancel", { id });
    await get().loadStatus();
  },

  execute: async (id, operation) => {
    const result = await tauriInvoke<OperationResult>("backup_execute", { id, operation });
    await Promise.all([get().loadJobs(), get().loadStatus()]);
    return result;
  },

  pause: async (minutes) => {
    const status = await tauriInvoke<SchedulerStatus>("backup_pause", { minutes });
    set({ status });
  },

  resume: async () => {
    const status = await tauriInvoke<SchedulerStatus>("backup_resume", {});
    set({ status });
  },

  clearError: () => set({ error: null }),
}));

/** Format a schedule for display. */
export function describeSchedule(s: BackupSchedule): string {
  if (!s.enabled) return "Manual only";
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const every =
    s.interval_minutes % 1440 === 0
      ? s.interval_minutes === 1440
        ? "daily"
        : `every ${s.interval_minutes / 1440} days`
      : s.interval_minutes % 60 === 0
        ? `every ${s.interval_minutes / 60} h`
        : `every ${s.interval_minutes} min`;
  const on = s.weekdays.length === 0 || s.weekdays.length === 7 ? "" : ` on ${s.weekdays.map((d) => days[d]).join(", ")}`;
  return `${every} from ${s.start_time}${on}`;
}

export function describeRetention(r: RetentionPolicy): string {
  switch (r.type) {
    case "keep_all":
      return "Keep all versions";
    case "keep_days":
      return `Keep ${r.days} day(s)`;
    case "keep_count":
      return `Keep ${r.count} version(s)`;
    case "smart":
      return "Smart (1W:1D, 4W:1W, 12M:1M)";
    case "custom":
      return `Custom: ${r.rules}`;
  }
}
