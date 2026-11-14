import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext } from '@/lib/session';
import { activeSessions, getSession, revokeSession, revokeUserSessions } from '@/lib/db/repos/sessions';

/** GET /api/account/sessions — your active sessions (this one marked `current`). */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ sessions: activeSessions(ctx.user.id).map((s) => ({ ...s, current: s.id === ctx.sid })) });
}

/** DELETE /api/account/sessions?id= — end one session; ?others=1 — every other session. */
export async function DELETE(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const q = new URL(request.url).searchParams;
  if (q.get('others') === '1') {
    const n = revokeUserSessions(ctx.user.id, 'signed out from another session', ctx.sid);
    audit(ctx, { eventType: 'auth.sessions_revoke', description: `Signed out ${n} other session(s)`, resourceType: 'user', resourceId: ctx.user.id });
    return NextResponse.json({ revoked: n });
  }
  const id = q.get('id') ?? '';
  const s = getSession(id);
  if (!s || s.userId !== ctx.user.id || !revokeSession(id, 'signed out from another session')) return badRequest('Session not found', 404);
  audit(ctx, { eventType: 'auth.sessions_revoke', description: 'Signed out a session', resourceType: 'user', resourceId: ctx.user.id, metadata: { sessionId: id } });
  return NextResponse.json({ revoked: 1 });
}
