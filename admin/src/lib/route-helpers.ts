import { NextResponse } from 'next/server';
import { OrgError } from './db/repos/orgs';

/** Map domain validation errors to 4xx responses; rethrow anything else. */
export function orgErrorResponse(e: unknown): NextResponse {
  if (e instanceof OrgError) return NextResponse.json({ error: e.message }, { status: e.status });
  throw e;
}

/** The console's externally visible origin (ADMIN_PUBLIC_URL behind a proxy). */
export function publicOrigin(req: Request): string {
  const env = process.env.ADMIN_PUBLIC_URL?.replace(/\/$/, '');
  return env || new URL(req.url).origin;
}

/** Only same-site relative paths are accepted as post-login destinations. */
export function safeRedirect(v: unknown): string {
  return typeof v === 'string' && v.startsWith('/') && !v.startsWith('//') && !v.startsWith('/\\') ? v : '/';
}
