import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import {
  deleteWebhook,
  deliveryStats,
  enqueueTest,
  getWebhook,
  listDeliveries,
  pauseWebhook,
  redeliver,
  resumeWebhook,
  rotateWebhookSecret,
  updateWebhook,
  type DeliveryStatus,
  type WebhookInput,
} from '@/lib/db/repos/webhooks';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };
const STATUSES: DeliveryStatus[] = ['pending', 'succeeded', 'dead'];

/** GET /api/webhooks/:id[?status=&limit=] — the webhook, counts and its delivery log. */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const w = getWebhook(ctx.orgId, id);
  if (!w) return badRequest('Webhook not found', 404);
  const q = new URL(request.url).searchParams;
  const status = q.get('status') as DeliveryStatus | null;
  if (status && !STATUSES.includes(status)) return badRequest('Unknown status');
  return NextResponse.json({ webhook: w, stats: deliveryStats(ctx.orgId, id), deliveries: listDeliveries(ctx.orgId, id, { status: status ?? undefined, limit: Number(q.get('limit')) || 100 }) });
}

/** PATCH /api/webhooks/:id {name?, url?, events?, headers?, enabled?} */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<WebhookInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const w = updateWebhook(ctx.orgId, id, b);
    if (!w) return badRequest('Webhook not found', 404);
    audit(ctx, { eventType: 'webhook.update', description: `Updated webhook "${w.name}"`, resourceType: 'webhook', resourceId: id, metadata: { events: w.events, enabled: w.enabled } });
    return NextResponse.json({ webhook: w });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

/**
 * POST /api/webhooks/:id {action}
 * - `test` — queue a webhook.test delivery
 * - `pause` {reason?} / `resume`
 * - `rotate_secret` — returns the new secret once
 * - `redeliver` {deliveryId}
 */
export async function POST(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const w = getWebhook(ctx.orgId, id);
  if (!w) return badRequest('Webhook not found', 404);
  const b = (await readJson<{ action?: string; reason?: string; deliveryId?: string }>(request)) ?? {};
  const log = (eventType: string, description: string) => audit(ctx, { eventType, description, resourceType: 'webhook', resourceId: id });
  switch (b.action) {
    case 'test': {
      if (w.pausedReason || !w.enabled) return badRequest('Resume or enable the webhook first', 409);
      const deliveryId = enqueueTest(w, ctx.user.email);
      log('webhook.test', `Sent a test delivery to "${w.name}"`);
      return NextResponse.json({ deliveryId });
    }
    case 'pause':
      pauseWebhook(ctx.orgId, id, b.reason ?? `Paused by ${ctx.user.email}`);
      log('webhook.pause', `Paused webhook "${w.name}"`);
      return NextResponse.json({ webhook: getWebhook(ctx.orgId, id) });
    case 'resume':
      resumeWebhook(ctx.orgId, id);
      log('webhook.resume', `Resumed webhook "${w.name}"`);
      return NextResponse.json({ webhook: getWebhook(ctx.orgId, id) });
    case 'rotate_secret': {
      const secret = rotateWebhookSecret(ctx.orgId, id);
      log('webhook.rotate_secret', `Rotated the signing secret of "${w.name}"`);
      return NextResponse.json({ secret });
    }
    case 'redeliver':
      if (!b.deliveryId || !redeliver(ctx.orgId, id, b.deliveryId)) return badRequest('Delivery not found', 404);
      log('webhook.redeliver', `Redelivered ${b.deliveryId} to "${w.name}"`);
      return NextResponse.json({ success: true });
    default:
      return badRequest('Unknown action');
  }
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const w = getWebhook(ctx.orgId, id);
  if (!w || !deleteWebhook(ctx.orgId, id)) return badRequest('Webhook not found', 404);
  audit(ctx, { eventType: 'webhook.delete', description: `Deleted webhook "${w.name}"`, resourceType: 'webhook', resourceId: id });
  return NextResponse.json({ success: true });
}
