/**
 * App settings an organization manages on its desktops: update channel,
 * automatic checks and a blackout window, the proxy, and AI. Delivered with
 * the policy bundle (`managedSettings`); `null` sections stay with the user.
 * The shape mirrors `ufop_core::agent::ManagedSettings` and is validated the
 * same way, so a desktop never receives settings it would reject.
 */
import { db, now, parseJson, type Row } from '../index';
import { OrgError } from './orgs';

export interface BlackoutWindow {
  start: string;
  end: string;
  days: number[];
}

export interface ManagedSettingsBody {
  updates: { channel: 'stable' | 'beta' | null; autoCheck: boolean | null; blackout: BlackoutWindow | null } | null;
  proxy: { mode: 'system' | 'none' | 'manual' | 'pac'; url: string; pacUrl: string; noProxy: string[] } | null;
  ai: { enabled: boolean | null; allowCloud: boolean | null } | null;
  /** Machine-wide caps for backups and restores (KiB/s; 0 = unlimited). */
  transfers: { maxUploadKbps: number | null; maxDownloadKbps: number | null; maxConcurrentRuns: number | null } | null;
}

export const MAX_CONCURRENT_RUNS = 8;

export interface ManagedSettings extends ManagedSettingsBody {
  version: number;
  updatedBy: string;
  updatedAt: string;
}

const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;
const triBool = (v: unknown, what: string): boolean | null => {
  if (v === null || v === undefined) return null;
  if (typeof v !== 'boolean') throw new OrgError(`${what} must be true, false or null`);
  return v;
};
const str = (v: unknown, n = 500) => (typeof v === 'string' ? v.trim().slice(0, n) : '');

/** Validate and normalize a settings body (throws `OrgError` with a readable message). */
export function normalizeManagedSettings(raw: unknown): ManagedSettingsBody {
  const b = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const out: ManagedSettingsBody = { updates: null, proxy: null, ai: null, transfers: null };
  if (b.updates) {
    const u = b.updates as Record<string, unknown>;
    const channel = u.channel === null || u.channel === undefined || u.channel === '' ? null : u.channel;
    if (channel !== null && channel !== 'stable' && channel !== 'beta') throw new OrgError('The update channel is stable or beta');
    let blackout: BlackoutWindow | null = null;
    if (u.blackout) {
      const w = u.blackout as Record<string, unknown>;
      const days = Array.isArray(w.days) ? w.days : [];
      if (!HHMM.test(String(w.start)) || !HHMM.test(String(w.end))) throw new OrgError('Blackout times must be HH:MM');
      if (w.start === w.end) throw new OrgError('The blackout window must not start and end at the same time');
      if (!days.every((d) => Number.isInteger(d) && (d as number) >= 0 && (d as number) <= 6)) throw new OrgError('Blackout days are 0 (Sunday) to 6 (Saturday)');
      blackout = { start: String(w.start), end: String(w.end), days: [...new Set(days as number[])].sort() };
    }
    out.updates = { channel: channel as 'stable' | 'beta' | null, autoCheck: triBool(u.autoCheck, 'Automatic update checks'), blackout };
    if (!out.updates.channel && out.updates.autoCheck === null && !out.updates.blackout) out.updates = null;
  }
  if (b.proxy) {
    const p = b.proxy as Record<string, unknown>;
    type Mode = NonNullable<ManagedSettingsBody['proxy']>['mode'];
    if (!['system', 'none', 'manual', 'pac'].includes(p.mode as string)) throw new OrgError('The proxy mode is system, none, manual or pac');
    const mode = p.mode as Mode;
    const proxy = { mode, url: str(p.url), pacUrl: str(p.pacUrl), noProxy: (Array.isArray(p.noProxy) ? p.noProxy : []).map((h) => str(h, 200)).filter(Boolean).slice(0, 200) };
    if (mode === 'manual' && !/^(https?|socks5h?):\/\/[^\s]+$/i.test(proxy.url)) throw new OrgError('A manual proxy needs a URL like http://proxy.example.com:3128');
    if (mode === 'pac' && !proxy.pacUrl) throw new OrgError('A PAC proxy needs the PAC file URL');
    out.proxy = proxy;
  }
  if (b.transfers) {
    const t = b.transfers as Record<string, unknown>;
    const kbps = (v: unknown, what: string): number | null => {
      if (v === null || v === undefined || v === '') return null;
      if (!Number.isInteger(v) || (v as number) < 0) throw new OrgError(`${what} must be a whole number of KB/s (0 = unlimited)`);
      return v as number;
    };
    const runs = t.maxConcurrentRuns === null || t.maxConcurrentRuns === undefined || t.maxConcurrentRuns === '' ? null : t.maxConcurrentRuns;
    if (runs !== null && (!Number.isInteger(runs) || (runs as number) < 1 || (runs as number) > MAX_CONCURRENT_RUNS)) throw new OrgError(`Concurrent operations must be 1 to ${MAX_CONCURRENT_RUNS}`);
    out.transfers = { maxUploadKbps: kbps(t.maxUploadKbps, 'The upload cap'), maxDownloadKbps: kbps(t.maxDownloadKbps, 'The download cap'), maxConcurrentRuns: runs as number | null };
    if (out.transfers.maxUploadKbps === null && out.transfers.maxDownloadKbps === null && out.transfers.maxConcurrentRuns === null) out.transfers = null;
  }
  if (b.ai) {
    const a = b.ai as Record<string, unknown>;
    out.ai = { enabled: triBool(a.enabled, 'AI enabled'), allowCloud: triBool(a.allowCloud, 'AI cloud models') };
    if (out.ai.enabled === null && out.ai.allowCloud === null) out.ai = null;
  }
  return out;
}

function row(orgId: string): Row | undefined {
  return db().prepare('SELECT * FROM managed_settings WHERE org_id = ?').get(orgId) as Row | undefined;
}

/** The settings in force; `null` when nothing is managed. */
export function getManagedSettings(orgId: string): ManagedSettings | null {
  const r = row(orgId);
  if (!r || r.settings === 'null') return null;
  const b = parseJson<ManagedSettingsBody>(r.settings, { updates: null, proxy: null, ai: null, transfers: null });
  return { updates: b.updates ?? null, proxy: b.proxy ?? null, ai: b.ai ?? null, transfers: b.transfers ?? null, version: Number(r.version), updatedBy: r.updated_by as string, updatedAt: r.updated_at as string };
}

/**
 * Save. An empty body means nothing is managed (agents release the
 * settings); the row keeps its version so the next change is newer still.
 */
export function setManagedSettings(orgId: string, raw: unknown, by: string): ManagedSettings | null {
  const body = normalizeManagedSettings(raw);
  const empty = !body.updates && !body.proxy && !body.ai && !body.transfers;
  const version = Number(row(orgId)?.version ?? 0) + 1;
  db()
    .prepare('INSERT INTO managed_settings (org_id, settings, version, updated_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(org_id) DO UPDATE SET settings = excluded.settings, version = excluded.version, updated_by = excluded.updated_by, updated_at = excluded.updated_at')
    .run(orgId, empty ? 'null' : JSON.stringify(body), version, by, now());
  return getManagedSettings(orgId);
}

/** What the agent receives: `null` when nothing is managed. */
export function managedSettingsForAgent(orgId: string): (ManagedSettingsBody & { version: number }) | null {
  const s = getManagedSettings(orgId);
  return s && { version: s.version, updates: s.updates, proxy: s.proxy, ai: s.ai, transfers: s.transfers };
}
