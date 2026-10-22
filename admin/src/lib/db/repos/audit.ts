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

/** Recompute the whole chain and check every anchor. */
export function verifyAudit(): AuditVerification {
  const rows = db().prepare('SELECT * FROM audit_entries WHERE sequence IS NOT NULL ORDER BY sequence ASC').all() as Row[];
  let prev = '';
  let expected = 1;
  const hashes = new Map<number, string>();
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
  const anchors = db().prepare('SELECT * FROM audit_anchors ORDER BY last_sequence ASC').all() as Row[];
  for (const a of anchors) {
    const seq = Number(a.last_sequence);
    const payload = `${seq}:${a.last_hash}:${a.entry_count}:${a.created_at}`;
    if (!timingSafeEqualHex(sign(payload), a.signature as string)) return { ...fail(seq, 'anchor signature is invalid', rows.length), anchorsChecked: 0 };
    if (hashes.get(seq) !== a.last_hash) return { ...fail(seq, seq > rows.length ? 'entries after an anchor were removed' : 'chain differs from a signed anchor', rows.length) };
  }
  return { ok: true, entriesChecked: rows.length, firstBadSequence: null, reason: null, anchorsChecked: anchors.length, headSequence: rows.length, headHash: prev };
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
