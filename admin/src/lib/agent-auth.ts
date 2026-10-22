/** Authentication for machine agents: `Authorization: Bearer ufop_agent_…`. */
import { NextRequest, NextResponse } from 'next/server';
import { machineByToken, touchMachine, type Machine } from './db/repos/machines';
import { clientIp } from './session';
import { enrollLimiter } from './rate-limit';

export async function agentAuth(req: NextRequest): Promise<Machine | NextResponse> {
  const ip = clientIp(req);
  if (enrollLimiter.wait(`agent:${ip}`)) return NextResponse.json({ error: 'Too many failed requests' }, { status: 429 });
  const h = req.headers.get('authorization') ?? '';
  const token = h.startsWith('Bearer ') ? h.slice(7).trim() : '';
  const m = token ? machineByToken(token) : null;
  if (!m) {
    enrollLimiter.fail(`agent:${ip}`);
    return NextResponse.json({ error: 'Unknown or revoked machine token', code: 'unauthorized' }, { status: 401 });
  }
  if (m.status === 'suspended') return NextResponse.json({ error: 'This machine is suspended in the console', code: 'suspended' }, { status: 423 });
  touchMachine(m.id, ip, {});
  return m;
}

export function isMachine(x: Machine | NextResponse): x is Machine {
  return !(x instanceof NextResponse);
}
