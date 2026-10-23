import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify } from 'jose';
import { logger } from '@/lib/utils/logger';
import { clientIp, SESSION_COOKIE } from '@/lib/session';
import { revokeSession } from '@/lib/db/repos/sessions';

/** POST /api/auth/logout — end the server-side session and clear the cookie. */
export async function POST(request: NextRequest) {
  const token = request.cookies.get(SESSION_COOKIE)?.value;
  let email = 'unknown';
  if (token && process.env.NEXTAUTH_SECRET) {
    try {
      const { payload } = await jwtVerify(token, new TextEncoder().encode(process.env.NEXTAUTH_SECRET));
      if (typeof payload.sid === 'string') revokeSession(payload.sid, 'signed out');
      email = String(payload.sub ?? email);
    } catch {
      // An expired or foreign cookie: just clear it.
    }
  }
  logger.info('auth.logout', { email, ip: clientIp(request) });
  const res = NextResponse.json({ success: true });
  res.cookies.set(SESSION_COOKIE, '', { httpOnly: true, secure: process.env.NODE_ENV === 'production', sameSite: 'strict', maxAge: 0, path: '/' });
  return res;
}
