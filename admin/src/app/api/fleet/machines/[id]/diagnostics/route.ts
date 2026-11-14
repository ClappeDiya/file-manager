import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { getMachine } from '@/lib/db/repos/machines';
import { deleteDiagnostics, diagnosticsData, getDiagnostics, listDiagnostics } from '@/lib/db/repos/diagnostics';
import { canSeeMachine } from '@/lib/scope';

type P = { params: Promise<{ id: string }> };

/** GET /api/fleet/machines/:id/diagnostics — list; `?download=<id>` returns the zip. */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const id = new URL(request.url).searchParams.get('download');
  if (!id) return NextResponse.json({ diagnostics: listDiagnostics(m.id) });
  const meta = getDiagnostics(m.id, id);
  const data = meta && diagnosticsData(m.id, id);
  if (!meta || !data) return badRequest('Diagnostics not found', 404);
  audit(ctx, { eventType: 'fleet.diagnostics_download', description: `Downloaded diagnostics ${meta.fileName} of "${m.name}"`, resourceType: 'machine', resourceId: m.id, metadata: { diagnosticsId: id } });
  return new NextResponse(new Uint8Array(data), {
    headers: { 'content-type': 'application/zip', 'content-disposition': `attachment; filename="${meta.fileName}"`, 'content-length': String(data.length), 'x-content-sha256': meta.sha256, 'cache-control': 'no-store' },
  });
}

/** DELETE /api/fleet/machines/:id/diagnostics?id= */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const id = new URL(request.url).searchParams.get('id') ?? '';
  if (!deleteDiagnostics(m.id, id)) return badRequest('Diagnostics not found', 404);
  audit(ctx, { eventType: 'fleet.diagnostics_delete', description: `Deleted diagnostics ${id} of "${m.name}"`, resourceType: 'machine', resourceId: m.id, metadata: { diagnosticsId: id } });
  return NextResponse.json({ success: true });
}
