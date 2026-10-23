/**
 * App settings an organization manages on its desktops: update channel,
 * automatic checks and a blackout window, the proxy, AI, transfer caps and
 * the mount profile (what drives may be mounted, and how). Delivered with
 * the policy bundle (`managedSettings`); `null` sections stay with the user.
 * Organization backup policy templates travel the same way (`backupTemplates`),
 * and so do accounts and per-job access rules for ufop-server nodes
 * (`serverAccess`; passwords only as Argon2 hashes).
 * The shape mirrors `ufop_core::agent::ManagedSettings` and is validated the
 * same way, so a desktop never receives settings it would reject.
 */
import { db, now, parseJson, type Row } from '../index';
import { OrgError } from './orgs';
import { MOUNT_CACHE_MODES, MOUNT_PROTOCOLS, type ManagedMounts } from '../../mount-profile';

export type { ManagedMounts };

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
  /** Mount profile: allowed protocols, read-only, forced cache mode and limits, mount root. */
  mounts: ManagedMounts | null;
  /** Automation rules the organization owns on its desktops (read-only there). */
  workflows: ManagedWorkflow[] | null;
  /** Backup policy templates offered (or, `enforced`, imposed) on every desktop. */
  backupTemplates: ManagedBackupTemplate[] | null;
  /** Notification profiles offered read-only on every desktop (secrets as `central:<id>` credentials). */
  notificationProfiles: ManagedNotificationProfile[] | null;
  /** Accounts and per-job access rules for ufop-server nodes (read-only there). */
  serverAccess: ManagedServerAccess | null;
}

export interface ManagedServerAccess {
  users: { username: string; passwordHash: string; role: 'admin' | 'operator' | 'viewer'; disabled: boolean }[];
  rules: { principal: string; jobPattern: string; permission: 'none' | 'read' | 'run' | 'write' | 'full' }[];
}

/** Mirrors `ufop_core::agent::{MAX_SERVER_USERS, MAX_SERVER_RULES}`. */
export const MAX_SERVER_USERS = 500;
export const MAX_SERVER_RULES = 2000;
const SERVER_ROLES = ['admin', 'operator', 'viewer'] as const;
const SERVER_PERMISSIONS = ['none', 'read', 'run', 'write', 'full'] as const;
const SERVER_USERNAME = /^[A-Za-z0-9._@-]{1,64}$/;

/** Validate server accounts and access rules (same checks as the server). */
export function normalizeServerAccess(raw: unknown): ManagedServerAccess | null {
  if (raw === null || raw === undefined) return null;
  if (typeof raw !== 'object' || Array.isArray(raw)) throw new OrgError('Server access must be an object with users and rules');
  const o = raw as Record<string, unknown>;
  const users = Array.isArray(o.users) ? o.users : [];
  const rules = Array.isArray(o.rules) ? o.rules : [];
  if (users.length > MAX_SERVER_USERS || rules.length > MAX_SERVER_RULES) throw new OrgError(`At most ${MAX_SERVER_USERS} server accounts and ${MAX_SERVER_RULES} access rules`);
  const names = new Set<string>();
  const outUsers = users.map((u) => {
    const x = (u && typeof u === 'object' ? u : {}) as Record<string, unknown>;
    const username = str(x.username, 64);
    if (!SERVER_USERNAME.test(username)) throw new OrgError(`Server account name '${username}' must be 1–64 letters, digits, '.', '_', '@' or '-'`);
    if (names.has(username.toLowerCase())) throw new OrgError(`Two server accounts are named '${username}'`);
    names.add(username.toLowerCase());
    const passwordHash = str(x.passwordHash, 1000);
    if (!passwordHash.startsWith('$argon2')) throw new OrgError(`Server account '${username}' needs an Argon2 password hash ($argon2id$…, from ufop-server hash-password), not a password`);
    const role = String(x.role ?? 'viewer') as (typeof SERVER_ROLES)[number];
    if (!SERVER_ROLES.includes(role)) throw new OrgError(`Server account '${username}' has an unknown role '${role}' (admin, operator or viewer)`);
    return { username, passwordHash, role, disabled: x.disabled === true };
  });
  const outRules = rules.map((r) => {
    const x = (r && typeof r === 'object' ? r : {}) as Record<string, unknown>;
    const principal = str(x.principal, 80);
    const jobPattern = str(x.jobPattern, 200);
    if (!principal || !jobPattern) throw new OrgError('Every access rule needs a principal and a job pattern');
    const permission = String(x.permission ?? '') as (typeof SERVER_PERMISSIONS)[number];
    if (!SERVER_PERMISSIONS.includes(permission)) throw new OrgError(`Unknown access permission '${permission}' (none, read, run, write or full)`);
    return { principal, jobPattern, permission };
  });
  return outUsers.length || outRules.length ? { users: outUsers, rules: outRules } : null;
}

/** A backup policy template: `settings` as the desktop app exports a template's settings. */
export interface ManagedBackupTemplate {
  id: string;
  name: string;
  description: string;
  enforced: boolean;
  settings: Record<string, unknown>;
}

/** Mirrors `ufop_core::agent::{MAX_BACKUP_TEMPLATES, MAX_BACKUP_TEMPLATE_BYTES}`. */
export const MAX_BACKUP_TEMPLATES = 50;
export const MAX_BACKUP_TEMPLATE_BYTES = 64 * 1024;

/** A notification profile: `notification` as the desktop app stores a profile's channel. */
export interface ManagedNotificationProfile {
  id: string;
  name: string;
  description: string;
  notification: Record<string, unknown>;
  minSeverity: 'success' | 'warning' | 'error' | null;
}

/** Mirrors `ufop_core::agent::{MAX_NOTIFICATION_PROFILES, MAX_NOTIFICATION_PROFILE_BYTES}`. */
export const MAX_NOTIFICATION_PROFILES = 50;
export const MAX_NOTIFICATION_PROFILE_BYTES = 64 * 1024;
const CENTRAL = /^central:[A-Za-z0-9_-]{1,64}$/;

/** Validate organization notification profiles: secrets must name console credentials, never travel in plain text. */
export function normalizeNotificationProfiles(raw: unknown): ManagedNotificationProfile[] | null {
  if (raw === null || raw === undefined) return null;
  if (!Array.isArray(raw)) throw new OrgError('Notification profiles must be a list');
  if (raw.length > MAX_NOTIFICATION_PROFILES) throw new OrgError(`At most ${MAX_NOTIFICATION_PROFILES} organization notification profiles`);
  const ids = new Set<string>();
  const out = raw.map((t, i) => {
    const o = (t && typeof t === 'object' ? t : {}) as Record<string, unknown>;
    const name = str(o.name, 200);
    if (!name) throw new OrgError(`Notification profile ${i + 1} needs a name`);
    const id = str(o.id, 64) || name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64);
    if (!WORKFLOW_ID.test(id)) throw new OrgError(`Notification profile id '${id}' must be 1–64 letters, digits, '-' or '_'`);
    if (ids.has(id)) throw new OrgError(`Two notification profiles share the id '${id}'`);
    ids.add(id);
    let n = o.notification ?? null;
    if (typeof n === 'string') {
      try {
        n = JSON.parse(n);
      } catch {
        throw new OrgError(`Notification profile '${name}' channel is not valid JSON`);
      }
    }
    if (!n || typeof n !== 'object' || Array.isArray(n)) throw new OrgError(`Notification profile '${name}' needs a notification object`);
    const ch = n as Record<string, unknown>;
    if (typeof ch.type !== 'string') throw new OrgError(`Notification profile '${name}' needs a channel type`);
    if (JSON.stringify(ch).length > MAX_NOTIFICATION_PROFILE_BYTES) throw new OrgError(`Notification profile '${name}' is larger than ${MAX_NOTIFICATION_PROFILE_BYTES / 1024} KB`);
    const cred = ch.credential_id;
    if (cred !== undefined && cred !== null && cred !== '' && !(typeof cred === 'string' && CENTRAL.test(cred))) throw new OrgError(`Notification profile '${name}': credential_id must name a console credential (central:<id>)`);
    for (const h of Array.isArray(ch.headers) ? (ch.headers as Record<string, unknown>[]) : []) {
      if (h && h.secret === true) {
        if (typeof h.value === 'string' && h.value !== '') throw new OrgError(`Notification profile '${name}': secret header values cannot be sent in plain text; use a console credential`);
        if (typeof h.credential_id !== 'string' || !CENTRAL.test(h.credential_id)) throw new OrgError(`Notification profile '${name}': secret headers must name a console credential (central:<id>)`);
      }
    }
    const sev = o.minSeverity ?? null;
    if (sev !== null && sev !== '' && !['success', 'warning', 'error'].includes(sev as string)) throw new OrgError(`Notification profile '${name}': unknown minimum severity '${String(sev)}'`);
    return { id, name, description: str(o.description, 1000), notification: ch, minSeverity: sev === '' ? null : (sev as ManagedNotificationProfile['minSeverity']) };
  });
  return out.length ? out : null;
}

/** Validate organization backup templates (the desktop checks the settings in full). */
export function normalizeBackupTemplates(raw: unknown): ManagedBackupTemplate[] | null {
  if (raw === null || raw === undefined) return null;
  if (!Array.isArray(raw)) throw new OrgError('Backup templates must be a list');
  if (raw.length > MAX_BACKUP_TEMPLATES) throw new OrgError(`At most ${MAX_BACKUP_TEMPLATES} organization backup templates`);
  const ids = new Set<string>();
  const out = raw.map((t, i) => {
    const o = (t && typeof t === 'object' ? t : {}) as Record<string, unknown>;
    const name = str(o.name, 200);
    if (!name) throw new OrgError(`Backup template ${i + 1} needs a name`);
    const id = str(o.id, 64) || name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64);
    if (!WORKFLOW_ID.test(id)) throw new OrgError(`Backup template id '${id}' must be 1–64 letters, digits, '-' or '_'`);
    if (ids.has(id)) throw new OrgError(`Two backup templates share the id '${id}'`);
    ids.add(id);
    let settings = o.settings ?? {};
    if (typeof settings === 'string') {
      try {
        settings = JSON.parse(settings);
      } catch {
        throw new OrgError(`Backup template '${name}' settings are not valid JSON`);
      }
    }
    if (!settings || typeof settings !== 'object' || Array.isArray(settings)) throw new OrgError(`Backup template '${name}' settings must be an object (export a template from the desktop app)`);
    if (JSON.stringify(settings).length > MAX_BACKUP_TEMPLATE_BYTES) throw new OrgError(`Backup template '${name}' is larger than ${MAX_BACKUP_TEMPLATE_BYTES / 1024} KB`);
    const sc = (settings as Record<string, unknown>).scripts;
    if (sc !== undefined && sc !== null) throw new OrgError(`Backup template '${name}' sets scripts; organization templates cannot`);
    return { id, name, description: str(o.description, 1000), enforced: o.enforced === true, settings: settings as Record<string, unknown> };
  });
  return out.length ? out : null;
}

/** An organization workflow: an automation rule as the desktop app exports it. */
export interface ManagedWorkflow {
  id: string;
  name: string;
  enabled: boolean;
  rule: Record<string, unknown>;
}

/** Mirrors `ufop_core::agent::{MAX_WORKFLOWS, MAX_WORKFLOW_BYTES, FORBIDDEN_WORKFLOW_ACTIONS}`. */
export const MAX_WORKFLOWS = 50;
export const MAX_WORKFLOW_BYTES = 256 * 1024;
const FORBIDDEN_ACTIONS = ['run_command', 'execute_custom_command'];
const WORKFLOW_ID = /^[A-Za-z0-9_-]{1,64}$/;

function forbiddenAction(v: unknown): string | null {
  if (Array.isArray(v)) {
    for (const x of v) {
      const f = forbiddenAction(x);
      if (f) return f;
    }
    return null;
  }
  if (v && typeof v === 'object') {
    const t = (v as Record<string, unknown>).type;
    if (typeof t === 'string' && FORBIDDEN_ACTIONS.includes(t)) return t;
    for (const x of Object.values(v)) {
      const f = forbiddenAction(x);
      if (f) return f;
    }
  }
  return null;
}

/** Validate organization workflows (the desktop checks the same, and refuses the rest). */
export function normalizeWorkflows(raw: unknown): ManagedWorkflow[] | null {
  if (raw === null || raw === undefined) return null;
  if (!Array.isArray(raw)) throw new OrgError('Workflows must be a list');
  if (raw.length > MAX_WORKFLOWS) throw new OrgError(`At most ${MAX_WORKFLOWS} organization workflows`);
  const ids = new Set<string>();
  const out = raw.map((w, i) => {
    const o = (w && typeof w === 'object' ? w : {}) as Record<string, unknown>;
    const name = str(o.name, 200);
    if (!name) throw new OrgError(`Workflow ${i + 1} needs a name`);
    const id = str(o.id, 64) || name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64);
    if (!WORKFLOW_ID.test(id)) throw new OrgError(`Workflow id '${id}' must be 1–64 letters, digits, '-' or '_'`);
    if (ids.has(id)) throw new OrgError(`Two workflows share the id '${id}'`);
    ids.add(id);
    let rule = o.rule;
    if (typeof rule === 'string') {
      try {
        rule = JSON.parse(rule);
      } catch {
        throw new OrgError(`Workflow '${name}' is not valid JSON`);
      }
    }
    if (!rule || typeof rule !== 'object' || Array.isArray(rule) || !('trigger' in rule) || !('action' in rule)) throw new OrgError(`Workflow '${name}' is not an automation rule (export it from the desktop app)`);
    if (JSON.stringify(rule).length > MAX_WORKFLOW_BYTES) throw new OrgError(`Workflow '${name}' is larger than ${MAX_WORKFLOW_BYTES / 1024} KB`);
    const bad = forbiddenAction(rule);
    if (bad) throw new OrgError(`Workflow '${name}' runs a program (${bad}); organization workflows cannot`);
    return { id, name, enabled: o.enabled !== false, rule: rule as Record<string, unknown> };
  });
  return out.length ? out : null;
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
  const out: ManagedSettingsBody = { updates: null, proxy: null, ai: null, transfers: null, mounts: null, workflows: normalizeWorkflows(b.workflows), backupTemplates: normalizeBackupTemplates(b.backupTemplates), notificationProfiles: normalizeNotificationProfiles(b.notificationProfiles), serverAccess: normalizeServerAccess(b.serverAccess) };
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
  if (b.mounts) {
    const m = b.mounts as Record<string, unknown>;
    const positive = (v: unknown, what: string): number | null => {
      if (v === null || v === undefined || v === '') return null;
      if (!Number.isInteger(v) || (v as number) < 1) throw new OrgError(`${what} must be a whole number of at least 1`);
      return v as number;
    };
    let allowedProtocols: string[] | null = null;
    if (m.allowedProtocols !== null && m.allowedProtocols !== undefined) {
      if (!Array.isArray(m.allowedProtocols)) throw new OrgError('Allowed protocols must be a list');
      allowedProtocols = [...new Set(m.allowedProtocols.map((p) => String(p).trim().toLowerCase()).filter(Boolean))];
      const unknown = allowedProtocols.filter((p) => !(MOUNT_PROTOCOLS as readonly string[]).includes(p));
      if (unknown.length) throw new OrgError(`Unknown mount protocol: ${unknown.join(', ')}`);
    }
    const cacheMode = m.cacheMode === null || m.cacheMode === undefined || m.cacheMode === '' ? null : m.cacheMode;
    if (cacheMode !== null && !(MOUNT_CACHE_MODES as readonly unknown[]).includes(cacheMode)) throw new OrgError(`The mount cache mode is ${MOUNT_CACHE_MODES.join(', ')}`);
    const root = str(m.mountRoot, 1000);
    out.mounts = {
      allowedProtocols,
      readOnly: triBool(m.readOnly, 'Read-only mounts'),
      cacheMode: cacheMode as ManagedMounts['cacheMode'],
      maxCacheMb: positive(m.maxCacheMb, 'The largest mount cache (MB)'),
      maxCacheAgeHours: positive(m.maxCacheAgeHours, 'How long cached files are kept (hours)'),
      mountRoot: root || null,
    };
    if (Object.values(out.mounts).every((v) => v === null)) out.mounts = null;
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
  const b = parseJson<ManagedSettingsBody>(r.settings, { updates: null, proxy: null, ai: null, transfers: null, mounts: null, workflows: null, backupTemplates: null, notificationProfiles: null, serverAccess: null });
  return { updates: b.updates ?? null, proxy: b.proxy ?? null, ai: b.ai ?? null, transfers: b.transfers ?? null, mounts: b.mounts ?? null, workflows: b.workflows ?? null, backupTemplates: b.backupTemplates ?? null, notificationProfiles: b.notificationProfiles ?? null, serverAccess: b.serverAccess ?? null, version: Number(r.version), updatedBy: r.updated_by as string, updatedAt: r.updated_at as string };
}

/**
 * Save. An empty body means nothing is managed (agents release the
 * settings); the row keeps its version so the next change is newer still.
 */
export function setManagedSettings(orgId: string, raw: unknown, by: string): ManagedSettings | null {
  const body = normalizeManagedSettings(raw);
  const empty = !body.updates && !body.proxy && !body.ai && !body.transfers && !body.mounts && !body.workflows && !body.backupTemplates && !body.notificationProfiles && !body.serverAccess;
  const version = Number(row(orgId)?.version ?? 0) + 1;
  db()
    .prepare('INSERT INTO managed_settings (org_id, settings, version, updated_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(org_id) DO UPDATE SET settings = excluded.settings, version = excluded.version, updated_by = excluded.updated_by, updated_at = excluded.updated_at')
    .run(orgId, empty ? 'null' : JSON.stringify(body), version, by, now());
  return getManagedSettings(orgId);
}

/** What the agent receives: `null` when nothing is managed. */
export function managedSettingsForAgent(orgId: string): (ManagedSettingsBody & { version: number }) | null {
  const s = getManagedSettings(orgId);
  return s && { version: s.version, updates: s.updates, proxy: s.proxy, ai: s.ai, transfers: s.transfers, mounts: s.mounts, workflows: s.workflows, backupTemplates: s.backupTemplates, notificationProfiles: s.notificationProfiles, serverAccess: s.serverAccess };
}
