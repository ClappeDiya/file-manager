/**
 * Mount options and the organization's mount profile (Rust:
 * `mount_engine::{MountOptions, PinReport}`, `ufop_core::agent::ManagedMounts`).
 */
import { useEffect, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";

export type CacheMode = "off" | "minimal" | "writes" | "full";

export interface MountOptions {
  cache_mode: CacheMode;
  max_cache_mb: number | null;
  max_cache_age_hours: number | null;
  read_only: boolean;
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

export const human = (n: number) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toFixed(1)} ${u[i]}`;
};

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
