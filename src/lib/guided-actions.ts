/**
 * What the guided wizards (`components/guided-flows/`) actually do. Each
 * wizard collects answers; these functions turn them into the same backend
 * calls the full screens use (sync pairs, connections, transfers, backups,
 * peers, compat renames, reports), so a wizard's result shows up there.
 */
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { isRemotePath } from "@/lib/location";
import { resolveItems, transferItems } from "@/lib/location-provider";
import { useSyncStore } from "@/stores/sync-store";
import { newBackupJob, useBackupStore, type BackupJob } from "@/stores/backup-store";

// ── Folders ──

interface FileEntryLite {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
}

/** `null` when every path is an existing local folder, else what is wrong. */
export async function checkFolders(paths: readonly string[]): Promise<string | null> {
  for (const p of paths) {
    if (!p.trim()) return "Choose both folders.";
    if (isRemotePath(p)) continue;
    try {
      const meta = await tauriInvoke<FileEntryLite>("get_file_metadata", { path: p });
      if (!meta.is_dir) return `"${p}" is a file, not a folder.`;
    } catch (e) {
      return `"${p}" cannot be opened: ${errorText(e)}`;
    }
  }
  const [a, b] = paths;
  if (a && b && a.replace(/[\\/]+$/, "") === b.replace(/[\\/]+$/, "")) return "The two folders must be different.";
  return null;
}

// ── Sync pairs ──

export type SyncSchedule = "manual" | "hourly" | "daily" | "realtime";
export type WizardSyncMode = "one-way" | "two-way" | "mirror";

/** The trigger and cron (7 fields, as the Sync panel writes them) for a schedule choice. */
export function syncTrigger(schedule: SyncSchedule): { trigger: string; cronExpr?: string } {
  switch (schedule) {
    case "hourly":
      return { trigger: "scheduled", cronExpr: "0 0 * * * * *" };
    case "daily":
      return { trigger: "scheduled", cronExpr: "0 0 2 * * * *" };
    case "realtime":
      return { trigger: "watch" };
    default:
      return { trigger: "manual" };
  }
}

export async function createSyncPair(p: { name: string; source: string; dest: string; mode: WizardSyncMode; schedule: SyncSchedule }) {
  const problem = await checkFolders([p.source, p.dest]);
  if (problem) throw new Error(problem);
  const { trigger, cronExpr } = syncTrigger(p.schedule);
  return useSyncStore.getState().createPair({
    name: p.name.trim() || defaultPairName(p.source, p.dest),
    sourcePath: p.source,
    destPath: p.dest,
    mode: p.mode.replace("-", "_"),
    trigger,
    cronExpr,
  });
}

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

export function defaultPairName(source: string, dest: string): string {
  return `${baseName(source)} → ${baseName(dest)}`;
}

// ── Connections ──

export interface ConnectionTest {
  connectionId: string;
  reachable: boolean;
  error: string | null;
}

/** Save a connection (or update `connectionId`), then sign in and list its
 *  folder: the real test (a wrong password or share name fails here). */
export async function saveAndTestConnection(p: {
  connectionId?: string | null;
  name: string;
  protocol: "smb" | "sftp";
  host: string;
  port: number | null;
  username: string | null;
  password: string | null;
  remotePath: string;
}): Promise<ConnectionTest> {
  const saved = await tauriInvoke<{ id: string }>("save_connection", {
    name: p.name,
    protocol: p.protocol,
    host: p.host.trim(),
    port: p.port,
    username: p.username || null,
    password: p.password || null,
    remotePath: p.remotePath || "/",
    groupId: null,
    bandwidthLimitBps: null,
    verifyChecksums: null,
    connectionId: p.connectionId ?? null,
  });
  try {
    await tauriInvoke("connector_connect", { protocol: p.protocol, host: p.host.trim(), port: p.port, username: p.username || null, remotePath: p.remotePath || "/", connectionId: saved.id });
    await tauriInvoke("connector_list_remote", { protocol: p.protocol, path: p.remotePath || "/", connectionId: saved.id });
    return { connectionId: saved.id, reachable: true, error: null };
  } catch (e) {
    return { connectionId: saved.id, reachable: false, error: errorText(e) };
  }
}

// ── Transfers ──

export type TransferMode = "copy" | "move";

/**
 * Copy or move a file or folder into `destDir`, locally or with a connection
 * on either end. `verify`: check a local copy byte for byte afterwards
 * (queued transfers are verified by the transfer engine itself).
 */
export async function startTransfer(source: string, destDir: string, mode: TransferMode, verify = false): Promise<string> {
  if (isRemotePath(source) || isRemotePath(destDir)) {
    const items = await resolveItems([source]);
    const out = await transferItems(items, destDir, mode);
    return `${out.jobs.length} file(s) queued${out.moved.length ? `, ${out.moved.length} moved on the server` : ""}.`;
  }
  const meta = verify && mode === "copy" ? await tauriInvoke<FileEntryLite>("get_file_metadata", { path: source }) : null;
  const r = await tauriInvoke<{ success: boolean; dest_paths: string[] }>(mode === "copy" ? "copy_files" : "move_files", { sourcePaths: [source], destDir });
  if (meta && r.dest_paths[0]) {
    const problem = await verifyCopy(source, r.dest_paths[0], meta.is_dir);
    if (problem) throw new Error(`Copied, but the check found a problem: ${problem}`);
    return "Copied and verified.";
  }
  return mode === "copy" ? "Copied." : "Moved.";
}

interface Totals {
  files: number;
  folders: number;
}

/**
 * Check a copy byte for byte: a file by SHA-256, a folder with the folder
 * comparison in checksum mode. `null` when the copy matches, else what differs.
 */
export async function verifyCopy(source: string, copy: string, isDir: boolean): Promise<string | null> {
  if (!isDir) {
    const [a, b] = await Promise.all([
      tauriInvoke<{ hash: string }>("integrity_checksum", { path: source, algorithm: "sha256" }),
      tauriInvoke<{ hash: string }>("integrity_checksum", { path: copy, algorithm: "sha256" }),
    ]);
    return a.hash === b.hash ? null : "The copy's content differs from the original.";
  }
  const r = await tauriInvoke<{ summary: { left_only: Totals; different: Totals; error: Totals } }>("compare_folders", {
    left: { type: "local", path: source },
    right: { type: "local", path: copy },
    options: { compare_mode: "checksum" },
    taskId: null,
  });
  const { left_only, different, error } = r.summary;
  const parts: string[] = [];
  if (left_only.files + left_only.folders > 0) parts.push(`${left_only.files + left_only.folders} item(s) missing in the copy`);
  if (different.files > 0) parts.push(`${different.files} file(s) differ`);
  if (error.files + error.folders > 0) parts.push(`${error.files + error.folders} item(s) could not be read`);
  return parts.length ? `${parts.join(", ")}.` : null;
}

export interface TransferJobLite {
  id: string;
  source_path: string;
  dest_path: string;
  total_bytes: number;
  transferred_bytes: number;
  status: string;
  error_message: string | null;
}

/** Transfers that stopped before the end: paused or failed. */
export async function listInterrupted(): Promise<TransferJobLite[]> {
  const jobs = await tauriInvoke<TransferJobLite[]>("list_transfers", undefined, []);
  return jobs.filter((j) => j.status === "paused" || j.status === "failed");
}

export type Resolution = "resume" | "cancel";

/** Resume (paused) or retry (failed) a transfer, or cancel it. */
export async function resolveInterrupted(job: TransferJobLite, resolution: Resolution): Promise<void> {
  if (resolution === "cancel") {
    await tauriInvoke("cancel_transfer", { jobId: job.id });
  } else if (job.status === "paused") {
    await tauriInvoke("resume_transfer", { jobId: job.id });
  } else if (!(await tauriInvoke<boolean>("retry_transfer", { jobId: job.id }))) {
    throw new Error("It has been retried as many times as allowed; start it again from its source.");
  }
}

// ── Drives ──

export interface DriveLite {
  name: string;
  mount_point: string;
  total_bytes: number;
  free_bytes: number;
  drive_type: string;
}

export async function listDrives(): Promise<DriveLite[]> {
  return tauriInvoke<DriveLite[]>("detect_drives", undefined, []);
}

/** Copy `sources` into `destDir` after checking there is room. */
export async function copyWithSpaceCheck(sources: readonly string[], destDir: string): Promise<string> {
  const counted = await tauriInvoke<{ files: number; bytes: number }>("count_affected_files", { paths: sources });
  const pre = await tauriInvoke<{ can_proceed: boolean; errors: { message: string }[] }>("transfer_preflight", { sourcePath: sources[0], destPath: destDir, totalBytes: counted.bytes });
  if (!pre.can_proceed) throw new Error(pre.errors.map((e) => e.message).join("; ") || "There is not enough room on the destination.");
  await tauriInvoke("copy_files", { sourcePaths: [...sources], destDir });
  return `Copied ${counted.files} file(s).`;
}

// ── Other computers (peer to peer) ──

export interface PeerLite {
  id: string;
  hostname: string;
  display_name: string | null;
  status: string;
}

/** Start looking for computers on this network and return the ones online. */
export async function listPeers(): Promise<PeerLite[]> {
  await tauriInvoke("peer_start_listener").catch(() => {});
  await tauriInvoke("peer_start_discovery").catch(() => {});
  return tauriInvoke<PeerLite[]>("peer_list_online", undefined, []);
}

/** Offer `paths` to another computer; it sends once the other side accepts. */
export async function sendToPeer(peerId: string, paths: readonly string[]): Promise<void> {
  await tauriInvoke("peer_request_transfer", { peerId, paths: [...paths] });
}

/** The usual folders of this computer's user, by name. */
export async function userFolders(names: readonly string[]): Promise<string[]> {
  const path = await import("@tauri-apps/api/path");
  const lookup: Record<string, () => Promise<string>> = {
    Documents: path.documentDir,
    Desktop: path.desktopDir,
    Pictures: path.pictureDir,
    Music: path.audioDir,
    Videos: path.videoDir,
    Downloads: path.downloadDir,
  };
  const out: string[] = [];
  for (const n of names) {
    const fn = lookup[n];
    if (fn) out.push(await fn());
  }
  return out;
}

// ── File name compatibility ──

export interface CompatIssue {
  path: string;
  name: string;
  suggested_name: string;
  rules: string[];
  tier: number;
  fixable: boolean;
}

export interface CompatScan {
  issues: CompatIssue[];
  scanned: number;
  truncated: boolean;
}

export interface CompatRenameReport {
  renamed: [string, string][];
  failed: { path: string; reason: string }[];
}

export interface CompatRenameRecord {
  id: string;
  original_name: string;
  translated_name: string;
  folder: string;
  exists: boolean;
  restorable: boolean;
  timestamp: string;
}

export const compatProfiles = () => tauriInvoke<{ id: string; name: string; description: string }[]>("compat_profiles", undefined, []);
export const scanNames = (path: string, profileId: string) => tauriInvoke<CompatScan>("compat_scan_folder", { path, profileId });
export const fixNames = (fixes: { path: string; new_name: string }[], profileId: string) => tauriInvoke<CompatRenameReport>("compat_fix_names", { fixes, profileId });
export const listRenames = (folder?: string | null) => tauriInvoke<CompatRenameRecord[]>("compat_list_renames", { folder: folder || null }, []);
export const restoreNames = (ids: string[]) => tauriInvoke<CompatRenameReport>("compat_restore_names", { ids });

// ── Reports ──

export type ReportKind = "transfer" | "activity" | "audit";
export type ReportFormat = "csv" | "json";

function csvCell(v: unknown): string {
  const s = v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v);
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** Rows as CSV with a header from the first row's keys. */
export function toCsv(rows: ReadonlyArray<Record<string, unknown>>): string {
  if (rows.length === 0) return "";
  const keys = Object.keys(rows[0]);
  return [keys.join(","), ...rows.map((r) => keys.map((k) => csvCell(r[k])).join(","))].join("\n") + "\n";
}

/** The report's text. The security audit log is always JSON lines. */
export async function buildReport(kind: ReportKind, format: ReportFormat): Promise<string> {
  switch (kind) {
    case "transfer":
      return tauriInvoke<string>("export_transfer_history", { format, pathContains: null, status: null, dateFrom: null, dateTo: null });
    case "activity": {
      const events = await tauriInvoke<Record<string, unknown>[]>("ledger_query", { query: { limit: 10_000 } }, []);
      return format === "csv" ? toCsv(events) : JSON.stringify(events, null, 2);
    }
    case "audit":
      return tauriInvoke<string>("export_audit_log");
  }
}

export function reportFileName(kind: ReportKind, format: ReportFormat, now = new Date()): string {
  const day = now.toISOString().slice(0, 10);
  const ext = kind === "audit" ? "jsonl" : format;
  return `ufop-${kind}-report-${day}.${ext}`;
}

/** Ask where to save, then write. `null` when the user cancelled. */
export async function saveReport(kind: ReportKind, format: ReportFormat): Promise<string | null> {
  const name = reportFileName(kind, format);
  const ext = name.split(".").pop()!;
  const path = await tauriInvoke<string | null>("pick_save_file", { title: "Save report", defaultName: name, extensions: [ext] });
  if (!path) return null;
  await tauriInvoke("write_text_file_full", { path, content: await buildReport(kind, format) });
  return path;
}

// ── Backups ──

export type BackupScheduleChoice = "once" | "daily" | "weekly";

/** A backup job for one folder to a local folder (or drive), with the defaults of a new job. */
export function backupJobFor(p: { source: string; dest: string; schedule: BackupScheduleChoice; encrypted: boolean }): BackupJob {
  const job = newBackupJob();
  job.name = `Backup of ${baseName(p.source)}`;
  job.sources = [p.source];
  job.destination = { type: "local", path: p.dest, alternate_paths: [], marker_file: null };
  job.encrypted = p.encrypted;
  if (p.schedule !== "once") {
    job.schedule = { ...job.schedule, enabled: true, interval_minutes: p.schedule === "daily" ? 1440 : 10080 };
  }
  return job;
}

/** Save the job and run its first backup now. Returns the job. */
export async function createBackup(p: { source: string; dest: string; schedule: BackupScheduleChoice; passphrase: string | null }): Promise<BackupJob> {
  const problem = await checkFolders([p.source, p.dest]);
  if (problem) throw new Error(problem);
  const store = useBackupStore.getState();
  const saved = await store.saveJob(backupJobFor({ ...p, encrypted: !!p.passphrase }), p.passphrase);
  await store.runJob(saved.id);
  return saved;
}
