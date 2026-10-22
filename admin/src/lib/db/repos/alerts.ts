/** Alert rules and alert instances. Evaluation lives in lib/alerts.ts. */
import { db, newId, now, parseJson, type Row } from '../index';
import { OrgError } from './orgs';
import { normalizeTags } from './machines';

export type AlertKind = 'backup_failed' | 'missed_backup' | 'changed_files' | 'duration_anomaly';
export const ALERT_KINDS: AlertKind[] = ['backup_failed', 'missed_backup', 'changed_files', 'duration_anomaly'];

export interface AlertParams {
  /** backup_failed: also alert on warnings. */
  includeWarnings?: boolean;
  /** missed_backup: minutes of slack after the expected run. */
  graceMinutes?: number;
  /** missed_backup: interval to assume when the client reports no schedule (hours; 0 = skip). */
  defaultIntervalHours?: number;
  /** changed_files: thresholds (any exceeded triggers). */
  maxChangedFiles?: number;
  maxChangedPercent?: number;
  maxDeletedFiles?: number;
  /** duration_anomaly. */
  minSamples?: number;
  factor?: number;
  stdDevs?: number;
  detectShort?: boolean;
  /** Notify when the alert resolves. */
  notifyOnResolve?: boolean;
}

export interface AlertScope {
  tags: string[];
  machineIds: string[];
}

export interface AlertRule {
  id: string;
  orgId: string;
  name: string;
  kind: AlertKind;
  params: AlertParams;
  scope: AlertScope;
  channelIds: string[];
  enabled: boolean;
  cooldownMinutes: number;
  createdAt: string;
}

function ruleFromRow(r: Row): AlertRule {
  const s = parseJson<Partial<AlertScope>>(r.scope, {});
  return { id: r.id as string, orgId: r.org_id as string, name: r.name as string, kind: r.kind as AlertKind, params: parseJson(r.params, {}), scope: { tags: s.tags ?? [], machineIds: s.machineIds ?? [] }, channelIds: parseJson<string[]>(r.channel_ids, []), enabled: Boolean(r.enabled), cooldownMinutes: Number(r.cooldown_minutes), createdAt: r.created_at as string };
}

export interface RuleInput {
  name: string;
  kind: AlertKind;
  params?: AlertParams;
  scope?: Partial<AlertScope>;
  channelIds?: string[];
  enabled?: boolean;
  cooldownMinutes?: number;
}

function validate(orgId: string, r: RuleInput) {
  if (!r.name?.trim()) throw new OrgError('Give the rule a name');
  if (!ALERT_KINDS.includes(r.kind)) throw new OrgError('Unknown alert type');
  const p = r.params ?? {};
  if (r.kind === 'changed_files' && !p.maxChangedFiles && !p.maxChangedPercent && !p.maxDeletedFiles) throw new OrgError('Set at least one changed-file threshold');
  for (const k of ['graceMinutes', 'defaultIntervalHours', 'maxChangedFiles', 'maxChangedPercent', 'maxDeletedFiles', 'minSamples', 'factor', 'stdDevs'] as const) {
    if (p[k] !== undefined && (typeof p[k] !== 'number' || (p[k] as number) < 0)) throw new OrgError(`${k} must be a non-negative number`);
  }
  for (const id of r.channelIds ?? []) if (!db().prepare('SELECT 1 FROM notification_channels WHERE org_id = ? AND id = ?').get(orgId, id)) throw new OrgError(`Channel ${id} does not exist`);
}

export function createRule(orgId: string, r: RuleInput): AlertRule {
  validate(orgId, r);
  const id = newId('alr');
  db()
    .prepare('INSERT INTO alert_rules (id, org_id, name, kind, params, scope, channel_ids, enabled, cooldown_minutes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, r.name.trim(), r.kind, JSON.stringify(r.params ?? {}), JSON.stringify({ tags: normalizeTags(r.scope?.tags ?? []), machineIds: r.scope?.machineIds ?? [] }), JSON.stringify(r.channelIds ?? []), r.enabled === false ? 0 : 1, Math.max(0, r.cooldownMinutes ?? 360), now());
  return getRule(orgId, id)!;
}

export function updateRule(orgId: string, id: string, r: Partial<RuleInput>): AlertRule | null {
  const cur = getRule(orgId, id);
  if (!cur) return null;
  const next: RuleInput = { name: r.name ?? cur.name, kind: cur.kind, params: r.params ?? cur.params, scope: r.scope ?? cur.scope, channelIds: r.channelIds ?? cur.channelIds, enabled: r.enabled ?? cur.enabled, cooldownMinutes: r.cooldownMinutes ?? cur.cooldownMinutes };
  validate(orgId, next);
  db()
    .prepare('UPDATE alert_rules SET name = ?, params = ?, scope = ?, channel_ids = ?, enabled = ?, cooldown_minutes = ? WHERE org_id = ? AND id = ?')
    .run(next.name.trim(), JSON.stringify(next.params), JSON.stringify({ tags: normalizeTags(next.scope?.tags ?? []), machineIds: next.scope?.machineIds ?? [] }), JSON.stringify(next.channelIds), next.enabled ? 1 : 0, next.cooldownMinutes, orgId, id);
  return getRule(orgId, id);
}

export function getRule(orgId: string, id: string): AlertRule | null {
  const r = db().prepare('SELECT * FROM alert_rules WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? ruleFromRow(r) : null;
}

export function listRules(orgId?: string): AlertRule[] {
  const rows = orgId ? (db().prepare('SELECT * FROM alert_rules WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]) : (db().prepare('SELECT * FROM alert_rules').all() as Row[]);
  return rows.map(ruleFromRow);
}

export function deleteRule(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM alert_rules WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

// ── Alert instances ──

export interface Alert {
  id: string;
  orgId: string;
  ruleId: string | null;
  machineId: string | null;
  backupId: string | null;
  kind: AlertKind;
  severity: 'warning' | 'critical';
  message: string;
  details: Record<string, unknown>;
  fingerprint: string;
  createdAt: string;
  lastNotifiedAt: string | null;
  resolvedAt: string | null;
  notified: { channelId: string; ok: boolean; error?: string; at: string }[];
}

function alertFromRow(r: Row): Alert {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    ruleId: (r.rule_id as string) ?? null,
    machineId: (r.machine_id as string) ?? null,
    backupId: (r.backup_id as string) ?? null,
    kind: r.kind as AlertKind,
    severity: r.severity as Alert['severity'],
    message: r.message as string,
    details: parseJson(r.details, {}),
    fingerprint: r.fingerprint as string,
    createdAt: r.created_at as string,
    lastNotifiedAt: (r.last_notified_at as string) ?? null,
    resolvedAt: (r.resolved_at as string) ?? null,
    notified: parseJson(r.notified, []),
  };
}

export function openAlert(fingerprint: string): Alert | null {
  const r = db().prepare('SELECT * FROM alerts WHERE fingerprint = ? AND resolved_at IS NULL').get(fingerprint) as Row | undefined;
  return r ? alertFromRow(r) : null;
}

export function insertAlert(a: Omit<Alert, 'id' | 'createdAt' | 'lastNotifiedAt' | 'resolvedAt' | 'notified'>): Alert {
  const id = newId('alt');
  db()
    .prepare('INSERT INTO alerts (id, org_id, rule_id, machine_id, backup_id, kind, severity, message, details, fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, a.orgId, a.ruleId, a.machineId, a.backupId, a.kind, a.severity, a.message, JSON.stringify(a.details), a.fingerprint, now());
  return alertFromRow(db().prepare('SELECT * FROM alerts WHERE id = ?').get(id) as Row);
}

export function recordNotification(id: string, deliveries: { channelId: string; ok: boolean; error?: string }[]): void {
  const cur = alertFromRow(db().prepare('SELECT * FROM alerts WHERE id = ?').get(id) as Row);
  const at = now();
  db().prepare('UPDATE alerts SET notified = ?, last_notified_at = ? WHERE id = ?').run(JSON.stringify([...cur.notified, ...deliveries.map((d) => ({ ...d, at }))].slice(-50)), at, id);
}

export function resolveAlert(id: string): void {
  db().prepare('UPDATE alerts SET resolved_at = ? WHERE id = ? AND resolved_at IS NULL').run(now(), id);
}

export function listAlerts(o: { orgIds: string[]; open?: boolean; machineIds?: string[]; limit?: number }): Alert[] {
  if (!o.orgIds.length || (o.machineIds && !o.machineIds.length)) return [];
  const where = [`org_id IN (${o.orgIds.map(() => '?').join(',')})`];
  const args: unknown[] = [...o.orgIds];
  if (o.open !== undefined) where.push(o.open ? 'resolved_at IS NULL' : 'resolved_at IS NOT NULL');
  if (o.machineIds) { where.push(`machine_id IN (${o.machineIds.map(() => '?').join(',')})`); args.push(...o.machineIds); }
  return (db().prepare(`SELECT * FROM alerts WHERE ${where.join(' AND ')} ORDER BY created_at DESC LIMIT ?`).all(...args, Math.min(o.limit ?? 200, 2000)) as Row[]).map(alertFromRow);
}

export function getAlert(orgId: string, id: string): Alert | null {
  const r = db().prepare('SELECT * FROM alerts WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? alertFromRow(r) : null;
}
