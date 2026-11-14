import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteChannel, updateChannel, type ChannelInput } from '@/lib/db/repos/channels';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

export async function PUT(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<Partial<ChannelInput>>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const c = updateChannel(ctx.orgId, id, b);
    if (!c) return badRequest('Channel not found', 404);
    audit(ctx, { eventType: 'alert.channel_update', description: `Updated channel "${c.name}"`, resourceType: 'channel', resourceId: id });
    return NextResponse.json({ channel: c });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!deleteChannel(ctx.orgId, id)) return badRequest('Channel not found', 404);
  audit(ctx, { eventType: 'alert.channel_delete', description: `Deleted channel ${id}`, resourceType: 'channel', resourceId: id });
  return NextResponse.json({ success: true });
}
