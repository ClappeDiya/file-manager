import { NextRequest, NextResponse } from 'next/server';
import { APPROVAL_EXPIRY_HOURS, type ApprovalRequest, type ApprovalState, type ApprovalTrigger } from '@/lib/types/approvals';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createApproval, getApproval, listApprovals, reviewApproval } from '@/lib/db/repos/governance';

/** GET /api/approvals - List approval requests of the current organization */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'approvals.read');
  if (!isContext(ctx)) return ctx;
  const { searchParams } = new URL(request.url);
  const state = searchParams.get('state') as ApprovalState | null;
  const trigger = searchParams.get('trigger') as ApprovalTrigger | null;
  let result = listApprovals(ctx.orgId);
  if (state) result = result.filter((a) => a.state === state);
  if (trigger) result = result.filter((a) => a.trigger === trigger);
  const count = (s: ApprovalState) => result.filter((a) => a.state === s).length;
  return NextResponse.json({ approvals: result, stats: { pending: count('pending'), approved: count('approved'), denied: count('denied'), expired: count('expired') }, total: result.length });
}

/** POST /api/approvals - Request an approval (the requester is the signed-in user) */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'admin.access');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<Partial<ApprovalRequest>>(request);
  if (!b?.operation || !b.trigger || !b.source || !b.destination) return badRequest('Missing required fields');
  const approval = createApproval(
    ctx.orgId,
    { operation: b.operation, trigger: b.trigger, source: b.source, destination: b.destination, fileCount: b.fileCount ?? 0, totalSize: b.totalSize ?? 0, reason: b.reason ?? '', triggeringPolicy: b.triggeringPolicy ?? '', requestedBy: ctx.user.email, requestedByName: ctx.user.name },
    APPROVAL_EXPIRY_HOURS,
  );
  audit(ctx, { eventType: 'approval.request', description: `Requested approval for ${approval.operation}`, resourceType: 'approval', resourceId: approval.id });
  return NextResponse.json({ approval }, { status: 201 });
}

/** PATCH /api/approvals - Approve or deny a pending request */
export async function PATCH(request: NextRequest) {
  const b = await readJson<{ id?: string; action?: string; reviewComment?: string }>(request);
  const ctx = await requirePermission(request, b?.action === 'deny' ? 'approvals.deny' : 'approvals.approve');
  if (!isContext(ctx)) return ctx;
  if (!b?.id || !b.action || !['approve', 'deny'].includes(b.action)) return badRequest('ID and action (approve/deny) required');
  const cur = getApproval(ctx.orgId, b.id);
  if (!cur) return badRequest('Approval not found', 404);
  if (cur.state === 'expired') return badRequest('Approval has expired');
  if (cur.state !== 'pending') return badRequest('Can only review pending approvals');
  if (cur.requestedBy === ctx.user.email) return badRequest('Cannot review your own approval request', 403);
  const approval = reviewApproval(ctx.orgId, b.id, b.action === 'approve', { email: ctx.user.email, name: ctx.user.name }, b.reviewComment ?? '');
  if (!approval) return badRequest('Approval is no longer pending', 409);
  audit(ctx, { eventType: b.action === 'approve' ? 'approval.approve' : 'approval.deny', description: `${b.action === 'approve' ? 'Approved' : 'Denied'} ${approval.operation}`, resourceType: 'approval', resourceId: approval.id });
  return NextResponse.json({ approval });
}
