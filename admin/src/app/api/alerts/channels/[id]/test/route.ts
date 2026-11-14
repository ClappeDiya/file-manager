import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, requirePermission } from '@/lib/session';
import { getChannel } from '@/lib/db/repos/channels';
import { deliver } from '@/lib/mailer';

/** POST /api/alerts/channels/:id/test — send a test message now. */
export async function POST(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const ch = getChannel(ctx.orgId, (await params).id);
  if (!ch) return badRequest('Channel not found', 404);
  const d = await deliver(ch, { subject: '[UFOP] Test notification', text: `This is a test from the UFOP backup console, sent by ${ctx.user.email}.`, data: { event: 'test' } });
  return NextResponse.json(d, { status: d.ok ? 200 : 502 });
}
