import { NextRequest, NextResponse } from 'next/server';
import { badRequest, can, isContext, requirePermission } from '@/lib/session';
import { renderReport, reportDefinition, type ReportFormat } from '@/lib/report-catalog';
import { getRun, runResult } from '@/lib/report-runner';

const TYPES: Record<ReportFormat, string> = { csv: 'text/csv; charset=utf-8', json: 'application/json', text: 'text/plain; charset=utf-8' };

/** GET /api/reports/runs/:id?format=csv|json|text — download a run's result (or its status without format). */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const run = getRun(ctx.orgId, (await params).id);
  const def = run ? reportDefinition(run.reportId) : null;
  if (!run || !def || !can(ctx, def.permission)) return badRequest('Run not found', 404);
  const format = new URL(request.url).searchParams.get('format') as ReportFormat | null;
  if (!format) return NextResponse.json({ run });
  if (!(format in TYPES)) return badRequest('The format is csv, json or text');
  const result = runResult(ctx.orgId, run.id);
  if (!result) return badRequest('This run has no result', 409);
  return new NextResponse(renderReport(result, format), {
    headers: { 'content-type': TYPES[format], 'content-disposition': `attachment; filename="${def.id}-${run.startedAt.slice(0, 10)}.${format === 'text' ? 'txt' : format}"`, 'cache-control': 'no-store' },
  });
}
