import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createWebhook, deliveryStats, EVENT_CATALOG, listWebhooks, type WebhookInput } from '@/lib/db/repos/webhooks';
import { orgErrorResponse } from '@/lib/route-helpers';

/**
 * Outbound webhooks of the current organization.
 *
 * GET  /api/webhooks — subscriptions with delivery counts, and the event catalog
 * POST /api/webhooks {name, url, events, headers?, enabled?} — returns the signing secret once
 */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({
    webhooks: listWebhooks(ctx.orgId).map((w) => ({ ...w, stats: deliveryStats(ctx.orgId, w.id) })),
    events: EVENT_CATALOG,
  });
}

export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<WebhookInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const { webhook, secret } = createWebhook(ctx.orgId, b, ctx.user.email);
    audit(ctx, { eventType: 'webhook.create', description: `Created webhook "${webhook.name}" (${new URL(webhook.url).host})`, resourceType: 'webhook', resourceId: webhook.id, metadata: { events: webhook.events } });
    return NextResponse.json({ webhook, secret }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
