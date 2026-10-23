import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { acknowledgeAlert, getAlert, resolveAlert, snoozeAlert, unacknowledgeAlert } from '@/lib/db/repos/alerts';

const MAX_SNOOZE_MINUTES = 30 * 24 * 60;

/**
 * PATCH /api/alerts/:id {action?}
 * - `resolve` (default, also an empty body): mark resolved by hand
 * - `acknowledge` / `unacknowledge`: stays open, no further notifications
 * - `snooze` {minutes}: no notifications until then (0 wakes it up)
 */
export async function PATCH(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const a = getAlert(ctx.orgId, id);
  if (!a) return badRequest('Alert not found', 404);
  const b = (await readJson<{ action?: string; minutes?: number }>(request)) ?? {};
  const action = b.action ?? 'resolve';
  if (action !== 'resolve' && a.resolvedAt) return badRequest('The alert is already resolved', 409);
  switch (action) {
    case 'resolve':
      resolveAlert(id);
      audit(ctx, { eventType: 'alert.resolve', description: `Resolved: ${a.message}`, resourceType: 'alert', resourceId: id });
      break;
    case 'acknowledge':
      acknowledgeAlert(ctx.orgId, id, ctx.user.email);
      audit(ctx, { eventType: 'alert.acknowledge', description: `Acknowledged: ${a.message}`, resourceType: 'alert', resourceId: id });
      break;
    case 'unacknowledge':
      unacknowledgeAlert(ctx.orgId, id);
      audit(ctx, { eventType: 'alert.unacknowledge', description: `Unacknowledged: ${a.message}`, resourceType: 'alert', resourceId: id });
      break;
    case 'snooze': {
      const minutes = Number(b.minutes);
      if (!Number.isInteger(minutes) || minutes < 0 || minutes > MAX_SNOOZE_MINUTES) return badRequest('Snooze for 0 minutes to 30 days');
      snoozeAlert(ctx.orgId, id, minutes ? new Date(Date.now() + minutes * 60_000) : null);
      audit(ctx, { eventType: 'alert.snooze', description: minutes ? `Snoozed for ${minutes} min: ${a.message}` : `Woke up: ${a.message}`, resourceType: 'alert', resourceId: id, metadata: { minutes } });
      break;
    }
    default:
      return badRequest('Unknown action');
  }
  return NextResponse.json({ alert: getAlert(ctx.orgId, id) });
}
