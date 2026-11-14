import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { getAiSettings, setAiSettings, type AiSettingsInput } from '@/lib/ai';
import { orgErrorResponse } from '@/lib/route-helpers';

export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ settings: getAiSettings(ctx.orgId) });
}

/** PUT — enable, model, custom instructions, schedule, channels, API key (write-only). */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<AiSettingsInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const s = setAiSettings(ctx.orgId, b);
    audit(ctx, { eventType: 'ai.settings', description: `Updated AI analysis settings (enabled: ${s.enabled}, schedule: ${s.schedule})`, metadata: { keyChanged: b.apiKey !== undefined } });
    return NextResponse.json({ settings: s });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
