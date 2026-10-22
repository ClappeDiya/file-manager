/**
 * Notification outbox: every alert, summary and invitation message sent
 * through a channel (email, Slack, webhook) is recorded here. The first
 * attempt happens right away; failures are retried on the webhook backoff
 * schedule and then marked failed. Administrators see the outbox and can
 * retry or discard messages.
 */
import { db, newId, now, parseJson, type Row } from '../index';
import { BACKOFF_SECONDS, MAX_ATTEMPTS } from '../../retry';

export type OutboxStatus = 'pending' | 'sent' | 'dead' | 'discarded';
export const OUTBOX_STATUSES: OutboxStatus[] = ['pending', 'sent', 'dead', 'discarded'];
/** Sent and discarded messages are kept this long. */
export const KEEP_DAYS = 30;

export interface OutboxMessage {
  subject: string;
  text: string;
  data?: Record<string, unknown>;
  to?: string[];
  /** Kept so retries send the same attachments (report runs cap them at 1 MB). */
  attachments?: { filename: string; content: string; contentType: string }[];
}

export interface OutboxEntry {
  id: string;
  orgId: string;
  channelId: string;
  kind: string;
  subject: string;
  message: OutboxMessage;
  context: string;
  status: OutboxStatus;
  attempts: number;
  nextAttemptAt: string | null;
  lastError: string | null;
  createdAt: string;
  sentAt: string | null;
}

function fromRow(r: Row): OutboxEntry {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    channelId: r.channel_id as string,
    kind: r.kind as string,
    subject: r.subject as string,
    message: parseJson<OutboxMessage>(r.message, { subject: '', text: '' }),
    context: r.context as string,
    status: r.status as OutboxStatus,
    attempts: Number(r.attempts),
    nextAttemptAt: (r.next_attempt_at as string) ?? null,
    lastError: (r.last_error as string) ?? null,
    createdAt: r.created_at as string,
    sentAt: (r.sent_at as string) ?? null,
  };
}

export function addToOutbox(orgId: string, channelId: string, kind: string, m: OutboxMessage, context = ''): OutboxEntry {
  const id = newId('obx');
  db()
    .prepare("INSERT INTO channel_outbox (id, org_id, channel_id, kind, subject, message, context, status, next_attempt_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)")
    .run(id, orgId, channelId, kind, m.subject.slice(0, 300), JSON.stringify(m), context.slice(0, 200), now(), now());
  return getOutboxEntry(orgId, id)!;
}

export function getOutboxEntry(orgId: string, id: string): OutboxEntry | null {
  const r = db().prepare('SELECT * FROM channel_outbox WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** Record one attempt: sent, or retried later, or failed for good. */
export function recordOutboxAttempt(id: string, error: string | null, at = new Date()): OutboxStatus {
  const r = db().prepare('SELECT attempts FROM channel_outbox WHERE id = ?').get(id) as { attempts: number } | undefined;
  if (!r) return 'discarded';
  const attempts = r.attempts + 1;
  if (!error) {
    db().prepare("UPDATE channel_outbox SET status = 'sent', attempts = ?, last_error = NULL, sent_at = ?, next_attempt_at = NULL WHERE id = ?").run(attempts, at.toISOString(), id);
    return 'sent';
  }
  const dead = attempts >= MAX_ATTEMPTS;
  db()
    .prepare('UPDATE channel_outbox SET status = ?, attempts = ?, last_error = ?, next_attempt_at = ? WHERE id = ?')
    .run(dead ? 'dead' : 'pending', attempts, error.slice(0, 500), dead ? null : new Date(at.getTime() + BACKOFF_SECONDS[attempts - 1] * 1000).toISOString(), id);
  return dead ? 'dead' : 'pending';
}

export function dueOutbox(at: Date, limit = 50): OutboxEntry[] {
  return (db().prepare("SELECT * FROM channel_outbox WHERE status = 'pending' AND next_attempt_at <= ? ORDER BY next_attempt_at, created_at LIMIT ?").all(at.toISOString(), limit) as Row[]).map(fromRow);
}

export function listOutbox(orgId: string, o: { status?: OutboxStatus; channelId?: string; limit?: number } = {}): OutboxEntry[] {
  const where = ['org_id = ?'];
  const args: unknown[] = [orgId];
  if (o.status) {
    where.push('status = ?');
    args.push(o.status);
  }
  if (o.channelId) {
    where.push('channel_id = ?');
    args.push(o.channelId);
  }
  args.push(Math.min(o.limit ?? 100, 500));
  return (db().prepare(`SELECT * FROM channel_outbox WHERE ${where.join(' AND ')} ORDER BY created_at DESC, rowid DESC LIMIT ?`).all(...args) as Row[]).map(fromRow);
}

export function outboxCounts(orgId: string): Record<OutboxStatus, number> {
  const out: Record<OutboxStatus, number> = { pending: 0, sent: 0, dead: 0, discarded: 0 };
  for (const r of db().prepare('SELECT status, COUNT(*) AS n FROM channel_outbox WHERE org_id = ? GROUP BY status').all(orgId) as { status: OutboxStatus; n: number }[]) out[r.status] = r.n;
  return out;
}

/** Try again now (pending or failed messages), with a fresh attempt count. */
export function retryOutbox(orgId: string, id: string): boolean {
  return db().prepare("UPDATE channel_outbox SET status = 'pending', attempts = 0, next_attempt_at = ? WHERE org_id = ? AND id = ? AND status IN ('pending', 'dead', 'discarded')").run(now(), orgId, id).changes > 0;
}

export function discardOutbox(orgId: string, id: string): boolean {
  return db().prepare("UPDATE channel_outbox SET status = 'discarded', next_attempt_at = NULL WHERE org_id = ? AND id = ? AND status IN ('pending', 'dead')").run(orgId, id).changes > 0;
}

/** Drop sent/discarded messages older than `days` (scheduled cleanup uses the retention settings, `lib/retention`). */
export function pruneOutbox(days = KEEP_DAYS, at = new Date()): number {
  return db().prepare("DELETE FROM channel_outbox WHERE status IN ('sent', 'discarded') AND created_at < ?").run(new Date(at.getTime() - days * 86_400_000).toISOString()).changes;
}
