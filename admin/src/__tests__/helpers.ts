/** Test helpers: users with roles, signed session cookies, and requests for route handlers. */
import { NextRequest } from 'next/server';
import { signSession, SESSION_COOKIE } from '@/lib/session';
import { createUser, type UserRecord } from '@/lib/db/repos/users';
import { setMembership } from '@/lib/db/repos/orgs';
import type { Role } from '@/lib/types/auth';

export function makeUser(email: string, role: Role, orgId = 'org_default', extra: Partial<Parameters<typeof createUser>[0]> = {}): UserRecord {
  const u = createUser({ email, name: email.split('@')[0], role, organizationId: orgId, ...extra });
  setMembership(u.id, orgId, role);
  return u;
}

export interface ReqOptions {
  body?: unknown;
  as?: UserRecord;
  org?: string;
  amr?: string[];
  headers?: Record<string, string>;
  /** Reuse this server-side session instead of creating one. */
  sid?: string;
}

export async function req(method: string, url: string, o: ReqOptions = {}): Promise<NextRequest> {
  const headers = new Headers({ 'content-type': 'application/json', origin: 'http://localhost:3001', host: 'localhost:3001', 'x-forwarded-for': '203.0.113.7', ...o.headers });
  if (o.as) headers.set('cookie', `${SESSION_COOKIE}=${await signSession(o.as, o.org ?? o.as.organizationId, o.amr ?? ['pwd'], o.sid)}`);
  return new NextRequest(new URL(url, 'http://localhost:3001'), { method, headers, body: o.body === undefined ? undefined : JSON.stringify(o.body) });
}

export async function json<T = Record<string, unknown>>(res: Response): Promise<{ status: number; body: T }> {
  return { status: res.status, body: (await res.json()) as T };
}

/** The session cookie a response set, for follow-up requests. */
export function cookieFrom(res: Response): string | null {
  const c = res.headers.get('set-cookie');
  const m = c && new RegExp(`${SESSION_COOKIE}=([^;]+)`).exec(c);
  return m ? m[1] : null;
}
