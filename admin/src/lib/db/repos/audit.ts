/**
 * Append-only, hash-chained audit log.
 *
 * Every entry stores `sequence`, `previous_hash` and
 * `integrity_hash = SHA-256(canonical JSON of the entry + previous hash)`.
 * SQLite triggers (migration 6) reject UPDATE and DELETE. Signed anchors
 * (HMAC over the chain head) let an auditor detect truncation or a rebuilt
 * chain even by someone with write access to the file.
 */
import crypto from 'crypto';
import { db, newId, now, parseJson, tx, type Row } from '../index';
import { enqueueEvent } from './webhooks';
import { sign, timingSafeEqualHex } from '../../crypto';

export interface AuditInput {
  eventType: string;
  severity?: 'info' | 'warning' | 'error' | 'critical';
  orgId?: string;
  userId?: string;
  userEmail?: string;
  userName?: string;
  ipAddress?: string;
  userAgent?: string;
  description: string;
  resourceType?: string;
  resourceId?: string;
  metadata?: Record<string, unknown>;
}

export interface AuditRecord {
  id: string;
  sequence: number;
  eventType: string;
  severity: string;
  timestamp: string;
  orgId: string;
  organizationId: string;
  userId: string;
  userEmail: string;
  userName: string;
  ipAddress: string;
  userAgent: string;
  description: string;
  resourceType?: string;
  resourceId?: string;
  metadata: Record<string, unknown>;
  integrityHash: string;
  previousHash: string;
}

/** Deterministic JSON: object keys sorted at every level. */
export function canonical(v: unknown): string {
  if (v === null || typeof v !== 'object') return JSON.stringify(v ?? null);
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  const o = v as Record<string, unknown>;
  return `{${Object.keys(o)
    .filter((k) => o[k] !== undefined)
    .sort()
    .map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`)
    .join(',')}}`;
}

function hashOf(r: Omit<AuditRecord, 'integrityHash' | 'organizationId'>): string {
  const body = {
    id: r.id,
    sequence: r.sequence,
    eventType: r.eventType,
    severity: r.severity,
    timestamp: r.timestamp,
    orgId: r.orgId,
    userId: r.userId,
    userEmail: r.userEmail,
    ipAddress: r.ipAddress,
    userAgent: r.userAgent,
    description: r.description,
    metadata: r.metadata,
    previousHash: r.previousHash,
  };
  return crypto.createHash('sha256').update(canonical(body)).digest('hex');
}

function fromRow(r: Row): AuditRecord {
  const meta = parseJson<Record<string, unknown>>(r.metadata, {});
  return {
    id: r.id as string,
    sequence: Number(r.sequence ?? 0),
    eventType: r.event_type as string,
    severity: r.severity as string,
    timestamp: r.timestamp as string,
    orgId: (r.org_id as string) || '',
    organizationId: (r.org_id as string) || '',
    userId: (r.user_id as string) || '',
    userEmail: (r.user_email as string) || '',
    userName: (meta.userName as string) || (r.user_email as string) || '',
    ipAddress: (r.ip_address as string) || '',
    userAgent: (r.user_agent as string) || '',
    description: r.description as string,
    resourceType: meta.resourceType as string | undefined,
    resourceId: meta.resourceId as string | undefined,
    metadata: meta,
    integrityHash: (r.integrity_hash as string) || '',
    previousHash: (r.previous_hash as string) || '',
  };
}

export function appendAudit(e: AuditInput): AuditRecord {
  return tx(() => {
    const head = db().prepare('SELECT sequence, integrity_hash FROM audit_entries WHERE sequence IS NOT NULL ORDER BY sequence DESC LIMIT 1').get() as Row | undefined;
    const metadata: Record<string, unknown> = { ...(e.metadata ?? {}) };
    if (e.userName) metadata.userName = e.userName;
    if (e.resourceType) metadata.resourceType = e.resourceType;
    if (e.resourceId) metadata.resourceId = e.resourceId;
    const base = {
      id: newId('aud'),
      sequence: head ? Number(head.sequence) + 1 : 1,
      eventType: e.eventType,
      severity: e.severity ?? 'info',
      timestamp: now(),
      orgId: e.orgId ?? '',
      userId: e.userId ?? '',
      userEmail: e.userEmail ?? '',
      userName: e.userName ?? '',
      ipAddress: e.ipAddress ?? '',
      userAgent: e.userAgent ?? '',
      description: e.description,
      metadata,
      previousHash: head ? (head.integrity_hash as string) : '',
    };
    const integrityHash = hashOf(base);
    db()
      .prepare(
        `INSERT INTO audit_entries (id, sequence, event_type, severity, timestamp, org_id, user_id, user_email, ip_address, user_agent, description, metadata, integrity_hash, previous_hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(base.id, base.sequence, base.eventType, base.severity, base.timestamp, base.orgId, base.userId, base.userEmail, base.ipAddress, base.userAgent, base.description, JSON.stringify(metadata), integrityHash, base.previousHash);
    // Same transaction: an audited action and its webhook deliveries are recorded together.
    enqueueEvent(base.orgId, base.eventType, { auditId: base.id, severity: base.severity, description: base.description, actor: base.userEmail || null, resourceType: e.resourceType ?? null, resourceId: e.resourceId ?? null, metadata });
    return { ...base, organizationId: base.orgId, resourceType: e.resourceType, resourceId: e.resourceId, integrityHash };
  });
}

export interface AuditQuery {
  orgIds?: string[];
  eventType?: string;
  /** Match every event type starting with this prefix, e.g. `ai.`. */
  eventPrefix?: string;
  severity?: string;
  userId?: string;
  from?: string;
  to?: string;
  search?: string;
  limit?: number;
  offset?: number;
}

export function listAudit(q: AuditQuery = {}): { entries: AuditRecord[]; total: number } {
  const where: string[] = ['sequence IS NOT NULL'];
  const args: unknown[] = [];
  if (q.orgIds) {
    where.push(`org_id IN (${q.orgIds.map(() => '?').join(',') || "''"})`);
    args.push(...q.orgIds);
  }
  if (q.eventType) { where.push('event_type = ?'); args.push(q.eventType); }
  if (q.eventPrefix) { where.push("event_type LIKE ? ESCAPE '\\'"); args.push(`${q.eventPrefix.replace(/[\\%_]/g, (c) => `\\${c}`)}%`); }
  if (q.severity) { where.push('severity = ?'); args.push(q.severity); }
  if (q.userId) { where.push('user_id = ?'); args.push(q.userId); }
  if (q.from) { where.push('timestamp >= ?'); args.push(q.from); }
  if (q.to) { where.push('timestamp <= ?'); args.push(q.to); }
  if (q.search) {
    where.push('(lower(description) LIKE ? OR lower(event_type) LIKE ? OR lower(user_email) LIKE ?)');
    const s = `%${q.search.toLowerCase()}%`;
    args.push(s, s, s);
  }
  const w = where.join(' AND ');
  const total = (db().prepare(`SELECT COUNT(*) n FROM audit_entries WHERE ${w}`).get(...args) as { n: number }).n;
  const rows = db()
    .prepare(`SELECT * FROM audit_entries WHERE ${w} ORDER BY sequence DESC LIMIT ? OFFSET ?`)
    .all(...args, Math.min(q.limit ?? 50, 10_000), q.offset ?? 0) as Row[];
  return { entries: rows.map(fromRow), total };
}

/** Entries of these organizations after a sequence number, oldest first (forwarding). */
export function auditAfter(orgIds: string[], afterSequence: number, limit: number): AuditRecord[] {
  if (!orgIds.length) return [];
  const rows = db()
    .prepare(`SELECT * FROM audit_entries WHERE sequence > ? AND org_id IN (${orgIds.map(() => '?').join(',')}) ORDER BY sequence LIMIT ?`)
    .all(afterSequence, ...orgIds, limit) as Row[];
  return rows.map(fromRow);
}

export function countAuditAfter(orgIds: string[], afterSequence: number): number {
  if (!orgIds.length) return 0;
  return (db().prepare(`SELECT COUNT(*) AS n FROM audit_entries WHERE sequence > ? AND org_id IN (${orgIds.map(() => '?').join(',')})`).get(afterSequence, ...orgIds) as { n: number }).n;
}

/** The newest sequence number (0 when empty). */
export function auditHead(): number {
  return Number((db().prepare('SELECT MAX(sequence) AS s FROM audit_entries').get() as Row).s ?? 0);
}

export interface AuditVerification {
  ok: boolean;
  entriesChecked: number;
  firstBadSequence: number | null;
  reason: string | null;
  anchorsChecked: number;
  headSequence: number;
  headHash: string;
}

/** The most recent anchored prune: entries up to `throughSequence` were removed. */
export function lastAuditPrune(): { throughSequence: number; throughHash: string; anchorId: string; pruned: number; createdAt: string } | null {
  const r = db().prepare('SELECT * FROM audit_prunes ORDER BY through_sequence DESC LIMIT 1').get() as Row | undefined;
  return r ? { throughSequence: Number(r.through_sequence), throughHash: r.through_hash as string, anchorId: r.anchor_id as string, pruned: Number(r.pruned), createdAt: r.created_at as string } : null;
}

/**
 * Recompute the chain and check every anchor. After an anchored prune the
 * chain starts at the prune point: the first remaining entry must link to
 * the pruned entry's hash, which a signed anchor vouches for.
 */
export function verifyAudit(): AuditVerification {
  const rows = db().prepare('SELECT * FROM audit_entries WHERE sequence IS NOT NULL ORDER BY sequence ASC').all() as Row[];
  const prune = lastAuditPrune();
  let prev = prune?.throughHash ?? '';
  let expected = (prune?.throughSequence ?? 0) + 1;
  const hashes = new Map<number, string>();
  if (prune) hashes.set(prune.throughSequence, prune.throughHash);
  const fail = (seq: number, reason: string, n: number): AuditVerification => ({ ok: false, entriesChecked: n, firstBadSequence: seq, reason, anchorsChecked: 0, headSequence: seq, headHash: '' });
  for (let i = 0; i < rows.length; i++) {
    const r = fromRow(rows[i]);
    if (r.sequence !== expected) return fail(expected, `entry ${expected} is missing`, i);
    if (r.previousHash !== prev) return fail(r.sequence, 'previous-hash link broken', i);
    const { integrityHash, organizationId: _o, ...rest } = r;
    void _o;
    if (hashOf({ ...rest, userName: rest.userName }) !== integrityHash) return fail(r.sequence, 'entry content does not match its hash', i);
    hashes.set(r.sequence, integrityHash);
    prev = integrityHash;
    expected++;
  }
  const head = expected - 1;
  const anchors = db().prepare('SELECT * FROM audit_anchors ORDER BY last_sequence ASC').all() as Row[];
  if (prune) {
    const a = anchors.find((x) => x.id === prune.anchorId);
    if (!a || Number(a.last_sequence) !== prune.throughSequence || a.last_hash !== prune.throughHash) return fail(prune.throughSequence, 'the prune point is not a signed anchor', rows.length);
  }
  for (const a of anchors) {
    const seq = Number(a.last_sequence);
    const payload = `${seq}:${a.last_hash}:${a.entry_count}:${a.created_at}`;
    if (!timingSafeEqualHex(sign(payload), a.signature as string)) return { ...fail(seq, 'anchor signature is invalid', rows.length), anchorsChecked: 0 };
    // Anchors before the prune point vouch for entries that were removed on purpose.
    if (prune && seq < prune.throughSequence) continue;
    if (hashes.get(seq) !== a.last_hash) return { ...fail(seq, seq > head ? 'entries after an anchor were removed' : 'chain differs from a signed anchor', rows.length) };
  }
  return { ok: true, entriesChecked: rows.length, firstBadSequence: null, reason: null, anchorsChecked: anchors.length, headSequence: head, headHash: prev };
}

/**
 * Remove entries older than `before`, but only up to the newest signed
 * anchor at or before that point (and never past `limitSequence`, e.g. what
 * SIEM destinations have not forwarded yet). Refused when the chain does not
 * verify, so evidence of tampering is never pruned. Returns how many entries
 * were removed (0 when there is no suitable anchor).
 */
export function pruneAudit(before: Date, o: { by: string; limitSequence?: number } = { by: 'system' }): number {
  const v = verifyAudit();
  if (!v.ok) throw new Error(`The audit log does not verify (${v.reason}); nothing was pruned`);
  const old = db().prepare('SELECT MAX(sequence) AS s FROM audit_entries WHERE timestamp < ?').get(before.toISOString()) as Row;
  let limit = Number(old.s ?? 0);
  if (o.limitSequence !== undefined) limit = Math.min(limit, o.limitSequence);
  const from = lastAuditPrune()?.throughSequence ?? 0;
  const anchor = db().prepare('SELECT * FROM audit_anchors WHERE last_sequence <= ? AND last_sequence > ? ORDER BY last_sequence DESC LIMIT 1').get(limit, from) as Row | undefined;
  if (!anchor) return 0;
  const through = Number(anchor.last_sequence);
  return tx(() => {
    db().exec('DROP TRIGGER audit_no_delete');
    const pruned = db().prepare('DELETE FROM audit_entries WHERE sequence <= ?').run(through).changes;
    db().exec("CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_entries BEGIN SELECT RAISE(ABORT, 'audit entries are append-only'); END");
    db().prepare('INSERT INTO audit_prunes (id, through_sequence, through_hash, anchor_id, pruned, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)').run(newId('apr'), through, anchor.last_hash, anchor.id, pruned, o.by, now());
    return pruned;
  });
}

/** Sign the current chain head. Returns null when the log is empty. */
export function anchorAudit(): { id: string; lastSequence: number; lastHash: string; signature: string } | null {
  const head = db().prepare('SELECT sequence, integrity_hash FROM audit_entries WHERE sequence IS NOT NULL ORDER BY sequence DESC LIMIT 1').get() as Row | undefined;
  if (!head) return null;
  const last = db().prepare('SELECT last_sequence FROM audit_anchors ORDER BY last_sequence DESC LIMIT 1').get() as Row | undefined;
  if (last && Number(last.last_sequence) === Number(head.sequence)) return null;
  const createdAt = now();
  const count = Number(head.sequence);
  const signature = sign(`${head.sequence}:${head.integrity_hash}:${count}:${createdAt}`);
  const id = newId('anc');
  db().prepare('INSERT INTO audit_anchors (id, last_sequence, last_hash, entry_count, created_at, signature) VALUES (?, ?, ?, ?, ?, ?)').run(id, head.sequence, head.integrity_hash, count, createdAt, signature);
  return { id, lastSequence: Number(head.sequence), lastHash: head.integrity_hash as string, signature };
}

export function listAnchors(): { id: string; lastSequence: number; lastHash: string; createdAt: string }[] {
  return (db().prepare('SELECT * FROM audit_anchors ORDER BY last_sequence DESC').all() as Row[]).map((a) => ({
    id: a.id as string,
    lastSequence: Number(a.last_sequence),
    lastHash: a.last_hash as string,
    createdAt: a.created_at as string,
  }));
}
