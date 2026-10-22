/**
 * How long the console keeps each kind of log (Settings → Retention). The
 * settings are console-wide — the audit log is one hash chain across
 * organizations — and are changed by owners of top-level organizations.
 *
 * The audit log is pruned only at a signed anchor (`pruneAudit`), never past
 * what SIEM destinations still have to forward, and never when the chain
 * does not verify. The other logs are plain time-based deletes; their
 * per-item caps (deliveries per webhook, runs per organization, …) still apply.
 */
import { db, now, parseJson, type Row } from './db';
import { pruneAudit } from './db/repos/audit';
import { OrgError } from './db/repos/orgs';

export interface LogType {
  label: string;
  description: string;
  /** Days; 0 = keep forever (only where allowed). */
  default: number;
  min: number;
  max: number;
  allowForever: boolean;
}

export const LOG_TYPES = {
  audit: { label: 'Audit log', description: 'Pruned at the newest signed checkpoint older than this; the chain stays verifiable.', default: 0, min: 365, max: 3650, allowForever: true },
  backup_reports: { label: 'Backup reports', description: 'Individual backup results from machines (the latest state of each backup is kept).', default: 365, min: 30, max: 3650, allowForever: true },
  ai_transcripts: { label: 'AI transcripts', description: 'Full exchanges with the model; their hashes stay on the audit log.', default: 365, min: 30, max: 3650, allowForever: true },
  report_runs: { label: 'Report runs', description: 'Stored results of report runs.', default: 90, min: 7, max: 3650, allowForever: false },
  api_requests: { label: 'API request log', description: 'Requests made with API tokens and refused requests.', default: 30, min: 7, max: 365, allowForever: false },
  sso_attempts: { label: 'SSO diagnostics', description: 'Step-by-step records of single sign-on attempts.', default: 90, min: 7, max: 365, allowForever: false },
  webhook_deliveries: { label: 'Webhook deliveries', description: 'Finished webhook deliveries and their responses.', default: 30, min: 7, max: 365, allowForever: false },
  outbox: { label: 'Notification outbox', description: 'Sent, failed and discarded notifications.', default: 30, min: 7, max: 365, allowForever: false },
  diagnostics: { label: 'Machine diagnostics', description: 'Diagnostic bundles uploaded by machines.', default: 90, min: 7, max: 3650, allowForever: false },
  sessions: { label: 'Ended sessions', description: 'Signed-out, revoked and expired console sessions.', default: 30, min: 7, max: 365, allowForever: false },
} satisfies Record<string, LogType>;

export type LogTypeKey = keyof typeof LOG_TYPES;
export type RetentionSettings = Record<LogTypeKey, number>;

const KEY = 'retention';

export function getRetention(): RetentionSettings {
  const r = db().prepare('SELECT value FROM console_settings WHERE key = ?').get(KEY) as Row | undefined;
  const saved = parseJson<Partial<RetentionSettings>>(r?.value, {});
  return Object.fromEntries((Object.keys(LOG_TYPES) as LogTypeKey[]).map((k) => [k, saved[k] ?? LOG_TYPES[k].default])) as RetentionSettings;
}

export function setRetention(patch: Partial<Record<string, unknown>>, by: string): RetentionSettings {
  const next = getRetention();
  for (const [k, v] of Object.entries(patch)) {
    const t = LOG_TYPES[k as LogTypeKey];
    if (!t) throw new OrgError(`Unknown log type ${k}`);
    const days = Number(v);
    const ok = Number.isInteger(days) && ((days === 0 && t.allowForever) || (days >= t.min && days <= t.max));
    if (!ok) throw new OrgError(`${t.label}: ${t.allowForever ? '0 (keep forever) or ' : ''}${t.min} to ${t.max} days`);
    next[k as LogTypeKey] = days;
  }
  db()
    .prepare(
      'INSERT INTO console_settings (key, value, updated_by, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_by = excluded.updated_by, updated_at = excluded.updated_at',
    )
    .run(KEY, JSON.stringify(next), by, now());
  return next;
}

/** Plain logs: table, time column and which rows are finished (safe to delete). */
const PLAIN: Record<Exclude<LogTypeKey, 'audit'>, { table: string; column: string; where?: string }> = {
  backup_reports: { table: 'backup_reports', column: 'finished_at' },
  ai_transcripts: { table: 'ai_transcripts', column: 'created_at' },
  report_runs: { table: 'report_runs', column: 'started_at', where: "status != 'running'" },
  api_requests: { table: 'api_requests', column: 'at' },
  sso_attempts: { table: 'sso_attempts', column: 'at' },
  webhook_deliveries: { table: 'webhook_deliveries', column: 'created_at', where: "status != 'pending'" },
  outbox: { table: 'channel_outbox', column: 'created_at', where: "status != 'pending'" },
  diagnostics: { table: 'machine_diagnostics', column: 'created_at' },
  // A revoked session counts from its revocation, others from their expiry (active ones expire in the future).
  sessions: { table: 'sessions', column: 'COALESCE(revoked_at, expires_at)' },
};

/** Entries SIEM destinations still have to forward are kept. */
function siemLimit(): number | undefined {
  const r = db().prepare('SELECT MIN(cursor) AS c FROM siem_destinations WHERE enabled = 1').get() as Row;
  return r.c === null || r.c === undefined ? undefined : Number(r.c);
}

/** Apply the retention settings once; returns how many items each log lost. */
export function applyRetention(at = new Date(), by = 'system'): Partial<Record<LogTypeKey, number>> {
  const s = getRetention();
  const out: Partial<Record<LogTypeKey, number>> = {};
  const cutoff = (days: number) => new Date(at.getTime() - days * 86_400_000).toISOString();
  for (const [k, p] of Object.entries(PLAIN) as [Exclude<LogTypeKey, 'audit'>, (typeof PLAIN)[keyof typeof PLAIN]][]) {
    if (!s[k]) continue;
    const n = db()
      .prepare(`DELETE FROM ${p.table} WHERE ${p.column} < ?${p.where ? ` AND (${p.where})` : ''}`)
      .run(cutoff(s[k])).changes;
    if (n) out[k] = n;
  }
  if (s.audit) {
    const n = pruneAudit(new Date(cutoff(s.audit)), { by, limitSequence: siemLimit() });
    if (n) out.audit = n;
  }
  return out;
}
