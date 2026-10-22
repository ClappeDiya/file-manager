/**
 * App settings managed by the organization's console. Mirrors
 * `ufop_core::agent::ManagedSettings` (camelCase on the wire); `null` means
 * the setting is the user's.
 */
import { tauriInvokeSafe } from "@/hooks/use-tauri";

export interface BlackoutWindow {
  /** `HH:MM`, local time. */
  start: string;
  end: string;
  /** 0 = Sunday … 6 = Saturday; empty = every day. */
  days: number[];
}

export interface ManagedUpdates {
  channel: "stable" | "beta" | null;
  autoCheck: boolean | null;
  blackout: BlackoutWindow | null;
}

export interface ManagedProxy {
  mode: "system" | "none" | "manual" | "pac";
  url: string;
  pacUrl: string;
  noProxy: string[];
}

export interface ManagedAi {
  enabled: boolean | null;
  allowCloud: boolean | null;
}

export interface ManagedTransfers {
  maxUploadKbps: number | null;
  maxDownloadKbps: number | null;
  maxConcurrentRuns: number | null;
}

export interface ManagedSettings {
  version: number;
  updates: ManagedUpdates | null;
  proxy: ManagedProxy | null;
  ai: ManagedAi | null;
  transfers: ManagedTransfers | null;
}

export function loadManagedSettings(): Promise<ManagedSettings | null> {
  return tauriInvokeSafe<ManagedSettings | null>("managed_settings_get", undefined, null);
}

function minutes(hhmm: string): number | null {
  const m = /^(\d{2}):(\d{2})$/.exec(hhmm);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

/** Same rule as the Rust `BlackoutWindow::contains` (overnight windows belong to their start day). */
export function inBlackout(w: BlackoutWindow | null | undefined, at: Date = new Date()): boolean {
  if (!w) return false;
  const s = minutes(w.start);
  const e = minutes(w.end);
  if (s === null || e === null) return false;
  const day = at.getDay();
  const min = at.getHours() * 60 + at.getMinutes();
  const dayOk = (d: number) => w.days.length === 0 || w.days.includes(d);
  if (s < e) return dayOk(day) && min >= s && min < e;
  return (dayOk(day) && min >= s) || (dayOk((day + 6) % 7) && min < e);
}

/** Short descriptions of what the organization manages, for Settings. */
export function describeManaged(m: ManagedSettings | null): string[] {
  if (!m) return [];
  const out: string[] = [];
  if (m.updates?.channel) out.push(`Update channel: ${m.updates.channel}`);
  if (m.updates?.autoCheck !== null && m.updates?.autoCheck !== undefined) out.push(`Automatic update checks: ${m.updates.autoCheck ? "on" : "off"}`);
  if (m.updates?.blackout) {
    const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const days = m.updates.blackout.days.length ? m.updates.blackout.days.map((d) => DAYS[d]).join(", ") : "every day";
    out.push(`No updates ${m.updates.blackout.start}–${m.updates.blackout.end} (${days})`);
  }
  if (m.proxy) out.push(`Proxy: ${m.proxy.mode}${m.proxy.mode === "manual" ? ` (${m.proxy.url})` : m.proxy.mode === "pac" ? ` (${m.proxy.pacUrl})` : ""}`);
  if (m.ai?.enabled === false) out.push("AI features: off");
  if (m.ai?.allowCloud === false) out.push("AI: local models only");
  const kbps = (v: number) => (v === 0 ? "unlimited" : `${v} KB/s`);
  if (m.transfers?.maxUploadKbps != null) out.push(`Total upload: ${kbps(m.transfers.maxUploadKbps)}`);
  if (m.transfers?.maxDownloadKbps != null) out.push(`Total download: ${kbps(m.transfers.maxDownloadKbps)}`);
  if (m.transfers?.maxConcurrentRuns != null) out.push(`Operations at the same time: ${m.transfers.maxConcurrentRuns}`);
  return out;
}
