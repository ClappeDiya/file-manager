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
  /** Ignore-file names read in every folder, gitignore syntax (default `.ufopignore`). */
  ignore_files: string[];
  /** Also honour `.gitignore` and `.git/info/exclude`. */
  respect_gitignore: boolean;
}

export type BackupDestination =
  | { type: "local"; path: string; alternate_paths: string[]; marker_file: string | null }
  | { type: "connection"; connection_id: string; remote_path: string }
  | { type: "library"; destination_id: string; remote_path: string }
  | { type: "cloud"; config: CloudConfig; credential_id: string };

/** Provider settings; `provider` selects the Rust `CloudConfig` variant. */
export type CloudConfig = { provider: string } & Record<string, unknown>;

export interface CloudProviderInfo {
  id: string;
  name: string;
  secret: string;
  verified: "emulator" | "real-server" | "cli" | "api-mock";
  hidden: boolean;
}

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
  /** Machine-wide caps (all jobs together). */
  caps: TransferCaps;
}

/** Mirrors `defaults::TransferCaps` (0 = unlimited / one at a time). */
export interface TransferCaps {
  upload_bps: number;
  download_bps: number;
  max_concurrent_runs: number;
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
  /** Volume encryption for a new repository (existing ones keep theirs). */
  encryption_module: "aes" | "gpg";
  gpg: GpgOptions;
  /** Argon2id cost for a new AES repository; null = app default. */
  kdf: KdfParams | null;
  object_lock: ObjectLockOptions | null;
  /** What `photos://` (macOS Photos library) sources export. */
  photos: PhotosOptions;
  // ── Repository format: new repositories only (existing ones keep theirs) ──
  /** 1 or 2; null = newest (2). */
  format_version: number | null;
  /** null = FastCDC on format 2 (format 1 is always fixed). */
  chunker: ChunkerKind | null;
  /** null = zstd on format 2 (format 1 is always deflate). */
  block_codec: BlockCodec | null;
  /** null = zstd on format 2 (format 1 never compresses metadata). */
  metadata_compression: MetadataCompression | null;
  /** Format 2 encrypted repositories only. */
  cipher: CipherKind;
  /** Format 2 only; sharded needs a destination with folders. */
  layout: RepoLayout;
  // ── Repository safety ──
  /** Repository lock lease in seconds, renewed every third of it (0 = 30 min). */
  lock_ttl_secs: number;
  /** Warn when the destination clock differs by this many seconds (0 = never). */
  clock_skew_warn_secs: number;
  /** Refuse to write when the clocks differ by this many seconds (0 = never). */
  clock_skew_abort_secs: number;
  /** Checkpoint (partial version) at most this often during a backup (0 = off). */
  checkpoint_interval_secs: number;
  /** Also checkpoint after this many uploaded bytes (0 = off). */
  checkpoint_interval_bytes: number;
  // ── Versions and shared repositories ──
  /** Note stored with each new version. */
  default_note: string | null;
  /** Tags stored with each new version. */
  default_tags: Record<string, string>;
  /** Who writes the versions; empty host / user = this computer's (override to continue another computer's history). */
  identity: ClientIdentity;
  /** Create a new (format 2) repository as shared by several jobs / computers. */
  shared_repository: boolean;
  /** Shared repositories: compaction leaves volumes younger than this alone. */
  shared_gc_delay_hours: number;
  // ── Sources and runs ──
  /** Do not descend into folders on another filesystem (mount points stay empty). */
  one_file_system: boolean;
  /** What unreadable files, folders and unsupported entries do to the run. */
  error_policy: ErrorPolicy;
  /** Re-read and re-hash about this % (0–100) of unchanged-looking files per run. */
  rehash_percent: number;
  /** Stop the run with a partial version after uploading this much; null = no limit. */
  max_upload_bytes_per_run: number | null;
  /** Files above this size are hashed by several threads (0 = off). */
  parallel_file_threshold: number;
}

/** Rust `ufop_core::backup::errors::ErrorPolicy`. */
export type ErrorAction = "continue" | "fail";
export type UnknownEntryAction = "ignore" | "warn" | "fail";
export interface ErrorPolicy {
  on_file_error: ErrorAction;
  on_dir_error: ErrorAction;
  on_unknown_entry: UnknownEntryAction;
  /** Stop at the first error (keeping a partial version). */
  fail_fast: boolean;
  /** Stop after more than this many read problems; null = never. */
  max_errors: number | null;
}

export const DEFAULT_PARALLEL_FILE_THRESHOLD = 256 * 1024 * 1024;

/** Rust `hooks::StdinCommand`: a command whose output each backup stores as a file. */
export interface StdinCommand {
  command: string;
  /** `name` → /stdin/name, or an absolute path such as /dumps/db.sql. */
  path: string;
  /** 0 = no limit. */
  timeout_secs: number;
}

/** Rust `ufop_core::backup::estimate::BackupEstimate`. */
export interface BackupEstimate {
  files: number;
  folders: number;
  bytes_examined: number;
  new_files: number;
  changed_files: number;
  unchanged_files: number;
  deleted_files: number;
  /** New and changed files' bytes: the most the run uploads. */
  new_bytes_at_most: number;
  /** New files plus the growth of changed files. */
  new_bytes_at_least: number;
  /** True when changed files make the figure an upper bound. */
  upper_bound: boolean;
  has_previous_version: boolean;
  bytes_per_sec: number | null;
  speed_source: "history" | "assumed" | "none";
  estimated_duration_secs: number | null;
  largest: { path: string; size: number; status: "new" | "changed" }[];
  warnings: string[];
}

/** Which jobs "Back up all now" starts (`runs::RunAllFilter`). */
export interface RunAllFilter {
  job_ids: string[];
  scheduled_only: boolean;
  dry_run: boolean;
}

export interface RunAllReport {
  started: { job_id: string; job_name: string; task_id: string }[];
  skipped: { job_id: string; job_name: string; reason: string }[];
}

/** Rust `ufop_core::backup::versions::ClientIdentity`. */
export interface ClientIdentity {
  hostname: string;
  username: string;
  job_id: string;
  source_label: string;
}

/** Rust `ufop_core::backup::versions::VersionStats`: what a version added. */
export interface VersionStats {
  /** Sealed block volumes and indexes uploaded (after compression and encryption). */
  bytes_added: number;
  new_blocks: number;
  files_added: number;
  files_changed: number;
  files_deleted: number;
  /** The run stopped at its upload limit (the version is partial). Absent when false. */
  upload_cap_reached?: boolean;
}

/** Rust `MetadataPatch`: null leaves a field unchanged; "" removes a note / reason. */
export interface MetadataPatch {
  note: string | null;
  tags: Record<string, string> | null;
  pinned: boolean | null;
  pin_reason: string | null;
}

/** Rust `retention::RetentionVerdict`. */
export interface RetentionVerdict {
  id: number;
  timestamp: string;
  keep: boolean;
  reasons: string[];
  /** "kept: daily 7, weekly 4" / "deleted: …" */
  summary: string;
}

/** Rust `maintenance::RetentionPreview`. */
export interface RetentionPreview {
  policy: RetentionPolicy;
  legal_hold: boolean;
  keep: number;
  delete: number;
  versions: RetentionVerdict[];
}

/** Rust `shared::SourceInfo`: one job / computer writing to a repository. */
export interface SourceInfo {
  id: string;
  identity: ClientIdentity | null;
  label: string;
  versions: number;
  newest: string | null;
  oldest: string | null;
  newest_size: number;
  own: boolean;
}

/** Rust `backup_engine::versions::CopyTarget`. */
export type CopyTarget =
  | { type: "job"; job_id: string }
  | { type: "destination"; destination: BackupDestination; passphrase: string | null; prefix: string | null; options: BackupOptions | null };

/** Rust `copy::CopyReport`. */
export interface CopyReport {
  dry_run: boolean;
  versions_copied: number;
  versions_skipped: number;
  files: number;
  blocks_copied: number;
  blocks_reused: number;
  bytes_copied: number;
  bytes_uploaded: number;
  rechunked: boolean;
  copied: string[];
  warnings: string[];
}

/** Rust `shared::SharingReport`. */
export interface SharingReport {
  already_shared: boolean;
  versions_assigned: number;
  source: string;
}

/** Rust `ufop_core::backup::lock::LockKind`. */
export type LockKind = "exclusive" | "shared";

/** One lock file on the destination (Rust `lock::LockEntry`). */
export interface RepoLockEntry {
  file: string;
  owner_id: string;
  hostname: string;
  username: string;
  pid: number;
  app: string;
  version: string;
  operation: string;
  kind: LockKind;
  acquired_at: string;
  expires_at: string;
  /** Expired or unreadable: the next operation takes it over. */
  stale: boolean;
  damaged: boolean;
}

/** Rust `lock::LockStatus`. */
export interface RepoLockStatus {
  locks: RepoLockEntry[];
  exclusive: boolean;
  readers: number;
  checked_at: string;
}

/** Rust `lock::BreakLockReport`. */
export interface BreakLockReport {
  removed: string[];
  holders: string[];
}

/** Rust `db::RekeyReport`. */
export interface RekeyReport {
  encrypted: boolean;
  files: number;
  settings: number;
}

/** The live lock that blocks others, as "Repository in use by host (operation) since …"; null when free. */
export function lockSummary(status: RepoLockStatus | null): string | null {
  const live = (status?.locks ?? []).filter((l) => !l.stale);
  const first = live.find((l) => l.kind === "exclusive") ?? live[0];
  if (!first) return null;
  const host = first.hostname || "another computer";
  const since = new Date(first.acquired_at).toLocaleString();
  const more = live.length > 1 ? ` and ${live.length - 1} other(s)` : "";
  return `Repository in use by ${host} (${first.operation}) since ${since}${more}`;
}

/** Rust `ufop_core::backup::ChunkerKind`. */
export type ChunkerKind = "fixed" | "fastcdc";
/** Rust `ufop_core::backup::BlockCodec`. */
export type BlockCodec = "deflate" | "zstd";
/** Rust `ufop_core::backup::MetadataCompression`. */
export type MetadataCompression = "none" | "zstd";
/** Rust `ufop_core::backup::crypto::CipherKind`. */
export type CipherKind = "aes256_gcm" | "chacha20_poly1305";
/** Rust `ufop_core::backup::layout::RepoLayout`. */
export type RepoLayout = { type: "flat" } | { type: "sharded"; prefix_len: number };
/** Rust `ufop_core::backup::chunker::ChunkerSpec` (recorded in the repository header). */
export type ChunkerSpec = { type: "fixed"; size: number } | { type: "fastcdc"; min: number; avg: number; max: number };

/** `format_info` operation report (Rust `format::FormatInfo`). */
export interface RepositoryFormatInfo {
  format_version: number;
  chunker: ChunkerSpec;
  block_codec: BlockCodec;
  metadata_compression: MetadataCompression;
  /** null when unencrypted or GPG. */
  cipher: CipherKind | null;
  envelope_key: boolean;
  layout: RepoLayout;
  features: string[];
  rollback_available: boolean;
}

/** Upgrade / rollback / password change result (Rust `format::FormatReport`). */
export interface FormatReport {
  from_version: number;
  to_version: number;
  unchanged: boolean;
  features: string[];
  message: string;
}

/** `reshard` operation report (Rust `layout::ReshardReport`). */
export interface ReshardReport {
  from: RepoLayout;
  to: RepoLayout;
  moved: number;
  bytes_moved: number;
  already_in_place: number;
  dry_run: boolean;
  errors: string[];
}

export type KdfPreset = "interactive" | "default" | "sensitive";

/** Human description of a repository layout. */
export function describeLayout(l: RepoLayout): string {
  return l.type === "flat" ? "Flat (one folder)" : `Sharded (${l.prefix_len}-digit subfolders)`;
}

/** Human description of a chunker. */
export function describeChunker(c: ChunkerSpec): string {
  return c.type === "fixed" ? `Fixed ${formatSize(c.size)} blocks` : `Content-defined (FastCDC), ~${formatSize(c.avg)} blocks`;
}

function formatSize(n: number): string {
  return n >= 1024 * 1024 ? `${Math.round(n / (1024 * 1024))} MiB` : `${Math.round(n / 1024)} KiB`;
}

/** Rust `ufop_core::backup::photos::PhotosOptions`. */
export interface PhotosOptions {
  include_hidden: boolean;
  include_trashed: boolean;
  /** Only assets in these user albums (case-insensitive); empty = all. */
  albums: string[];
}

/** A Photos library found in ~/Pictures (`backup_photos_libraries`). */
export interface PhotosLibraryCandidate {
  path: string;
  name: string;
  /** `photos://` job source. */
  source: string;
  has_database: boolean;
  is_default: boolean;
}

/** `backup_photos_library_info`. */
export interface PhotosLibraryInfo {
  path: string;
  assets: number;
  hidden: number;
  trashed: number;
  albums: string[];
  warnings: string[];
}

export type LockMode = "governance" | "compliance";

export interface ObjectLockOptions {
  days: number;
  mode: LockMode;
}

export interface LocalTransfer {
  use_move: boolean;
  verify_size: boolean;
  sync_writes: boolean;
}

export type ReplicaSchedule = { type: "after_backup" } | { type: "interval"; minutes: number } | { type: "manual" };

export interface Replica {
  id: string;
  name: string;
  destination: BackupDestination;
  enabled: boolean;
  schedule: ReplicaSchedule;
  delete_extraneous: boolean;
  verify: boolean;
  throttle: Throttle;
  last_run: string | null;
  last_result: string | null;
}

export interface RemoteFile {
  name: string;
  size: number;
}

export type SnapshotPolicy = "off" | "auto" | "on" | "required";

/** Linux snapshot method; `auto` picks ZFS, btrfs or LVM by filesystem. */
export type SnapshotProvider = "auto" | "lvm" | "zfs" | "btrfs";

export interface SnapshotOptions {
  lvm_size: string;
  vss_provider: string | null;
  vss_exclude_writers: string[];
  mount_root: string | null;
  provider: SnapshotProvider;
}

export interface SnapshotDiagnostics {
  method: string;
  ok: boolean;
  mappings: [string, string][];
  steps: string[];
  error: string | null;
  /** Snapshot methods of this platform and whether their tools are installed. */
  providers: { name: string; available: boolean; detail: string }[];
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
  | { type: "custom"; rules: string }
  | { type: "counts"; keep_latest: number; keep_hourly: number; keep_daily: number; keep_weekly: number; keep_monthly: number; keep_yearly: number };

/** The everyday GFS preset: 7 daily, 4 weekly, 12 monthly, 3 yearly. */
export const GFS_PRESET: RetentionPolicy = { type: "counts", keep_latest: 0, keep_hourly: 0, keep_daily: 7, keep_weekly: 4, keep_monthly: 12, keep_yearly: 3 };

export interface BackupSchedule {
  enabled: boolean;
  start_time: string;
  interval_minutes: number;
  weekdays: number[];
  skip_on_battery: boolean;
  /** Holiday calendars whose days off skip runs (`schedule_library`). */
  calendar_ids: string[];
  /** Cron mode when not empty: expressions (5, 6 or 7 fields, local time) replacing start time, interval and weekdays. */
  cron: string[];
}

/** Mirrors `hooks::HookMode`. */
export type HookMode = "required" | "optional" | "async";

export interface ScriptConfig {
  run_before: string | null;
  /** Older flag; `before_mode` replaces it when set. */
  run_before_required: boolean;
  before_mode?: HookMode | null;
  run_after: string | null;
  after_mode?: HookMode | null;
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
  /** Replace file paths in reports and notifications with placeholders. */
  redact_paths: boolean;
  /** Trusted CA or pinned self-signed certificate (PEM). */
  ca_pem: string | null;
  retry_delay_ms: number;
  min_log_level: LogLevel;
  /** SHA-256 fingerprints; when set, only these certificates are accepted. */
  pinned_sha256: string[];
}

export type TlsMode = "start_tls" | "implicit" | "none";

export interface CertPolicy {
  accept_invalid_certs: boolean;
  ca_pem: string | null;
  pinned_sha256: string[];
}

export type NotificationChannel =
  | { type: "email"; host: string; port: number | null; tls: TlsMode; username: string; from: string; to: string[]; subject: string }
  | { type: "telegram"; chat_id: string; api_base: string }
  | { type: "xmpp"; jid: string; to: string; host: string; port: number | null; tls: TlsMode };

/** A notification channel (flattened) with its own result filter; the password or bot token lives under `credential_id`. */
export type BackupNotification = NotificationChannel & {
  send_on: string[];
  credential_id: string;
  template: string;
  cert: CertPolicy;
};

export interface LogFilter {
  min_level: LogLevel;
  include_tags: string[];
  exclude_tags: string[];
  pattern: string;
}

export interface LogOutputs {
  file: string | null;
  file_filter: LogFilter;
  file_max_mb: number;
  system_log: boolean;
  system_filter: LogFilter;
}

export interface KdfParams {
  memory_kib: number;
  iterations: number;
  parallelism: number;
}

export const KDF_PRESETS: Record<string, KdfParams> = {
  interactive: { memory_kib: 19 * 1024, iterations: 2, parallelism: 1 },
  default: { memory_kib: 65536, iterations: 3, parallelism: 4 },
  sensitive: { memory_kib: 256 * 1024, iterations: 4, parallelism: 4 },
};

export interface GpgOptions {
  /** Encrypt to these keys; empty = symmetric with the passphrase. */
  recipients: string[];
  executable: string;
  homedir: string;
}

export interface CryptReport {
  input_bytes: number;
  output_bytes: number;
  chunks: number;
}

export interface TlsOptions {
  accept_invalid_certs: boolean;
  ca_pem: string | null;
  pinned_sha256: string[];
}

export type SecretProviderKind =
  | { type: "env"; prefix: string }
  | { type: "file"; path: string }
  | { type: "pass"; executable: string; prefix: string }
  | { type: "vault"; address: string; mount: string; path: string; namespace: string; kv_version: number; tls: TlsOptions }
  | { type: "aws_secrets_manager"; region: string; access_key_id: string; endpoint: string; secret_id: string; tls: TlsOptions }
  | { type: "gcp_secret_manager"; project: string; endpoint: string; tls: TlsOptions }
  | { type: "azure_key_vault"; vault_url: string; tenant_id: string; client_id: string; authority: string; tls: TlsOptions };

/** A secret provider (kind flattened); its own credential lives under `credential_id`. */
export type SecretProvider = SecretProviderKind & {
  id: string;
  name: string;
  credential_id: string;
  enabled: boolean;
  /** Used by every job; otherwise only by jobs that select it. */
  global: boolean;
};

export interface SecretSettings {
  providers: SecretProvider[];
  cache: "none" | "memory" | "persistent";
  cache_ttl_secs: number;
}

export interface SecretProviderTest {
  provider_id: string;
  provider: string;
  kind: string;
  ok: boolean;
  has_credential: boolean;
  latency_ms: number;
  found: string[];
  missing: string[];
  errors: string[];
}

export interface PlaceholderReport {
  placeholders: { name: string; provider: string | null; from_cache: boolean; error: string | null }[];
  providers: string[];
}

export interface BugReport {
  path: string;
  size: number;
  files: string[];
  paths_obfuscated: number;
  runs_included: number;
}

export interface DatabaseCheck {
  info: DbInfo | null;
  problems: string[];
  missing_remote: string[];
  size_mismatch: string[];
  untracked_remote: string[];
  remote_files: number;
  tracked_volumes: number;
  check_ms: number;
}

export interface NetworkTiming {
  host: string;
  port: number;
  tls: boolean;
  addresses: string[];
  dns_ms: number | null;
  tcp_ms: number | null;
  tls_ms: number | null;
  tls_version: string | null;
  error: string | null;
}

export interface Throughput {
  list_ms: number[];
  test_bytes: number;
  upload_ms: number;
  upload_bps: number;
  download_ms: number;
  download_bps: number;
  verified: boolean;
  error: string | null;
}

export interface Diagnostics {
  database: DatabaseCheck | null;
  database_error: string | null;
  network: NetworkTiming | null;
  throughput: Throughput | null;
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
  local_transfer: LocalTransfer;
  replicas: Replica[];
  notifications: BackupNotification[];
  log_outputs: LogOutputs;
  /** Secret providers used by this job besides the global ones (ids). */
  secret_providers: string[];
  prevent_sleep: boolean;
  notify_on: string[];
  /** Legal hold: while set, retention is suspended and nothing can be removed. */
  legal_hold: LegalHold | null;
  /** Restore and browse only: every write to the destination is refused. */
  read_only: boolean;
  /** Encrypt the local database (key in the keychain). null: decided when saved (on for encrypted jobs); older jobs stay plain. */
  encrypt_local_database: boolean | null;
  /** A command whose output each backup stores as a virtual file (e.g. `pg_dump mydb`). */
  stdin_command: StdinCommand | null;
  created_at: string;
  updated_at: string;
  stats: JobStats;
}

export interface LegalHold {
  matter: string;
  reason: string;
  placed_by: string;
  placed_at: string;
}

/** `governance::ediscovery::EDiscoveryRequest`. */
export interface EDiscoveryRequest {
  matter: string;
  output: string;
  paths: string[];
  since: string | null;
  until: string | null;
  /** Empty = every job under legal hold. */
  job_ids: string[];
  include_files: boolean;
  max_file_bytes: number | null;
}

export interface EDiscoveryReport {
  output: string;
  sha256: string;
  bytes: number;
  entries: number;
  ledger_events: number;
  jobs: string[];
  restored_files: number;
  files_truncated: boolean;
  audit_ok: boolean;
}

export interface Fileset {
  id: number;
  timestamp: string;
  volume_name: string;
  partial: boolean;
  file_count: number;
  total_size: number;
  /** Source id (shared repositories); "" for versions from before identities. */
  source: string;
  note: string | null;
  tags: Record<string, string>;
  /** "Keep forever": retention skips it and deleting needs force. */
  pinned: boolean;
  pin_reason: string | null;
  identity: ClientIdentity | null;
  /** null for versions from before this was recorded ("—"). */
  stats: VersionStats | null;
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
  /** Shared repositories: restore from this source (id, or "*") instead of the job's own. */
  source: string | null;
  /** Pick the version by time, newest/oldest or id instead of `version`. */
  selector: VersionSelector | null;
  /** Write a ZIP / TAR archive instead of files. */
  archive: ArchiveTarget | null;
  /** Mirror: delete what the restored folders in `target` hold but the version does not. */
  delete_extraneous: boolean;
  restore_owner: OwnerPolicy;
  error_policy: RestoreErrorPolicy;
  sparse: SparseMode;
  /** Read volumes through the local volume cache. */
  use_cache: boolean;
}

/** Wire mirror of `ufop_core::backup::select::VersionSelector`. */
export type VersionSelector =
  | { type: "index"; index: number }
  | { type: "latest"; allow_partial?: boolean }
  | { type: "oldest"; allow_partial?: boolean }
  | { type: "as_of"; time: string; allow_partial?: boolean }
  | { type: "id"; id: string };

export type ArchiveFormat = "zip" | "zip_stored" | "tar" | "tar_gz";
export interface ArchiveTarget {
  path: string;
  /** null: from the file name's extension. */
  format: ArchiveFormat | null;
}
export type OwnerPolicy = "auto" | "always" | "never";
export type SparseMode = "auto" | "off";
export interface RestoreErrorPolicy {
  on_write_error: ErrorAction;
  on_dir_error: ErrorAction;
  on_missing_data: ErrorAction;
  on_metadata_error: UnknownEntryAction;
  fail_fast: boolean;
  max_errors: number | null;
}

export const DEFAULT_RESTORE_ERROR_POLICY: RestoreErrorPolicy = {
  on_write_error: "fail",
  on_dir_error: "fail",
  on_missing_data: "fail",
  on_metadata_error: "warn",
  fail_fast: false,
  max_errors: null,
};

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
  /** The restored version's number (one version restored). */
  version_index: number | null;
  /** The archive written (archive restores). */
  archive: string | null;
  /** Mirror: paths deleted (dry run: to delete); folders end in "/". */
  planned_deletions: string[];
  files_deleted: number;
  folders_deleted: number;
  owners_restored: number;
  sparse_files: number;
  stop_reason: string | null;
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
  elapsed_secs: number;
  bytes_per_sec: number;
  eta_secs: number | null;
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
  tag?: string | null;
}

export function newLogFilter(): LogFilter {
  return { min_level: "info", include_tags: [], exclude_tags: [], pattern: "" };
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
  /** Mirror restores delete files only with this set (after confirmation). */
  confirm_delete?: boolean;
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
  /** Sensitive columns are encrypted with the job's key. */
  encrypted: boolean;
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
  | { type: "delete_versions"; versions: number[]; allow_full_removal: boolean; force?: boolean }
  | { type: "consistency" }
  | { type: "rebuild_missing" }
  | { type: "extract_control_files"; version: number; target: string }
  | { type: "auto_tune"; sample_volumes: number }
  | { type: "snapshot_test" }
  | { type: "replicate"; replica_id: string | null }
  | { type: "apply_locks"; days: number; mode: LockMode; all: boolean }
  | { type: "list_locks" }
  | { type: "upgrade_format"; rollback: boolean }
  | { type: "reshard"; layout: RepoLayout; dry_run: boolean }
  | { type: "format_info" }
  | { type: "set_version_metadata"; version: number; source: string | null; patch: MetadataPatch }
  | { type: "apply_retention"; policy: RetentionPolicy | null; source: string | null }
  | { type: "copy_versions"; versions: number[]; source: string | null; target: CopyTarget; dry_run: boolean }
  | { type: "enable_sharing"; claim_all: boolean }
  | { type: "sync_sources" };

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
      ignore_files: [".ufopignore"],
      respect_gitignore: false,
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
      snapshot: { lvm_size: "10%ORIGIN", vss_provider: null, vss_exclude_writers: [], mount_root: null, provider: "auto" },
      encryption_module: "aes",
      gpg: { recipients: [], executable: "", homedir: "" },
      kdf: null,
      object_lock: null,
      photos: { include_hidden: false, include_trashed: false, albums: [] },
      format_version: null,
      chunker: null,
      block_codec: null,
      metadata_compression: null,
      cipher: "aes256_gcm",
      layout: { type: "flat" },
      lock_ttl_secs: 1800,
      clock_skew_warn_secs: 300,
      clock_skew_abort_secs: 3600,
      checkpoint_interval_secs: 900,
      checkpoint_interval_bytes: 0,
      default_note: null,
      default_tags: {},
      identity: { hostname: "", username: "", job_id: "", source_label: "" },
      shared_repository: false,
      shared_gc_delay_hours: 4,
      one_file_system: false,
      error_policy: { on_file_error: "continue", on_dir_error: "continue", on_unknown_entry: "ignore", fail_fast: false, max_errors: null },
      rehash_percent: 0,
      max_upload_bytes_per_run: null,
      parallel_file_threshold: DEFAULT_PARALLEL_FILE_THRESHOLD,
    },
    retention: { type: "keep_all" },
    schedule: { enabled: false, start_time: "02:00", interval_minutes: 1440, weekdays: [], skip_on_battery: false, calendar_ids: [], cron: [] },
    scripts: { run_before: null, run_before_required: false, before_mode: null, run_after: null, after_mode: null, run_on_source_release: null, timeout_secs: 60, result_format: "json" },
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
      redact_paths: false,
      ca_pem: null,
      retry_delay_ms: 500,
      min_log_level: "info",
      pinned_sha256: [],
    },
    throttle: { upload_bps: 0, download_bps: 0 },
    retry: { retries: 3, delay_ms: 2000, exponential: true, max_delay_ms: 60000 },
    soft_delete_prefix: null,
    store_config_remotely: false,
    remote_config_include_secrets: false,
    network_timeout_secs: 0,
    local_transfer: { use_move: true, verify_size: true, sync_writes: false },
    replicas: [],
    notifications: [],
    secret_providers: [],
    log_outputs: { file: null, file_filter: newLogFilter(), file_max_mb: 10, system_log: false, system_filter: { ...newLogFilter(), min_level: "warning" } },
    prevent_sleep: true,
    notify_on: ["warning", "error"],
    legal_hold: null,
    read_only: false,
    encrypt_local_database: null,
    stdin_command: null,
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
  source: null,
  selector: null,
  archive: null,
  delete_extraneous: false,
  restore_owner: "auto",
  error_policy: DEFAULT_RESTORE_ERROR_POLICY,
  sparse: "auto",
  use_cache: false,
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
  /** Queue a backup of every job (read-only and busy ones are skipped). */
  runAll: (filter?: Partial<RunAllFilter>) => Promise<RunAllReport>;
  /** Files, new data and time the next backup of a job likely needs. */
  estimate: (id: string, assumedBytesPerSec?: number | null) => Promise<BackupEstimate>;
  cancel: (id: string) => Promise<void>;
  execute: (id: string, operation: BackupOperation) => Promise<OperationResult>;
  /** Change an encrypted job's passphrase in place (format 2); `current` null = the stored one. */
  changePassphrase: (id: string, current: string | null, newPassphrase: string, kdfPreset?: KdfPreset | null) => Promise<FormatReport>;
  /** Who holds the job's repository lock right now. */
  lockStatus: (id: string) => Promise<RepoLockStatus>;
  /** Force-unlock (only when the holder is no longer running); `ownerId` limits it to the lock shown. */
  breakLock: (id: string, ownerId: string | null) => Promise<BreakLockReport>;
  /** Encrypt (or decrypt) the job's local database now. */
  setDatabaseEncryption: (id: string, enabled: boolean) => Promise<RekeyReport>;
  /** Versions of the job's source (`source`: another id or "*"), only those with every tag (`key=value` / `key`). */
  listVersions: (id: string, source?: string | null, tags?: string[]) => Promise<Fileset[]>;
  /** Sources (jobs / computers) known in the job's repository, own first. */
  listSources: (id: string) => Promise<SourceInfo[]>;
  /** Keep/delete per version for the job's policy or an unsaved one (changes nothing). */
  previewRetention: (id: string, policy: RetentionPolicy | null, source?: string | null) => Promise<RetentionPreview>;
  /** Change a version's note, tags or pin; returns the updated version. */
  setVersionMetadata: (id: string, version: number, patch: Partial<MetadataPatch>, source?: string | null) => Promise<Fileset>;
  pause: (minutes: number | null) => Promise<void>;
  resume: () => Promise<void>;
  placeLegalHold: (id: string, matter: string, reason: string) => Promise<BackupJob>;
  releaseLegalHold: (id: string) => Promise<BackupJob>;
  exportEDiscovery: (request: EDiscoveryRequest) => Promise<EDiscoveryReport>;
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

  runAll: async (filter = {}) => {
    const full: RunAllFilter = { job_ids: [], scheduled_only: false, dry_run: false, ...filter };
    const report = await tauriInvoke<RunAllReport>("backup_run_all", { filter: full });
    await get().loadStatus();
    return report;
  },

  estimate: (id, assumedBytesPerSec = null) => tauriInvoke<BackupEstimate>("backup_estimate", { jobId: id, assumedBytesPerSec }),

  cancel: async (id) => {
    await tauriInvoke<boolean>("backup_cancel", { id });
    await get().loadStatus();
  },

  execute: async (id, operation) => {
    const result = await tauriInvoke<OperationResult>("backup_execute", { id, operation });
    await Promise.all([get().loadJobs(), get().loadStatus()]);
    return result;
  },

  changePassphrase: async (id, current, newPassphrase, kdfPreset = null) => {
    const report = await tauriInvoke<FormatReport>("backup_change_passphrase", { id, current, newPassphrase, kdfPreset });
    await get().loadJobs();
    return report;
  },

  lockStatus: (id) => tauriInvoke<RepoLockStatus>("backup_lock_status", { jobId: id }),

  breakLock: (id, ownerId) => tauriInvoke<BreakLockReport>("backup_break_lock", { jobId: id, confirm: true, ownerId }),

  setDatabaseEncryption: async (id, enabled) => {
    const report = await tauriInvoke<RekeyReport>("backup_set_database_encryption", { jobId: id, enabled });
    await get().loadJobs();
    return report;
  },

  listVersions: (id, source = null, tags = []) => {
    const args: Record<string, unknown> = { id };
    if (source) args.source = source;
    if (tags.length) args.tags = tags;
    return ipc<Fileset[]>("backup_list_versions", args, []);
  },

  listSources: (id) => ipc<SourceInfo[]>("backup_list_sources", { id }, []),

  previewRetention: (id, policy, source = null) =>
    ipc<RetentionPreview>("backup_preview_retention", { id, policy, source }, { policy: policy ?? { type: "keep_all" }, legal_hold: false, keep: 0, delete: 0, versions: [] }),

  setVersionMetadata: async (id, version, patch, source = null) => {
    const full: MetadataPatch = { note: null, tags: null, pinned: null, pin_reason: null, ...patch };
    const r = await tauriInvoke<OperationResult>("backup_set_version_metadata", { id, version, source, patch: full });
    return r.report as unknown as Fileset;
  },

  pause: async (minutes) => {
    const status = await tauriInvoke<SchedulerStatus>("backup_pause", { minutes });
    set({ status });
  },

  resume: async () => {
    const status = await tauriInvoke<SchedulerStatus>("backup_resume", {});
    set({ status });
  },

  placeLegalHold: async (id, matter, reason) => {
    const job = await tauriInvoke<BackupJob>("backup_place_legal_hold", { id, matter, reason });
    await get().loadJobs();
    return job;
  },

  releaseLegalHold: async (id) => {
    const job = await tauriInvoke<BackupJob>("backup_release_legal_hold", { id });
    await get().loadJobs();
    return job;
  },

  exportEDiscovery: (request) => tauriInvoke<EDiscoveryReport>("ediscovery_export", { request }),

  clearError: () => set({ error: null }),
}));

/** Format a schedule for display. */
/** The pre-script's mode (maps the older `run_before_required`), like `ScriptConfig::before_mode`. */
export function hookBeforeMode(s: ScriptConfig): HookMode {
  return s.before_mode ?? (s.run_before_required ? "required" : "optional");
}

export function describeSchedule(s: BackupSchedule): string {
  if (!s.enabled) return "Manual only";
  const cron = (s.cron ?? []).filter((c) => c.trim() !== "");
  if (cron.length) return `cron ${cron.join(" | ")}`;
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
    case "counts":
      return describeCounts(r);
  }
}

/** "Keep 7 daily, 4 weekly, 12 monthly" (all zero: keep everything). */
export function describeCounts(r: Extract<RetentionPolicy, { type: "counts" }>): string {
  const parts = (
    [
      ["latest", r.keep_latest],
      ["hourly", r.keep_hourly],
      ["daily", r.keep_daily],
      ["weekly", r.keep_weekly],
      ["monthly", r.keep_monthly],
      ["yearly", r.keep_yearly],
    ] as const
  )
    .filter(([, n]) => n > 0)
    .map(([k, n]) => `${n} ${k}`);
  return parts.length ? `Keep ${parts.join(", ")}` : "Keep all versions (no limits set)";
}

/** `key=value` / `key` text for a tag. */
export function tagText(k: string, v: string): string {
  return v ? `${k}=${v}` : k;
}

/** Parse `key=value` (or bare `key`) lines / comma lists into tags; invalid names are dropped. */
export function parseTags(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const raw of text.split(/[,\n]/)) {
    const item = raw.trim();
    if (!item) continue;
    const i = item.indexOf("=");
    const k = (i < 0 ? item : item.slice(0, i)).trim();
    const v = i < 0 ? "" : item.slice(i + 1).trim();
    if (/^[A-Za-z0-9_.:-]{1,64}$/.test(k)) out[k] = v;
  }
  return out;
}

/** Does a version carry every tag of `filter` (bare key = any value)? Mirrors the Rust filter. */
export function matchesTags(tags: Record<string, string>, filter: Record<string, string>): boolean {
  return Object.entries(filter).every(([k, v]) => k in tags && (v === "" || tags[k] === v));
}
