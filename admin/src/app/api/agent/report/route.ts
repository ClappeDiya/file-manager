import { NextRequest, NextResponse } from 'next/server';
import { readJson } from '@/lib/session';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { evaluateReport } from '@/lib/alerts';

/** POST /api/agent/report — one finished operation (backup, restore, verify…). */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const raw = await readJson<Record<string, unknown>>(request);
  if (!raw) return NextResponse.json({ error: 'Invalid JSON' }, { status: 400 });
  let rep;
  try {
    rep = normalizeReport(raw);
  } catch (e) {
    return NextResponse.json({ error: (e as Error).message }, { status: 422 });
  }
  const stored = ingestReport(m.orgId, m.id, rep, raw);
  await evaluateReport(m, stored);
  return NextResponse.json({ id: stored.id }, { status: 201 });
}
