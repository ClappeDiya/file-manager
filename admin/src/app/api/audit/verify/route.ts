import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, requirePermission } from '@/lib/session';
import { anchorAudit, listAnchors, verifyAudit } from '@/lib/db/repos/audit';

/** GET /api/audit/verify — recompute the hash chain and check signed anchors. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ verification: verifyAudit(), anchors: listAnchors().slice(0, 50) });
}

/** POST /api/audit/verify — sign the current chain head (checkpoint). */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  const anchor = anchorAudit();
  if (anchor) audit(ctx, { eventType: 'audit.anchor', description: `Signed audit checkpoint at entry ${anchor.lastSequence}`, resourceType: 'audit_anchor', resourceId: anchor.id });
  return NextResponse.json({ anchor, verification: verifyAudit() });
}
