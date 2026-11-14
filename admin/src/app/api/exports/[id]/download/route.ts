import { NextRequest, NextResponse } from 'next/server';
import zlib from 'zlib';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { exportFile, getExport } from '@/lib/exports';

const TYPES = { csv: 'text/csv; charset=utf-8', json: 'application/json', syslog: 'text/plain; charset=utf-8' } as const;

/** GET /api/exports/:id/download[?gz=1] — the finished file (gzip-compressed with gz=1). */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const job = getExport(ctx.orgId, (await params).id);
  if (!job || job.requestedBy !== ctx.user.email) return badRequest('Export not found', 404);
  const file = exportFile(ctx.orgId, job.id);
  if (!file) return badRequest(job.status === 'expired' ? 'This export has expired; start a new one' : 'This export is not ready', 409);
  const gz = new URL(request.url).searchParams.get('gz') === '1';
  const name = `${job.kind}-${job.createdAt.slice(0, 10)}.${job.format === 'syslog' ? 'log' : job.format}${gz ? '.gz' : ''}`;
  audit(ctx, { eventType: 'export.download', description: `Downloaded export ${job.id} (${job.kind}, ${job.total} rows)`, resourceType: 'export', resourceId: job.id });
  return new NextResponse(new Uint8Array(gz ? file : zlib.gunzipSync(file)), {
    headers: { 'content-type': gz ? 'application/gzip' : TYPES[job.format], 'content-disposition': `attachment; filename="${name}"`, 'cache-control': 'no-store', 'x-content-sha256': job.sha256 ?? '' },
  });
}
