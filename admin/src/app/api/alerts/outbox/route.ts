import { NextRequest, NextResponse } from 'next/server';
import { pageInfo } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
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
  const pr = readPage(request, { defaultLimit: 100 });
  if (pr instanceof NextResponse) return pr;
  const list = listOutbox(ctx.orgId, { status: status ?? undefined, channelId: q.get('channelId') ?? undefined, limit: pr.limit, offset: pr.offset });
  return pagedJson(request, {
    counts: outboxCounts(ctx.orgId),
    channels,
    messages: list.map((m) => ({
      ...m,
      // Recipients and bodies stay on the server; the list shows what and where.
      message: { subject: m.message.subject, recipients: m.message.to?.length ?? null },
    })),
  }, pageInfo(pr, list.length, null));
}
