import { NextResponse } from 'next/server';
import { BadCursor, pageRequest, withPageHeaders, type PageInfo, type PageRequest } from './pagination';
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

/** Paging parameters for a list route (see `lib/pagination`), or a 400 for a bad cursor. */
export function readPage(request: Request, o?: { defaultLimit?: number; maxLimit?: number }): PageRequest | NextResponse {
  try {
    return pageRequest(new URL(request.url).searchParams, o);
  } catch (e) {
    if (e instanceof BadCursor) return NextResponse.json({ error: e.message }, { status: 400 });
    throw e;
  }
}

/** A list response with `pagination` and the paging headers. */
export function pagedJson(request: Request, body: Record<string, unknown>, pagination: PageInfo): NextResponse {
  return withPageHeaders(NextResponse.json({ ...body, pagination }), pagination, request.url);
}
