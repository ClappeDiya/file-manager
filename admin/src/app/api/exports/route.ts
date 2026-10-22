import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, isContext, readJson, requirePermission } from '@/lib/session';
import { listExports, queueExport, SOURCES, type ExportKind } from '@/lib/exports';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/exports — your exports in this organization and what you can export. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const kinds = (Object.keys(SOURCES) as ExportKind[]).filter((k) => can(ctx, SOURCES[k].permission)).map((k) => ({ kind: k, label: SOURCES[k].label, formats: SOURCES[k].formats }));
  return NextResponse.json({ exports: listExports(ctx.orgId, ctx.user.email), kinds });
}

/** POST /api/exports {kind, format, params?} — queue a background export. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ kind?: ExportKind; format?: string; params?: Record<string, string> }>(request)) ?? {};
  const src = b.kind ? SOURCES[b.kind] : undefined;
  if (src && !can(ctx, src.permission)) return badRequest(`Forbidden: requires ${src.permission}`, 403);
  try {
    const job = queueExport(ctx.orgId, b.kind, b.format, b.params, ctx.user.email);
    audit(ctx, { eventType: 'export.queue', description: `Started an export of the ${src!.label.toLowerCase()} (${job.format})`, resourceType: 'export', resourceId: job.id, metadata: { params: job.params } });
    return NextResponse.json({ export: job }, { status: 202 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
