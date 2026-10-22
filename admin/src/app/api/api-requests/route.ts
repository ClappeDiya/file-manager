import { NextRequest, NextResponse } from 'next/server';
import { isContext, requirePermission } from '@/lib/session';
import { listApiRequests, KEEP_DAYS } from '@/lib/db/repos/api-log';

/** GET /api/api-requests?tokenId=&result=ok|refused&limit=&offset= — API-token requests and refused requests of this organization. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.read');
  if (!isContext(ctx)) return ctx;
  const q = new URL(request.url).searchParams;
  const result = q.get('result');
  return NextResponse.json({
    ...listApiRequests(ctx.orgId, {
      tokenId: q.get('tokenId') ?? undefined,
      result: result === 'ok' || result === 'refused' ? result : undefined,
      limit: Number(q.get('limit') ?? 50),
      offset: Number(q.get('offset') ?? 0),
    }),
    keepDays: KEEP_DAYS,
  });
}
