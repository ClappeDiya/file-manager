import { NextRequest, NextResponse } from 'next/server';
import { readJson } from '@/lib/session';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { completeCommand } from '@/lib/db/repos/commands';

/** POST /api/agent/commands/:id {ok, result} — the agent's answer to a command. */
export async function POST(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const { id } = await params;
  const b = await readJson<{ ok?: boolean; result?: unknown }>(request);
  if (!completeCommand(m.id, id, !!b?.ok, b?.result ?? null)) return NextResponse.json({ error: 'No open command with this id' }, { status: 404 });
  return NextResponse.json({ ok: true });
}
