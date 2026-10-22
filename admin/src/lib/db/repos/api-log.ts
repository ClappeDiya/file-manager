/**
 * API request log: every request made with an API token (accepted or not)
 * and every request refused for its network, so administrators can see
 * which scripts call the console and why calls fail. Kept KEEP_DAYS days.
 */
import { db, newId, now, type Row } from '../index';

export const KEEP_DAYS = 30;

export interface ApiRequest {
  id: string;
  orgId: string;
  at: string;
  method: string;
  path: string;
  status: number;
  /** Why it was refused, or `authorized`. */
  outcome: string;
  auth: 'token' | 'session';
  tokenId: string | null;
  userId: string | null;
  userEmail: string;
  ip: string;
  userAgent: string;
}

function fromRow(r: Row): ApiRequest {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    at: r.at as string,
    method: r.method as string,
    path: r.path as string,
    status: Number(r.status),
    outcome: r.outcome as string,
    auth: r.auth as ApiRequest['auth'],
    tokenId: (r.token_id as string) ?? null,
    userId: (r.user_id as string) ?? null,
    userEmail: r.user_email as string,
    ip: r.ip as string,
    userAgent: r.user_agent as string,
  };
}

export function logApiRequest(e: Omit<ApiRequest, 'id' | 'at'>): void {
  db()
    .prepare('INSERT INTO api_requests (id, org_id, at, method, path, status, outcome, auth, token_id, user_id, user_email, ip, user_agent) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(newId('req'), e.orgId, now(), e.method, e.path.slice(0, 300), e.status, e.outcome.slice(0, 200), e.auth, e.tokenId, e.userId, e.userEmail, e.ip.slice(0, 100), e.userAgent.slice(0, 300));
}

export interface ApiRequestQuery {
  tokenId?: string;
  /** `ok` (2xx), `refused` (4xx/5xx). */
  result?: 'ok' | 'refused';
  limit?: number;
  offset?: number;
}

export function listApiRequests(orgId: string, q: ApiRequestQuery = {}): { requests: ApiRequest[]; total: number } {
  const where = ['org_id = ?'];
  const args: unknown[] = [orgId];
  if (q.tokenId) {
    where.push('token_id = ?');
    args.push(q.tokenId);
  }
  if (q.result === 'ok') where.push('status < 400');
  if (q.result === 'refused') where.push('status >= 400');
  const w = where.join(' AND ');
  const total = (
    db()
      .prepare(`SELECT COUNT(*) AS n FROM api_requests WHERE ${w}`)
      .get(...args) as { n: number }
  ).n;
  const rows = db()
    .prepare(`SELECT * FROM api_requests WHERE ${w} ORDER BY at DESC, rowid DESC LIMIT ? OFFSET ?`)
    .all(...args, Math.min(q.limit ?? 50, 500), q.offset ?? 0) as Row[];
  return { requests: rows.map(fromRow), total };
}

export function pruneApiRequests(at = new Date()): number {
  return db()
    .prepare('DELETE FROM api_requests WHERE at < ?')
    .run(new Date(at.getTime() - KEEP_DAYS * 86_400_000).toISOString()).changes;
}
