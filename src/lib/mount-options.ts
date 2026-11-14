/**
 * Mount options and the organization's mount profile (Rust:
 * `mount_engine::{MountOptions, PinReport}`, `ufop_core::agent::ManagedMounts`).
 */
import { formatBytes } from "@/lib/format-bytes";
import { useEffect, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";

export type CacheMode = "off" | "minimal" | "writes" | "full";

export interface MountOptions {
  cache_mode: CacheMode;
  max_cache_mb: number | null;
  max_cache_age_hours: number | null;
  read_only: boolean;
  /** Mount this folder of the connection instead of its whole remote path. */
  subfolder?: string | null;
  /** Open the folder in the file manager after it mounts. */
  open_after_mount?: boolean;
  /** Windows: mount as this drive letter (D–Z). */
  drive_letter?: string | null;
  /** The name Explorer / Finder shows (Windows and macOS). */
  volume_label?: string | null;
  /** Windows: show as a network drive. */
  network_mode?: boolean;
  /** Not Windows: let other users of this computer see the mount. */
  allow_other?: boolean;
  /** Octal, e.g. "0644". */
  file_perms?: string | null;
  dir_perms?: string | null;
  /** Not Windows. */
  uid?: number | null;
  gid?: number | null;
  /** Keep this drive's cache under this folder (default: the app's cache folder). */
  cache_dir?: string | null;
  /** How long a folder listing is trusted, e.g. "30s" (default 30s). */
  dir_cache_time?: string | null;
  /** Extra rclone mount flags from the allow-list: `--name` or `--name=value`. */
  extra_flags?: string[];
}

/** Mirrors `mount_engine::MountStatus`. */
export type MountStatus = "mounted" | "unmounted" | "error";

/** Mirrors `mount_engine::MountInfo`. */
export interface MountInfo {
  id: string;
  connection_id: string;
  connection_name: string;
  protocol: string;
  mount_point: string;
  status: MountStatus;
  auto_mount: boolean;
  created_at: string;
  error_message: string | null;
  /** Cache and read-only options in force (after the mount profile). */
  options?: MountOptions;
  /** What the organization's mount profile changed. */
  policy_notes?: string[];
}

/** Mirrors `mount_engine::MountResult`. */
export interface MountResult {
  mount_id: string;
  mount_point: string;
  status: MountStatus;
  message: string;
}

/** Mirrors `mount_engine::UnmountAllReport`. */
export interface UnmountAllReport {
  unmounted: number;
  failed: string[];
}

export interface ManagedMounts {
  allowedProtocols: string[] | null;
  readOnly: boolean | null;
  cacheMode: CacheMode | null;
  maxCacheMb: number | null;
  maxCacheAgeHours: number | null;
  mountRoot: string | null;
}

export interface PinReport {
  files: number;
  bytes: number;
  failed: number;
  truncated: boolean;
}

export const DEFAULT_MOUNT_OPTIONS: MountOptions = {
  cache_mode: "writes",
  max_cache_mb: null,
  max_cache_age_hours: null,
  read_only: false,
};

export const CACHE_MODES: [CacheMode, string][] = [
  ["off", "Off — always read from the server"],
  ["minimal", "Minimal — only open files"],
  ["writes", "Writes — upload in the background (default)"],
  ["full", "Full — keep files for offline use"],
];

export const isCloudMount = (protocol: string) =>
  protocol === "rclone" || protocol === "s3";

/** Sizes in the units chosen in Settings → Appearance. */
export const human = (n: number) => formatBytes(n);

/** What the profile says, in one line. */
export function describePolicy(p: ManagedMounts): string {
  const parts: string[] = [];
  if (p.allowedProtocols)
    parts.push(`only ${p.allowedProtocols.join(", ") || "no"} drives`);
  if (p.mountRoot) parts.push(`mount points inside ${p.mountRoot}`);
  if (p.readOnly) parts.push("read-only");
  if (p.cacheMode) parts.push(`cache ${p.cacheMode}`);
  if (p.maxCacheMb) parts.push(`cache ≤ ${p.maxCacheMb} MB`);
  if (p.maxCacheAgeHours) parts.push(`cached files ≤ ${p.maxCacheAgeHours} h`);
  return parts.join(" · ");
}

export function useMountPolicy(): ManagedMounts | null {
  const [p, setP] = useState<ManagedMounts | null>(null);
  useEffect(() => {
    void tauriInvokeSafe<ManagedMounts | null>(
      "mount_policy",
      undefined,
      null,
    ).then(setP);
  }, []);
  return p;
}

/** Protocols the Mounts screen can mount (natively, or through rclone). */
const MOUNTABLE_PROTOCOLS = [
  "smb",
  "cifs",
  "nfs",
  "sftp",
  "webdav",
  "rclone",
  "s3",
];
export const isMountable = (protocol: string) =>
  MOUNTABLE_PROTOCOLS.includes(protocol.toLowerCase());

export type HostOs = "windows" | "mac" | "linux";

/** The OS the window runs on (which mount options exist differs). */
export function hostOs(): HostOs {
  if (typeof navigator === "undefined") return "linux";
  const p = `${navigator.platform} ${navigator.userAgent}`;
  if (/win/i.test(navigator.platform)) return "windows";
  if (/mac/i.test(p)) return "mac";
  return "linux";
}

/**
 * The options to send: the four cache options always, and an advanced option
 * only when it has a value, so a default mount sends what it always sent.
 */
export function cleanMountOptions(o: MountOptions): MountOptions {
  const out: MountOptions = {
    cache_mode: o.cache_mode,
    max_cache_mb: o.max_cache_mb,
    max_cache_age_hours: o.max_cache_age_hours,
    read_only: o.read_only,
  };
  const text = (v: string | null | undefined) => v?.trim() || null;
  const set = <K extends keyof MountOptions>(
    k: K,
    v: MountOptions[K] | null | undefined | false,
  ) => {
    if (v !== null && v !== undefined && v !== false)
      out[k] = v as MountOptions[K];
  };
  set("subfolder", text(o.subfolder));
  set("open_after_mount", o.open_after_mount);
  set("drive_letter", text(o.drive_letter));
  set("volume_label", text(o.volume_label));
  set("network_mode", o.network_mode);
  set("allow_other", o.allow_other);
  set("file_perms", text(o.file_perms));
  set("dir_perms", text(o.dir_perms));
  set("uid", o.uid);
  set("gid", o.gid);
  set("cache_dir", text(o.cache_dir));
  set("dir_cache_time", text(o.dir_cache_time));
  if (o.extra_flags?.length) out.extra_flags = o.extra_flags;
  return out;
}

/** Native (non-rclone) mounts take only the subfolder and "open after mounting". */
export function mountPayloadOptions(
  cloud: boolean,
  o: MountOptions,
): MountOptions | null {
  const clean = cleanMountOptions(o);
  if (cloud) return clean;
  return clean.subfolder || clean.open_after_mount ? clean : null;
}
