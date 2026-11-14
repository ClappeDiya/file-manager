import { NextRequest, NextResponse } from 'next/server';
import { readJson } from '@/lib/session';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { decodeUpload, DiagnosticsError, storeDiagnostics } from '@/lib/db/repos/diagnostics';

/** POST /api/agent/diagnostics {fileName, summary, bundleBase64} — a diagnostics zip from the machine. */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const raw = await readJson<Record<string, unknown>>(request);
  if (!raw) return NextResponse.json({ error: 'Invalid JSON' }, { status: 400 });
  try {
    const stored = storeDiagnostics(m.orgId, m.id, decodeUpload(raw));
    return NextResponse.json({ id: stored.id }, { status: 201 });
  } catch (e) {
    if (e instanceof DiagnosticsError) return NextResponse.json({ error: e.message }, { status: e.status });
    throw e;
  }
}
