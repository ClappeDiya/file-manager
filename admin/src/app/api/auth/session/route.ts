import { NextResponse } from 'next/server';
import { jwtVerify } from 'jose';
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

    return NextResponse.json({
      user: {
        email: payload.sub || '',
        name: String(payload.name || ''),
        role: String(payload.role || 'viewer'),
      },
    });
  } catch {
    return NextResponse.json(
      { error: 'Session expired or invalid' },
      { status: 401 }
    );
  }
}
