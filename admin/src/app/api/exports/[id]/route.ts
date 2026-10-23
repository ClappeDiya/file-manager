import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, requirePermission, type AuthContext } from '@/lib/session';
import { deleteExport, getExport, type ExportJob } from '@/lib/exports';

type P = { params: Promise<{ id: string }> };

/** Exports are visible to the person who requested them. */
async function own(request: NextRequest, id: string): Promise<{ res: NextResponse } | { ctx: AuthContext; job: ExportJob }> {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return { res: ctx };
  const job = getExport(ctx.orgId, id);
  if (!job || job.requestedBy !== ctx.user.email) return { res: badRequest('Export not found', 404) };
  return { ctx, job };
}

/** GET — status and progress. */
export async function GET(request: NextRequest, { params }: P) {
  const r = await own(request, (await params).id);
  if ('res' in r) return r.res;
  return NextResponse.json({ export: r.job });
}

/** DELETE — remove a finished export (or cancel a queued one). */
export async function DELETE(request: NextRequest, { params }: P) {
  const r = await own(request, (await params).id);
  if ('res' in r) return r.res;
  if (!deleteExport(r.ctx.orgId, r.job.id)) return badRequest('A running export cannot be removed', 409);
  return NextResponse.json({ success: true });
}
