import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, requirePermission } from '@/lib/session';
import { listOutbox, outboxCounts, OUTBOX_STATUSES, type OutboxStatus } from '@/lib/db/repos/outbox';
import { listChannels } from '@/lib/db/repos/channels';

/** GET /api/alerts/outbox[?status=&channelId=&limit=] — notification delivery status. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  const q = new URL(request.url).searchParams;
  const status = q.get('status') as OutboxStatus | null;
  if (status && !OUTBOX_STATUSES.includes(status)) return badRequest('Unknown status');
  const channels = Object.fromEntries(listChannels(ctx.orgId).map((c) => [c.id, { name: c.name, kind: c.kind }]));
  return NextResponse.json({
    counts: outboxCounts(ctx.orgId),
    channels,
    messages: listOutbox(ctx.orgId, { status: status ?? undefined, channelId: q.get('channelId') ?? undefined, limit: Number(q.get('limit')) || 100 }).map((m) => ({
      ...m,
      // Recipients and bodies stay on the server; the list shows what and where.
      message: { subject: m.message.subject, recipients: m.message.to?.length ?? null },
    })),
  });
}
