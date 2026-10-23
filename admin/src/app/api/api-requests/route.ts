import { NextRequest, NextResponse } from 'next/server';
import { isContext, requirePermission } from '@/lib/session';
import { listApiRequests } from '@/lib/db/repos/api-log';
import { getRetention } from '@/lib/retention';
import { pageInfo } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';

/** GET /api/api-requests?tokenId=&result=ok|refused&limit=&cursor= — API-token requests and refused requests of this organization. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.read');
  if (!isContext(ctx)) return ctx;
  const pr = readPage(request);
  if (pr instanceof NextResponse) return pr;
  const q = new URL(request.url).searchParams;
  const result = q.get('result');
  const { requests, total } = listApiRequests(ctx.orgId, { tokenId: q.get('tokenId') ?? undefined, result: result === 'ok' || result === 'refused' ? result : undefined, limit: pr.limit, offset: pr.offset });
  return pagedJson(request, { requests, total, keepDays: getRetention().api_requests }, pageInfo(pr, requests.length, total));
}
