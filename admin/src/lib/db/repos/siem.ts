/**
 * SIEM forwarding destinations. Each destination keeps a cursor (the last
 * audit sequence it accepted), so nothing is lost or sent twice across
 * restarts and outages: on failure the cursor stays, the error is recorded
 * and the destination retries with the shared backoff, the backlog growing
 * until it recovers. Sending lives in `lib/siem.ts`.
 */
import { db, newId, now, parseJson, type Row } from '../index';
import { openOpt, sealOpt } from '../../crypto';
import { managedSubtree, OrgError } from './orgs';
import { auditHead, countAuditAfter } from './audit';
import { backoffSeconds } from '../../retry';

export type SiemKind = 'syslog' | 'hec' | 'https' | 'jsonl';
export const SIEM_KINDS: SiemKind[] = ['syslog', 'hec', 'https', 'jsonl'];

export interface SyslogConfig {
  host: string;
  port: number;
  /** udp (RFC 5426), tcp (RFC 6587 octet counting) or tls (RFC 5425). */
  transport: 'udp' | 'tcp' | 'tls';
  /** Trusted CA for tls (PEM; default: system roots). */
  caPem?: string | null;
  /** Syslog facility (default 16 = local0). */
  facility?: number;
}
/** Splunk HTTP Event Collector: the full collector URL; the token is the secret. */
export interface HecConfig {
  url: string;
  index?: string;
  sourcetype?: string;
}
/** Any HTTPS endpoint taking a JSON batch; the secret (if any) is sent as a bearer token and signs the body. */
export interface HttpsConfig {
  url: string;
}
/** Append JSON lines to a file in SIEM_JSONL_DIR (for log shippers). */
export interface JsonlConfig {
  file: string;
}

export interface SiemDestination {
  id: string;
  orgId: string;
  name: string;
  kind: SiemKind;
  config: SyslogConfig | HecConfig | HttpsConfig | JsonlConfig;
  hasSecret: boolean;
  /** Also forward the audit entries of child organizations (not across detached ones). */
  includeChildren: boolean;
  enabled: boolean;
  cursor: number;
  forwardedTotal: number;
  failures: number;
  nextAttemptAt: string | null;
  lastError: string | null;
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
}

function fromRow(r: Row): SiemDestination {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    kind: r.kind as SiemKind,
    config: parseJson(r.config, {}) as SiemDestination['config'],
    hasSecret: !!r.secret_ciphertext,
    includeChildren: !!r.include_children,
    enabled: !!r.enabled,
    cursor: Number(r.cursor),
    forwardedTotal: Number(r.forwarded_total),
    failures: Number(r.failures),
    nextAttemptAt: (r.next_attempt_at as string) ?? null,
    lastError: (r.last_error as string) ?? null,
    lastAttemptAt: (r.last_attempt_at as string) ?? null,
    lastSuccessAt: (r.last_success_at as string) ?? null,
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    updatedAt: r.updated_at as string,
  };
}

const httpUrl = (v: unknown) => {
  try {
    const u = new URL(String(v));
    return u.protocol === 'https:' || u.protocol === 'http:';
  } catch {
    return false;
  }
};

/** Validate and normalize a destination's configuration. */
export function validateSiem(kind: unknown, c: Record<string, unknown>): SiemDestination['config'] {
  switch (kind) {
    case 'syslog': {
      const transport = c.transport ?? 'tcp';
      if (typeof c.host !== 'string' || !c.host.trim()) throw new OrgError('Enter the syslog server');
      const port = Number(c.port ?? (transport === 'tls' ? 6514 : 514));
      if (!Number.isInteger(port) || port < 1 || port > 65535) throw new OrgError('The port is 1 to 65535');
      if (!['udp', 'tcp', 'tls'].includes(String(transport))) throw new OrgError('The transport is udp, tcp or tls');
      const facility = Number(c.facility ?? 16);
      if (!Number.isInteger(facility) || facility < 0 || facility > 23) throw new OrgError('The facility is 0 to 23');
      return { host: c.host.trim(), port, transport: transport as SyslogConfig['transport'], caPem: typeof c.caPem === 'string' && c.caPem.trim() ? c.caPem : null, facility };
    }
    case 'hec':
      if (!httpUrl(c.url)) throw new OrgError('Enter the HTTP Event Collector URL, e.g. https://splunk.example.com:8088/services/collector/event');
      return { url: String(c.url), index: typeof c.index === 'string' && c.index ? c.index : undefined, sourcetype: typeof c.sourcetype === 'string' && c.sourcetype ? c.sourcetype : 'ufop:audit' };
    case 'https':
      if (!httpUrl(c.url)) throw new OrgError('Enter an http(s) URL');
      return { url: String(c.url) };
    case 'jsonl':
      if (typeof c.file !== 'string' || !/^[\w.-]{1,100}\.jsonl$/.test(c.file)) throw new OrgError('Enter a file name ending in .jsonl (no folders); it is written in SIEM_JSONL_DIR');
      if (!process.env.SIEM_JSONL_DIR) throw new OrgError('Set SIEM_JSONL_DIR on the console server to write JSON-lines files');
      return { file: c.file };
    default:
      throw new OrgError('The kind is syslog, hec, https or jsonl');
  }
}

export interface SiemInput {
  name: string;
  kind: SiemKind;
  config: Record<string, unknown>;
  /** HEC token or HTTPS bearer/signing secret; null clears, undefined keeps. */
  secret?: string | null;
  includeChildren?: boolean;
  enabled?: boolean;
  /** On create: also send the existing history (default: start from now). */
  backfill?: boolean;
}

export function createSiemDestination(orgId: string, i: SiemInput, by: string): SiemDestination {
  if (!i.name?.trim()) throw new OrgError('Give the destination a name');
  const config = validateSiem(i.kind, i.config ?? {});
  if (i.kind === 'hec' && !i.secret) throw new OrgError('Enter the HEC token');
  const id = newId('siem');
  const ts = now();
  db()
    .prepare('INSERT INTO siem_destinations (id, org_id, name, kind, config, secret_ciphertext, include_children, enabled, cursor, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, i.name.trim().slice(0, 100), i.kind, JSON.stringify(config), sealOpt(i.secret), i.includeChildren ? 1 : 0, i.enabled === false ? 0 : 1, i.backfill ? 0 : auditHead(), by, ts, ts);
  return getSiemDestination(orgId, id)!;
}

export function updateSiemDestination(orgId: string, id: string, i: Partial<SiemInput>): SiemDestination | null {
  const cur = getSiemDestination(orgId, id);
  if (!cur) return null;
  const config = i.config ? validateSiem(cur.kind, i.config) : cur.config;
  db()
    .prepare('UPDATE siem_destinations SET name = ?, config = ?, include_children = ?, enabled = ?, updated_at = ? WHERE id = ?')
    .run(i.name?.trim() || cur.name, JSON.stringify(config), (i.includeChildren ?? cur.includeChildren) ? 1 : 0, (i.enabled ?? cur.enabled) ? 1 : 0, now(), id);
  if (i.secret !== undefined) db().prepare('UPDATE siem_destinations SET secret_ciphertext = ? WHERE id = ?').run(sealOpt(i.secret), id);
  // Re-enabling or fixing the configuration retries right away.
  db().prepare('UPDATE siem_destinations SET next_attempt_at = NULL WHERE id = ?').run(id);
  return getSiemDestination(orgId, id);
}

export function getSiemDestination(orgId: string, id: string): SiemDestination | null {
  const r = db().prepare('SELECT * FROM siem_destinations WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function listSiemDestinations(orgId: string): SiemDestination[] {
  return (db().prepare('SELECT * FROM siem_destinations WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

export function deleteSiemDestination(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM siem_destinations WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

export function siemSecret(id: string): string | null {
  return openOpt((db().prepare('SELECT secret_ciphertext FROM siem_destinations WHERE id = ?').get(id) as Row | undefined)?.secret_ciphertext);
}

/** Organizations whose audit entries a destination forwards. */
export function siemOrgIds(d: SiemDestination): string[] {
  return d.includeChildren ? managedSubtree(d.orgId) : [d.orgId];
}

/** Entries waiting to be forwarded. */
export function siemBacklog(d: SiemDestination): number {
  return countAuditAfter(siemOrgIds(d), d.cursor);
}

/** Enabled destinations whose retry time has come. */
export function dueSiemDestinations(at = new Date()): SiemDestination[] {
  return (db().prepare('SELECT * FROM siem_destinations WHERE enabled = 1 AND (next_attempt_at IS NULL OR next_attempt_at <= ?)').all(at.toISOString()) as Row[]).map(fromRow);
}

/** A batch was accepted: advance the cursor. */
export function recordSiemSuccess(id: string, cursor: number, count: number, at = new Date()): void {
  db()
    .prepare('UPDATE siem_destinations SET cursor = MAX(cursor, ?), forwarded_total = forwarded_total + ?, failures = 0, next_attempt_at = NULL, last_error = NULL, last_attempt_at = ?, last_success_at = ? WHERE id = ?')
    .run(cursor, count, at.toISOString(), at.toISOString(), id);
}

/** A send failed: keep the cursor, back off. */
export function recordSiemFailure(id: string, error: string, at = new Date()): void {
  const r = db().prepare('SELECT failures FROM siem_destinations WHERE id = ?').get(id) as Row | undefined;
  const failures = Number(r?.failures ?? 0) + 1;
  db()
    .prepare('UPDATE siem_destinations SET failures = ?, next_attempt_at = ?, last_error = ?, last_attempt_at = ? WHERE id = ?')
    .run(failures, new Date(at.getTime() + backoffSeconds(failures) * 1000).toISOString(), error.slice(0, 500), at.toISOString(), id);
}
