import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createChannel, listChannels, type ChannelInput } from '@/lib/db/repos/channels';
import { orgErrorResponse } from '@/lib/route-helpers';

export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ channels: listChannels(ctx.orgId) });
}

export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<ChannelInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const c = createChannel(ctx.orgId, b);
    audit(ctx, { eventType: 'alert.channel_create', description: `Added ${c.kind} channel "${c.name}"`, resourceType: 'channel', resourceId: c.id });
    return NextResponse.json({ channel: c }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
