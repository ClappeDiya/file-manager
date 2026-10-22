import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { startClaim, type ClientInfo } from '@/lib/db/repos/machines';
import { enrollLimiter } from '@/lib/rate-limit';

/** POST /api/agent/claim {info} — start interactive claiming; show `code` on the machine. */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`claim:${ip}`)) return NextResponse.json({ error: 'Too many attempts' }, { status: 429 });
  enrollLimiter.fail(`claim:${ip}`); // every start counts: claims are cheap to request, expensive to guess
  const b = await readJson<{ info?: ClientInfo }>(request);
  return NextResponse.json(startClaim(b?.info ?? {}), { status: 201 });
}
