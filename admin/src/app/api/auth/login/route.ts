import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';
import { cookies } from 'next/headers';
import { compare, hash } from 'bcryptjs';
import { dataStore } from '@/lib/data-store';
import { ROLE_PERMISSIONS } from '@/lib/types/auth';
import { logger } from '@/lib/utils/logger';

/**
 * Rate limiter: tracks failed login attempts per IP and per email.
 * Limits: 5 failed attempts per key per 15-minute window.
 */
const LOGIN_ATTEMPTS: Map<string, { count: number; resetAt: number }> = new Map();
const MAX_ATTEMPTS = 5;
const WINDOW_MS = 15 * 60 * 1000; // 15 minutes

function checkLoginRateLimit(key: string): { allowed: boolean; retryAfterSeconds: number } {
  const now = Date.now();
  const entry = LOGIN_ATTEMPTS.get(key);

  if (!entry || now > entry.resetAt) {
    LOGIN_ATTEMPTS.set(key, { count: 1, resetAt: now + WINDOW_MS });
    return { allowed: true, retryAfterSeconds: 0 };
  }

  if (entry.count >= MAX_ATTEMPTS) {
    const retryAfterSeconds = Math.ceil((entry.resetAt - now) / 1000);
    return { allowed: false, retryAfterSeconds };
  }

  entry.count++;
  return { allowed: true, retryAfterSeconds: 0 };
}

function recordFailedLogin(key: string): void {
  const now = Date.now();
  const entry = LOGIN_ATTEMPTS.get(key);
  if (!entry || now > entry.resetAt) {
    LOGIN_ATTEMPTS.set(key, { count: 1, resetAt: now + WINDOW_MS });
  } else {
    entry.count++;
  }
}

/**
 * POST /api/auth/login — Authenticate a user via email + password.
 *
 * Checks two sources in order:
 * 1. SQLite database users (with password_hash column)
 * 2. Env-var bootstrap admin (ADMIN_EMAIL + ADMIN_PASSWORD)
 *
 * On first run with no DB users, the env-var admin is auto-seeded
 * into the database for persistence.
 */
export async function POST(request: NextRequest) {
  try {
    const jwtSecret = process.env.NEXTAUTH_SECRET;
    if (!jwtSecret) {
      return NextResponse.json(
        { error: 'Server configuration error' },
        { status: 500 }
      );
    }

    const { email, password } = await request.json();

    if (!email || !password) {
      return NextResponse.json({ error: 'Email and password are required' }, { status: 400 });
    }

    // Rate limit by IP and email
    const ip = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim() || 'unknown';
    const ipCheck = checkLoginRateLimit(`ip:${ip}`);
    const emailCheck = checkLoginRateLimit(`email:${email.toLowerCase()}`);

    if (!ipCheck.allowed || !emailCheck.allowed) {
      const retryAfter = Math.max(ipCheck.retryAfterSeconds, emailCheck.retryAfterSeconds);
      logger.warn('auth.rate_limited', { ip, email, retryAfter });
      return NextResponse.json(
        { error: 'Too many login attempts. Please try again later.' },
        { status: 429, headers: { 'Retry-After': String(retryAfter) } }
      );
    }

    // Auto-seed the bootstrap admin into DB on first access
    await ensureBootstrapAdmin();

    // Look up user in SQLite database
    const dbUser = dataStore.findUserByEmail(email);
    if (!dbUser || !dbUser.passwordHash) {
      recordFailedLogin(`ip:${ip}`);
      recordFailedLogin(`email:${email.toLowerCase()}`);
      logger.warn('auth.login_failed', { ip, email, reason: 'user_not_found' });
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    if (!dbUser.isActive) {
      recordFailedLogin(`ip:${ip}`);
      logger.warn('auth.login_failed', { ip, email, reason: 'account_deactivated' });
      return NextResponse.json({ error: 'Account is deactivated' }, { status: 403 });
    }

    // Verify password using bcrypt (timing-safe comparison)
    const isValid = await compare(password, dbUser.passwordHash);
    if (!isValid) {
      recordFailedLogin(`ip:${ip}`);
      recordFailedLogin(`email:${email.toLowerCase()}`);
      logger.warn('auth.login_failed', { ip, email, reason: 'invalid_password' });
      return NextResponse.json({ error: 'Invalid credentials' }, { status: 401 });
    }

    // Update last login timestamp
    dataStore.updateUser(dbUser.id, { lastLoginAt: new Date().toISOString() });

    // Create JWT session token
    const secret = new TextEncoder().encode(jwtSecret);
    const token = await new SignJWT({
      sub: dbUser.email,
      name: dbUser.name,
      role: dbUser.role,
      userId: dbUser.id,
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

    logger.info('auth.login_success', { ip, email: dbUser.email, role: dbUser.role, userId: dbUser.id });

    return NextResponse.json({
      user: { id: dbUser.id, email: dbUser.email, name: dbUser.name, role: dbUser.role },
    });
  } catch (err) {
    logger.error('auth.login_error', { error: err instanceof Error ? err.message : 'unknown' });
    return NextResponse.json({ error: 'Authentication failed' }, { status: 500 });
  }
}

/**
 * Seed the bootstrap admin from env vars into SQLite on first run.
 * This ensures the admin console is accessible even on a fresh database.
 */
async function ensureBootstrapAdmin() {
  const adminEmail = process.env.ADMIN_EMAIL || 'admin@ufop.local';
  const adminPasswordRaw = process.env.ADMIN_PASSWORD;

  if (!adminPasswordRaw) return; // No bootstrap admin configured

  const existing = dataStore.findUserByEmail(adminEmail);
  if (existing) return; // Already seeded

  // ADMIN_PASSWORD can be either a bcrypt hash ($2a$...) or a plaintext password
  const passwordHash = adminPasswordRaw.startsWith('$2')
    ? adminPasswordRaw
    : await hash(adminPasswordRaw, 12);

  const { v4: uuidv4 } = await import('uuid');
  dataStore.addUser({
    id: uuidv4(),
    email: adminEmail,
    name: 'Administrator',
    role: 'org_owner',
    permissions: ROLE_PERMISSIONS.org_owner,
    organizationId: 'org_default',
    createdAt: new Date().toISOString(),
    isActive: true,
    passwordHash,
  });

  // Bootstrap admin seeded — do not log email to avoid leaking identity
}
