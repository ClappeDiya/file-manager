import { NextRequest, NextResponse } from 'next/server';
import { pageInfo } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
import { badRequest, isContext, requirePermission } from '@/lib/session';
import { getProvider, listSsoAttempts } from '@/lib/db/repos/sso';

/** GET /api/sso/:id/attempts — recent sign-ins through this provider, step by step (diagnostics). */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const p = getProvider(id);
  if (!p || p.orgId !== ctx.orgId) return badRequest('Provider not found', 404);
  const pr = readPage(request, { maxLimit: 200 });
  if (pr instanceof NextResponse) return pr;
  const attempts = listSsoAttempts(ctx.orgId, id, pr.limit, pr.offset);
  return pagedJson(request, { attempts }, pageInfo(pr, attempts.length, null));
}
