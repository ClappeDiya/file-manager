import { NextResponse } from 'next/server';
import { jwtVerify } from 'jose';
import { dataStore } from '@/lib/data-store';
import { cookies } from 'next/headers';

/** GET /api/auth/session - Validate session and return user */
export async function GET() {
  const jwtSecret = process.env.NEXTAUTH_SECRET;
  if (!jwtSecret) {
    console.error('NEXTAUTH_SECRET environment variable is not set');
    return NextResponse.json(
      { error: 'Server configuration error' },
      { status: 500 }
    );
  }

  const cookieStore = await cookies();
  const sessionToken = cookieStore.get('ufop-admin-session')?.value;

  if (!sessionToken) {
    return NextResponse.json(
      { error: 'No session found' },
      { status: 401 }
    );
  }

  try {
    const secret = new TextEncoder().encode(jwtSecret);
    const { payload } = await jwtVerify(sessionToken, secret);

    const user = dataStore.users.find(candidate => candidate.email === payload.sub && candidate.isActive);
    if (!user || payload.userId && payload.userId !== user.id) {
      return NextResponse.json({ error: 'Session user is unavailable' }, { status: 401 });
    }
    return NextResponse.json({ user });

  } catch {
    return NextResponse.json(
      { error: 'Session expired or invalid' },
      { status: 401 }
    );
  }
}
