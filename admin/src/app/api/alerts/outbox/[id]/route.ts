import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { discardOutbox, getOutboxEntry, retryOutbox } from '@/lib/db/repos/outbox';
import { sendOutboxNow } from '@/lib/mailer';

type P = { params: Promise<{ id: string }> };

/** POST /api/alerts/outbox/:id {action: retry | discard} — retry sends now. */
export async function POST(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<{ action?: string }>(request);
  if (!getOutboxEntry(ctx.orgId, id)) return badRequest('Message not found', 404);
  if (b?.action === 'retry') {
    if (!retryOutbox(ctx.orgId, id)) return badRequest('This message was already sent', 409);
    const d = await sendOutboxNow(ctx.orgId, id);
    audit(ctx, { eventType: 'outbox.retry', description: `Retried notification ${id}`, resourceType: 'outbox', resourceId: id, metadata: { ok: d?.ok ?? false } });
    return NextResponse.json({ ok: d?.ok ?? false, error: d?.error ?? (d ? null : 'Channel removed'), message: getOutboxEntry(ctx.orgId, id) });
  }
  if (b?.action === 'discard') {
    if (!discardOutbox(ctx.orgId, id)) return badRequest('Only pending or failed messages can be discarded', 409);
    audit(ctx, { eventType: 'outbox.discard', description: `Discarded notification ${id}`, resourceType: 'outbox', resourceId: id });
    return NextResponse.json({ ok: true });
  }
  return badRequest('Unknown action');
}
