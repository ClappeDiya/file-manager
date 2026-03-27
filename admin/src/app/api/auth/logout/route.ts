import { NextRequest, NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { logger } from '@/lib/utils/logger';

/**
 * POST /api/auth/logout
 * Clear the session cookie and invalidate the user session.
 */
export async function POST(request: NextRequest) {
  const email = request.headers.get('x-user-email') || 'unknown';
  const ip = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim() || 'unknown';

  const cookieStore = await cookies();
  cookieStore.set('ufop-admin-session', '', {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'strict',
    maxAge: 0,
    path: '/',
  });

  logger.info('auth.logout', { email, ip });

  return NextResponse.json({ success: true });
}
