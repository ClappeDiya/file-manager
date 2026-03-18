import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';
import { cookies } from 'next/headers';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';

const PASSWORD_FILE = path.join(process.cwd(), '.data', 'admin-password.json');

const ADMIN_USERS = [
  {
    email: process.env.ADMIN_EMAIL || 'admin@ufop.local',
    // In production, use hashed passwords and a real user store
    name: 'Administrator',
    role: 'admin' as const,
  },
];

function verifyStoredPassword(password: string): boolean {
  try {
    if (fs.existsSync(PASSWORD_FILE)) {
      const data = JSON.parse(fs.readFileSync(PASSWORD_FILE, 'utf-8'));
      const [salt, hash] = data.hash.split(':');
      const verify = crypto.pbkdf2Sync(password, salt, 100000, 64, 'sha512').toString('hex');
      return verify === hash;
    }
  } catch {
    // Fall through to env-based check
  }
  return false;
}

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

    // Check password: first try stored password (set via recovery/reset),
    // then fall back to env-based password
    const hasStoredPassword = fs.existsSync(PASSWORD_FILE);
    let passwordValid = false;

    if (hasStoredPassword) {
      passwordValid = verifyStoredPassword(password);
    }

    if (!passwordValid) {
      // Fall back to env-based password (required in production)
      const adminPassword = process.env.ADMIN_PASSWORD;
      if (!adminPassword) {
        console.error('[security] ADMIN_PASSWORD environment variable is not set');
        return NextResponse.json({ error: 'Server not configured — set ADMIN_PASSWORD env var' }, { status: 500 });
      }
      passwordValid = password === adminPassword;
    }

    if (!passwordValid) {
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Create JWT session token
    if (!process.env.NEXTAUTH_SECRET) {
      console.error('[security] NEXTAUTH_SECRET environment variable is not set');
      return NextResponse.json({ error: 'Server not configured — set NEXTAUTH_SECRET env var' }, { status: 500 });
    }
    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET);
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
