/**
 * Outbound webhooks: per-organization subscriptions to console events, and
 * a persisted delivery queue (retries with backoff, automatic pause after
 * repeated failures, a per-delivery log and redelivery).
 *
 * Events are the audit event types (`machine.register`, `credential.rotate`,
 * …) plus domain events (`backup.success|warning|error|cancelled`,
 * `alert.opened`, `alert.resolved`). A subscription matches exact names,
 * `prefix.*` or `*`. Sending lives in `lib/delivery.ts`.
 */
import { db, newId, newToken, now, parseJson, type Row } from '../index';
import { open, seal } from '../../crypto';
import { OrgError } from './orgs';

export type DeliveryStatus = 'pending' | 'succeeded' | 'dead';

export interface Webhook {
  id: string;
  orgId: string;
  name: string;
  url: string;
  events: string[];
  headers: Record<string, string>;
  enabled: boolean;
  pausedReason: string | null;
  consecutiveFailures: number;
  lastSuccessAt: string | null;
  lastFailureAt: string | null;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
}

export interface WebhookDelivery {
  id: string;
  webhookId: string;
  event: string;
  payload: Record<string, unknown>;
  status: DeliveryStatus;
  attempts: number;
  nextAttemptAt: string | null;
  lastStatus: number | null;
  lastError: string | null;
  durationMs: number | null;
  createdAt: string;
  deliveredAt: string | null;
}

import { BACKOFF_SECONDS, MAX_ATTEMPTS } from '../../retry';
/** The shared schedule (see `lib/retry`); after MAX_ATTEMPTS the delivery is dead. */
export { BACKOFF_SECONDS, MAX_ATTEMPTS };
/** Consecutive failed attempts after which a webhook is paused. */
export const PAUSE_AFTER_FAILURES = 15;
/** Delivery log kept per webhook. */
export const KEEP_DELIVERIES = 500;

/** Events offered in the UI (subscriptions may also use any audit event type or wildcard). */
export const EVENT_CATALOG: { group: string; events: string[] }[] = [
  { group: 'Backups', events: ['backup.success', 'backup.warning', 'backup.error', 'backup.cancelled'] },
  { group: 'Alerts', events: ['alert.opened', 'alert.resolved'] },
  { group: 'Machines', events: ['machine.register', 'machine.claim', 'machine.update', 'machine.suspend', 'machine.resume', 'machine.revoke', 'machine.unregister', 'machine.delete', 'machine.rotate_token'] },
  { group: 'Fleet', events: ['fleet.policy_create', 'fleet.policy_update', 'fleet.policy_delete', 'policy.apply_failed', 'fleet.managed_settings', 'credential.create', 'credential.rotate', 'credential.delete', 'credential.deliver'] },
  { group: 'Users & access', events: ['user.create', 'user.update', 'user.deactivate', 'user.delete', 'auth.login', 'auth.login_failed', 'auth.mfa_enroll'] },
  { group: 'Remote', events: ['remote.command', 'remote.command_cancel'] },
  { group: 'Billing', events: ['billing.subscription', 'billing.trial_will_end', 'billing.invoice_paid', 'billing.payment_failed'] },
  { group: 'Webhooks', events: ['webhook.test'] },
];

const EVENT_RE = /^(\*|[a-z][a-z0-9_]*(\.[a-z0-9_]+)*(\.\*)?)$/;
const RESERVED_HEADERS = new Set(['content-type', 'content-length', 'host', 'user-agent', 'x-ufop-event', 'x-ufop-delivery', 'x-ufop-signature', 'x-ufop-timestamp']);

export function eventMatches(patterns: string[], event: string): boolean {
  return patterns.some((p) => p === '*' || p === event || (p.endsWith('.*') && event.startsWith(p.slice(0, -1))));
}

function fromRow(r: Row): Webhook {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    url: r.url as string,
    events: parseJson<string[]>(r.events, []),
    headers: parseJson<Record<string, string>>(r.headers, {}),
    enabled: Boolean(r.enabled),
    pausedReason: (r.paused_reason as string) ?? null,
    consecutiveFailures: Number(r.consecutive_failures),
    lastSuccessAt: (r.last_success_at as string) ?? null,
    lastFailureAt: (r.last_failure_at as string) ?? null,
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    updatedAt: r.updated_at as string,
  };
}

function deliveryFromRow(r: Row): WebhookDelivery {
  return {
    id: r.id as string,
    webhookId: r.webhook_id as string,
    event: r.event as string,
    payload: parseJson(r.payload, {}),
    status: r.status as DeliveryStatus,
    attempts: Number(r.attempts),
    nextAttemptAt: (r.next_attempt_at as string) ?? null,
    lastStatus: (r.last_status as number) ?? null,
    lastError: (r.last_error as string) ?? null,
    durationMs: (r.duration_ms as number) ?? null,
    createdAt: r.created_at as string,
    deliveredAt: (r.delivered_at as string) ?? null,
  };
}

export interface WebhookInput {
  name?: string;
  url?: string;
  events?: string[];
  headers?: Record<string, string>;
  enabled?: boolean;
}

function validate(w: WebhookInput, existing?: Webhook): { name: string; url: string; events: string[]; headers: Record<string, string> } {
  const name = (w.name ?? existing?.name ?? '').trim().slice(0, 120);
  if (!name) throw new OrgError('Give the webhook a name');
  const url = (w.url ?? existing?.url ?? '').trim();
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    throw new OrgError('Enter a valid URL');
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') throw new OrgError('The URL must be http(s)');
  if (u.username || u.password) throw new OrgError('Put credentials in a header, not in the URL');
  const events = [...new Set((w.events ?? existing?.events ?? []).map((e) => String(e).trim()).filter(Boolean))];
  if (!events.length) throw new OrgError('Choose at least one event');
  const bad = events.find((e) => !EVENT_RE.test(e));
  if (bad) throw new OrgError(`'${bad}' is not an event name or pattern`);
  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(w.headers ?? existing?.headers ?? {})) {
    const key = k.trim().toLowerCase();
    if (!/^[a-z0-9-]{1,64}$/.test(key)) throw new OrgError(`'${k}' is not a valid header name`);
    if (RESERVED_HEADERS.has(key)) throw new OrgError(`The '${key}' header is set by the console`);
    if (typeof v !== 'string' || /[\r\n]/.test(v) || v.length > 2000) throw new OrgError(`The value of '${k}' is not valid`);
    headers[key] = v;
  }
  if (Object.keys(headers).length > 20) throw new OrgError('At most 20 custom headers');
  return { name, url, events, headers };
}

export function listWebhooks(orgId: string): Webhook[] {
  return (db().prepare('SELECT * FROM webhooks WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

export function getWebhook(orgId: string, id: string): Webhook | null {
  const r = db().prepare('SELECT * FROM webhooks WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** Create; returns the signing secret once. */
export function createWebhook(orgId: string, w: WebhookInput, by: string): { webhook: Webhook; secret: string } {
  const v = validate(w);
  const id = newId('whk');
  const secret = newToken('whsec', 24);
  const ts = now();
  db()
    .prepare('INSERT INTO webhooks (id, org_id, name, url, events, headers, secret_ciphertext, enabled, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, v.name, v.url, JSON.stringify(v.events), JSON.stringify(v.headers), seal(secret), w.enabled === false ? 0 : 1, by, ts, ts);
  return { webhook: getWebhook(orgId, id)!, secret };
}

export function updateWebhook(orgId: string, id: string, w: WebhookInput): Webhook | null {
  const existing = getWebhook(orgId, id);
  if (!existing) return null;
  const v = validate(w, existing);
  const enabled = w.enabled ?? existing.enabled;
  db()
    .prepare('UPDATE webhooks SET name = ?, url = ?, events = ?, headers = ?, enabled = ?, paused_reason = CASE WHEN ? THEN NULL ELSE paused_reason END, consecutive_failures = CASE WHEN ? THEN 0 ELSE consecutive_failures END, updated_at = ? WHERE id = ?')
    .run(v.name, v.url, JSON.stringify(v.events), JSON.stringify(v.headers), enabled ? 1 : 0, enabled ? 1 : 0, enabled && !existing.enabled ? 1 : 0, now(), id);
  return getWebhook(orgId, id);
}

export function deleteWebhook(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM webhooks WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

export function rotateWebhookSecret(orgId: string, id: string): string | null {
  const secret = newToken('whsec', 24);
  return db().prepare('UPDATE webhooks SET secret_ciphertext = ?, updated_at = ? WHERE org_id = ? AND id = ?').run(seal(secret), now(), orgId, id).changes ? secret : null;
}

/** Sending only. */
export function webhookSecret(id: string): string | null {
  const r = db().prepare('SELECT secret_ciphertext FROM webhooks WHERE id = ?').get(id) as { secret_ciphertext: string } | undefined;
  return r ? open(r.secret_ciphertext) : null;
}

function insertDelivery(h: Webhook, event: string, payload: Record<string, unknown>): string {
  const id = newId('dlv');
  const body = { id, event, occurredAt: now(), organizationId: h.orgId, data: payload };
  db()
    .prepare("INSERT INTO webhook_deliveries (id, webhook_id, org_id, event, payload, status, next_attempt_at, created_at) VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)")
    .run(id, h.id, h.orgId, event, JSON.stringify(body), now(), now());
  return id;
}

let onEnqueue: (() => void) | null = null;
/** The delivery service asks to be told when new work is queued. */
export function setEnqueueListener(fn: (() => void) | null): void {
  onEnqueue = fn;
}

/** Queue `event` for every enabled, unpaused webhook of the organization that subscribes to it. */
export function enqueueEvent(orgId: string, event: string, data: Record<string, unknown>): number {
  if (!orgId) return 0;
  const hooks = listWebhooks(orgId).filter((h) => h.enabled && !h.pausedReason && eventMatches(h.events, event));
  for (const h of hooks) insertDelivery(h, event, data);
  if (hooks.length) onEnqueue?.();
  return hooks.length;
}

/** A test delivery, regardless of the subscription (but not of the pause). */
export function enqueueTest(h: Webhook, by: string): string {
  const id = insertDelivery(h, 'webhook.test', { message: 'Test delivery from the UFOP console', requestedBy: by });
  onEnqueue?.();
  return id;
}

/** Pending deliveries due at `at`, oldest first, for webhooks that are not paused. */
export function dueDeliveries(at: Date, limit = 50): (WebhookDelivery & { webhook: Webhook })[] {
  const rows = db()
    .prepare(
      "SELECT d.*, w.id AS w_id FROM webhook_deliveries d JOIN webhooks w ON w.id = d.webhook_id WHERE d.status = 'pending' AND d.next_attempt_at <= ? AND w.enabled = 1 AND w.paused_reason IS NULL ORDER BY d.next_attempt_at, d.created_at LIMIT ?",
    )
    .all(at.toISOString(), limit) as Row[];
  return rows.map((r) => ({ ...deliveryFromRow(r), webhook: fromRow(db().prepare('SELECT * FROM webhooks WHERE id = ?').get(r.w_id) as Row) }));
}

/** Record one attempt; schedules the retry or gives up, and pauses a webhook that keeps failing. */
export function recordAttempt(d: WebhookDelivery, r: { ok: boolean; status: number | null; error: string | null; durationMs: number }, at = new Date()): DeliveryStatus {
  const attempts = d.attempts + 1;
  const ts = at.toISOString();
  if (r.ok) {
    db().prepare("UPDATE webhook_deliveries SET status = 'succeeded', attempts = ?, last_status = ?, last_error = NULL, duration_ms = ?, delivered_at = ?, next_attempt_at = NULL WHERE id = ?").run(attempts, r.status, r.durationMs, ts, d.id);
    db().prepare('UPDATE webhooks SET consecutive_failures = 0, last_success_at = ? WHERE id = ?').run(ts, d.webhookId);
    return 'succeeded';
  }
  const dead = attempts >= MAX_ATTEMPTS;
  const next = dead ? null : new Date(at.getTime() + BACKOFF_SECONDS[attempts - 1] * 1000).toISOString();
  db()
    .prepare('UPDATE webhook_deliveries SET status = ?, attempts = ?, last_status = ?, last_error = ?, duration_ms = ?, next_attempt_at = ? WHERE id = ?')
    .run(dead ? 'dead' : 'pending', attempts, r.status, (r.error ?? '').slice(0, 500), r.durationMs, next, d.id);
  db().prepare('UPDATE webhooks SET consecutive_failures = consecutive_failures + 1, last_failure_at = ? WHERE id = ?').run(ts, d.webhookId);
  const h = db().prepare('SELECT consecutive_failures FROM webhooks WHERE id = ?').get(d.webhookId) as { consecutive_failures: number };
  if (h.consecutive_failures >= PAUSE_AFTER_FAILURES) {
    db().prepare('UPDATE webhooks SET paused_reason = ? WHERE id = ? AND paused_reason IS NULL').run(`Paused after ${h.consecutive_failures} failed deliveries in a row`, d.webhookId);
  }
  return dead ? 'dead' : 'pending';
}

export function listDeliveries(orgId: string, webhookId: string, o: { status?: DeliveryStatus; limit?: number; offset?: number } = {}): WebhookDelivery[] {
  const where = o.status ? 'AND status = ?' : '';
  const args: unknown[] = [orgId, webhookId, ...(o.status ? [o.status] : []), Math.min(o.limit ?? 100, 500), o.offset ?? 0];
  return (db().prepare(`SELECT * FROM webhook_deliveries WHERE org_id = ? AND webhook_id = ? ${where} ORDER BY created_at DESC, rowid DESC LIMIT ? OFFSET ?`).all(...args) as Row[]).map(deliveryFromRow);
}

/** Send a delivery again (any status): a fresh attempt now, attempts counted anew. */
export function redeliver(orgId: string, webhookId: string, deliveryId: string): boolean {
  const ok = db().prepare("UPDATE webhook_deliveries SET status = 'pending', attempts = 0, next_attempt_at = ? WHERE org_id = ? AND webhook_id = ? AND id = ?").run(now(), orgId, webhookId, deliveryId).changes > 0;
  if (ok) onEnqueue?.();
  return ok;
}

/** Resume a paused webhook; its pending deliveries continue. */
export function resumeWebhook(orgId: string, id: string): boolean {
  const ok = db().prepare('UPDATE webhooks SET paused_reason = NULL, consecutive_failures = 0, updated_at = ? WHERE org_id = ? AND id = ?').run(now(), orgId, id).changes > 0;
  if (ok) onEnqueue?.();
  return ok;
}

export function pauseWebhook(orgId: string, id: string, reason: string): boolean {
  return db().prepare('UPDATE webhooks SET paused_reason = ?, updated_at = ? WHERE org_id = ? AND id = ?').run(reason.slice(0, 200) || 'Paused', now(), orgId, id).changes > 0;
}

/** Keep the newest deliveries per webhook. */
export function pruneDeliveries(keep = KEEP_DELIVERIES): number {
  return db()
    .prepare(
      `DELETE FROM webhook_deliveries WHERE status != 'pending' AND id IN (
         SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY webhook_id ORDER BY created_at DESC, rowid DESC) AS n FROM webhook_deliveries) WHERE n > ?)`,
    )
    .run(keep).changes;
}

export function deliveryStats(orgId: string, webhookId: string): Record<DeliveryStatus, number> {
  const out: Record<DeliveryStatus, number> = { pending: 0, succeeded: 0, dead: 0 };
  for (const r of db().prepare('SELECT status, COUNT(*) AS n FROM webhook_deliveries WHERE org_id = ? AND webhook_id = ? GROUP BY status').all(orgId, webhookId) as { status: DeliveryStatus; n: number }[]) out[r.status] = r.n;
  return out;
}
