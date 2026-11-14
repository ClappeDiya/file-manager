/**
 * One paging convention for every console list route:
 *
 *   request   ?limit=N (1–500) and ?cursor=<opaque, from the previous page>
 *             (`offset`, and `page` + `pageSize`, are still accepted)
 *   response  the list under its usual key, plus
 *             `pagination: { limit, offset, total, nextCursor, hasMore }`,
 *             an `X-Total-Count` header and a `Link: <…>; rel="next"` header
 *
 * Routes whose screens show everything (members, machines) return the whole
 * list unless a limit or cursor is given; the others default to 50.
 */
import type { NextResponse } from 'next/server';

export const DEFAULT_LIMIT = 50;
export const MAX_LIMIT = 500;

export interface PageRequest {
  /** Infinity = no limit (only where the route allows it). */
  limit: number;
  offset: number;
}

export interface PageInfo {
  limit: number | null;
  offset: number;
  total: number | null;
  nextCursor: string | null;
  hasMore: boolean;
}

export const encodeCursor = (offset: number) => Buffer.from(JSON.stringify({ o: offset })).toString('base64url');

export function decodeCursor(cursor: string): number | null {
  try {
    const v = JSON.parse(Buffer.from(cursor, 'base64url').toString('utf8')) as { o?: unknown };
    return Number.isInteger(v.o) && (v.o as number) >= 0 ? (v.o as number) : null;
  } catch {
    return null;
  }
}

/** Thrown for a cursor that was not issued by this API. */
export class BadCursor extends Error {}

/**
 * Read paging parameters. `defaultLimit: Infinity` keeps a route's
 * "everything unless asked" behaviour.
 */
export function pageRequest(sp: URLSearchParams, o: { defaultLimit?: number; maxLimit?: number } = {}): PageRequest {
  const max = o.maxLimit ?? MAX_LIMIT;
  const def = o.defaultLimit ?? DEFAULT_LIMIT;
  const pageSize = sp.get('pageSize');
  const rawLimit = sp.get('limit') ?? pageSize;
  const n = rawLimit === null ? def : Number(rawLimit);
  const limit = Number.isFinite(n) && n >= 1 ? Math.min(Math.floor(n), max) : Number.isFinite(def) ? Math.min(def, max) : def;
  let offset = 0;
  const cursor = sp.get('cursor');
  if (cursor) {
    const c = decodeCursor(cursor);
    if (c === null) throw new BadCursor('The cursor is not valid; start again without it');
    offset = c;
  } else if (sp.get('offset')) {
    offset = Math.max(0, Math.floor(Number(sp.get('offset')) || 0));
  } else if (sp.get('page') && Number.isFinite(limit)) {
    offset = (Math.max(1, Math.floor(Number(sp.get('page')) || 1)) - 1) * limit;
  }
  return { limit, offset };
}

/** Page metadata for `returned` items at `req`, out of `total` (null when unknown: then a full page means "maybe more"). */
export function pageInfo(req: PageRequest, returned: number, total: number | null): PageInfo {
  const hasMore = total !== null ? req.offset + returned < total : Number.isFinite(req.limit) && returned === req.limit;
  return { limit: Number.isFinite(req.limit) ? req.limit : null, offset: req.offset, total, nextCursor: hasMore ? encodeCursor(req.offset + returned) : null, hasMore };
}

/** Page an in-memory list. */
export function paged<T>(all: T[], req: PageRequest): { items: T[]; page: PageInfo } {
  const items = Number.isFinite(req.limit) ? all.slice(req.offset, req.offset + req.limit) : all.slice(req.offset);
  return { items, page: pageInfo(req, items.length, all.length) };
}

/** `X-Total-Count` and an RFC 8288 `Link` to the next page. */
export function withPageHeaders<T extends NextResponse>(res: T, page: PageInfo, url: string): T {
  if (page.total !== null) res.headers.set('x-total-count', String(page.total));
  if (page.nextCursor) {
    const next = new URL(url);
    next.searchParams.delete('offset');
    next.searchParams.delete('page');
    next.searchParams.set('cursor', page.nextCursor);
    if (page.limit !== null) next.searchParams.set('limit', String(page.limit));
    res.headers.set('link', `<${next.pathname}${next.search}>; rel="next"`);
  }
  return res;
}
