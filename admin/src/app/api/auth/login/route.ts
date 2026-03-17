import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';
import { cookies } from 'next/headers';

const ADMIN_USERS = [
  {
    email: process.env.ADMIN_EMAIL || 'admin@ufop.local',
    // In production, use hashed passwords and a real user store
    passwordHash: process.env.ADMIN_PASSWORD_HASH || '',
    name: 'Administrator',
    role: 'admin' as const,
  },
];

export async function POST(request: NextRequest) {
  try {
    const { email, password } = await request.json();

    if (!email || !password) {
      return NextResponse.json({ error: 'Email and password are required' }, { status: 400 });
    }

    // Find user
    const user = ADMIN_USERS.find((u) => u.email === email);
    if (!user) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Verify password - for initial setup, allow env-based password
    const adminPassword = process.env.ADMIN_PASSWORD || 'admin';
    if (password !== adminPassword) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Create JWT session token
    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET || 'ufop-admin-secret-change-me');
    const token = await new SignJWT({
      sub: user.email,
      name: user.name,
      role: user.role,
    })
      .setProtectedHeader({ alg: 'HS256' })
      .setExpirationTime('8h')
      .setIssuedAt()
      .sign(secret);

    // Set session cookie
    const cookieStore = await cookies();
    cookieStore.set('ufop-admin-session', token, {
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'lax',
      maxAge: 8 * 60 * 60, // 8 hours
      path: '/',
    });

    return NextResponse.json({
      user: { email: user.email, name: user.name, role: user.role },
    });
  } catch {
    return NextResponse.json({ error: 'Authentication failed' }, { status: 500 });
  }
}
