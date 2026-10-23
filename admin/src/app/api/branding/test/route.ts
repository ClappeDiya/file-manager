import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { EMAIL_TEMPLATES, renderTemplate, SAMPLE_VARS, type EmailTemplateKey } from '@/lib/branding';
import { getChannel } from '@/lib/db/repos/channels';
import { deliver } from '@/lib/mailer';

/** POST /api/branding/test {template, channelId} — send the template with example values to yourself through an email channel. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.write');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ template?: string; channelId?: string }>(request)) ?? {};
  if (!b.template || !(b.template in EMAIL_TEMPLATES)) return badRequest('Choose an email template');
  const ch = b.channelId ? getChannel(ctx.orgId, b.channelId) : null;
  if (!ch || ch.kind !== 'email') return badRequest('Choose an email channel');
  const key = b.template as EmailTemplateKey;
  const d = await deliver(ch, { to: [ctx.user.email], ...renderTemplate(ctx.orgId, key, SAMPLE_VARS[key]) });
  audit(ctx, { eventType: 'branding.test_email', description: `Sent a test ${EMAIL_TEMPLATES[key].label.toLowerCase()} email to ${ctx.user.email}`, resourceType: 'channel', resourceId: ch.id, metadata: { ok: d.ok } });
  return NextResponse.json(d, { status: d.ok ? 200 : 502 });
}
