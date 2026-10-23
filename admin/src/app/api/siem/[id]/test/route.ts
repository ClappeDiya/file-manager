import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, requirePermission } from '@/lib/session';
import { getSiemDestination } from '@/lib/db/repos/siem';
import { sendToSiem, testEvent } from '@/lib/siem';

/** POST /api/siem/:id/test — send one synthetic event (the cursor does not move). */
export async function POST(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  const d = getSiemDestination(ctx.orgId, (await params).id);
  if (!d) return badRequest('Destination not found', 404);
  const r = await sendToSiem(d, [testEvent(d)]);
  return NextResponse.json(r, { status: r.ok ? 200 : 502 });
}
