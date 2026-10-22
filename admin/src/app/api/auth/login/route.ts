import { NextRequest, NextResponse } from 'next/server';
import { compare, hash } from 'bcryptjs';
import { logger } from '@/lib/utils/logger';
import { clientIp, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { createUser, findUserByEmail, listUsers } from '@/lib/db/repos/users';
import { setMembership } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { loginLimiter } from '@/lib/rate-limit';

/** Seed ADMIN_EMAIL / ADMIN_PASSWORD as the owner of the default organization on an empty database. */
async function ensureBootstrapAdmin() {
  const pass = process.env.ADMIN_PASSWORD;
  if (!pass || listUsers().length > 0) return;
  const email = process.env.ADMIN_EMAIL || 'admin@ufop.local';
  const passwordHash = pass.startsWith('$2') ? pass : await hash(pass, 12);
  const u = createUser({ email, name: 'Administrator', role: 'org_owner', organizationId: 'org_default', passwordHash });
  setMembership(u.id, 'org_default', 'org_owner');
}

/** POST /api/auth/login — email + password; may answer `mfaRequired` with a challenge id. */
export async function POST(request: NextRequest) {
  if (!process.env.NEXTAUTH_SECRET) return NextResponse.json({ error: 'Server configuration error' }, { status: 500 });
  const body = await readJson<{ email?: string; password?: string; redirect?: string }>(request);
  const email = body?.email?.trim().toLowerCase();
  const password = body?.password;
  if (!email || !password) return NextResponse.json({ error: 'Email and password are required' }, { status: 400 });

  const ip = clientIp(request);
  const wait = Math.max(loginLimiter.wait(`ip:${ip}`), loginLimiter.wait(`email:${email}`));
  if (wait) {
    logger.warn('auth.rate_limited', { ip, retryAfter: wait });
    return NextResponse.json({ error: 'Too many login attempts. Please try again later.' }, { status: 429, headers: { 'Retry-After': String(wait) } });
  }

  await ensureBootstrapAdmin();
  const user = findUserByEmail(email);
  const fail = (reason: string, status = 401, message = 'Invalid credentials') => {
    loginLimiter.fail(`ip:${ip}`);
    loginLimiter.fail(`email:${email}`);
    appendAudit({ eventType: 'auth.login_failed', severity: 'warning', orgId: user?.organizationId ?? '', userId: user?.id ?? '', userEmail: email, ipAddress: ip, description: `Failed sign-in (${reason})` });
    return NextResponse.json({ error: message }, { status });
  };
  if (!user || !user.passwordHash) return fail('unknown account or no password');
  if (!(await compare(password, user.passwordHash))) return fail('wrong password');
  if (!user.isActive) return fail('account deactivated', 403, 'Account is deactivated');

  const redirect = typeof body?.redirect === 'string' && body.redirect.startsWith('/') && !body.redirect.startsWith('//') ? body.redirect : '/';
  return finishSignIn(user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: 'pwd', redirect });
}
