import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { EMAIL_TEMPLATES, previewTemplate, type EmailTemplateKey } from '@/lib/branding';
import { orgErrorResponse } from '@/lib/route-helpers';

/** POST /api/branding/preview {template, subject?, body?} — a template rendered with example values, as text and branded HTML. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.write');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ template?: string; subject?: string; body?: string }>(request)) ?? {};
  if (!b.template || !(b.template in EMAIL_TEMPLATES)) return badRequest('Choose an email template');
  try {
    return NextResponse.json(previewTemplate(ctx.orgId, b.template as EmailTemplateKey, { subject: b.subject, body: b.body }));
  } catch (e) {
    return orgErrorResponse(e);
  }
}
