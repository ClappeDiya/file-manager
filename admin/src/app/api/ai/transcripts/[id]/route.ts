import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { getTranscript } from '@/lib/ai';

/** GET /api/ai/transcripts/:reportId — the full exchange with the model, checked against the audit chain. */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'ai.read', 'audit.read');
  if (!isContext(ctx)) return ctx;
  const t = getTranscript(ctx.orgId, (await params).id);
  if (!t) return badRequest('Transcript not found', 404);
  audit(ctx, { eventType: 'ai.transcript_view', description: `Viewed the AI transcript of report ${t.reportId}`, resourceType: 'ai_report', resourceId: t.reportId });
  return NextResponse.json({ transcript: t });
}
