import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';
import { cookies } from 'next/headers';
import { compare } from 'bcryptjs';

const ADMIN_USERS = [
  {
    email: process.env.ADMIN_EMAIL || 'admin@ufop.local',
    name: 'Administrator',
    role: 'admin' as const,
  },
];

export async function POST(request: NextRequest) {
  try {
    // Security: fail hard if required env vars are not set
    const adminPasswordHash = process.env.ADMIN_PASSWORD;
    if (!adminPasswordHash) {
      console.error('ADMIN_PASSWORD environment variable is not set');
      return NextResponse.json(
        { error: 'Server configuration error' },
        { status: 500 }
      );
    }

    const jwtSecret = process.env.NEXTAUTH_SECRET;
    if (!jwtSecret) {
      console.error('NEXTAUTH_SECRET environment variable is not set');
      return NextResponse.json(
        { error: 'Server configuration error' },
        { status: 500 }
      );
    }

    const { email, password } = await request.json();

    if (!email || !password) {
      return NextResponse.json({ error: 'Email and password are required' }, { status: 400 });
    }

    // Find user
    const user = ADMIN_USERS.find((u) => u.email === email);
    if (!user) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Security: verify password using bcrypt (timing-safe comparison)
    // ADMIN_PASSWORD env var must contain a bcrypt hash, e.g.:
    //   node -e "require('bcryptjs').hash('yourpassword', 12).then(console.log)"
    const isValid = await compare(password, adminPasswordHash);
    if (!isValid) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Create JWT session token
    const secret = new TextEncoder().encode(jwtSecret);
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
      sameSite: 'strict',
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
