import { NextRequest, NextResponse } from 'next/server';
import { clientIp } from '@/lib/session';
import { machineForReport, touchMachine } from '@/lib/db/repos/machines';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { OrgError } from '@/lib/db/repos/orgs';
import { evaluateReport } from '@/lib/alerts';
import { enrollLimiter } from '@/lib/rate-limit';

/**
 * POST /api/ingest/:reportKey — the body a client sends to a plain report URL
 * (JSON, or form with `data`). The report key is a registration-link token.
 */
export async function POST(request: NextRequest, { params }: { params: Promise<{ token: string }> }) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`ingest:${ip}`)) return NextResponse.json({ error: 'Too many failed requests' }, { status: 429 });
  const { token } = await params;
  let raw: Record<string, unknown> | null;
  const type = request.headers.get('content-type') ?? '';
  try {
    if (type.includes('application/x-www-form-urlencoded') || type.includes('multipart/form-data')) {
      const form = await request.formData();
      const data = form.get('data');
      raw = typeof data === 'string' ? JSON.parse(data) : { message: String(form.get('message') ?? '') };
    } else {
      raw = await request.json();
    }
  } catch {
    return NextResponse.json({ error: 'The report body is not valid JSON' }, { status: 400 });
  }
  if (!raw || typeof raw !== 'object') return NextResponse.json({ error: 'Empty report' }, { status: 400 });
  try {
    const m = machineForReport(decodeURIComponent(token), String(raw.machine ?? raw.machine_name ?? ''));
    if (!m) {
      enrollLimiter.fail(`ingest:${ip}`);
      return NextResponse.json({ error: 'Unknown or revoked report key' }, { status: 401 });
    }
    if (m.status !== 'active') return NextResponse.json({ error: 'This machine is suspended' }, { status: 423 });
    touchMachine(m.id, ip, {});
    const stored = ingestReport(m.orgId, m.id, normalizeReport(raw), raw);
    await evaluateReport(m, stored);
    return NextResponse.json({ id: stored.id, machineId: m.id }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) return NextResponse.json({ error: e.message }, { status: e.status });
    if (e instanceof Error && /backup id/.test(e.message)) return NextResponse.json({ error: e.message }, { status: 422 });
    throw e;
  }
}
