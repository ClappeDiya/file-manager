import { NextRequest, NextResponse } from 'next/server';
import { compare, hash } from 'bcryptjs';
import { audit, authenticate, badRequest, isContext, readJson } from '@/lib/session';
import { updateUser } from '@/lib/db/repos/users';
import { checkPassword } from '@/lib/password-policy';
import { loginLimiter } from '@/lib/rate-limit';

/** POST /api/account/password {current, password} — change your own password. */
export async function POST(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const key = `pwdchange:${ctx.user.id}`;
  if (loginLimiter.wait(key)) return badRequest('Too many attempts. Try again later.', 429);
  const b = await readJson<{ current?: string; password?: string }>(request);
  if (!ctx.user.passwordHash) return badRequest('This account signs in with single sign-on and has no password');
  if (!b?.current || !(await compare(b.current, ctx.user.passwordHash))) {
    loginLimiter.fail(key);
    return badRequest('The current password is not correct', 403);
  }
  const check = checkPassword(b.password ?? '', { email: ctx.user.email, name: ctx.user.name });
  if (!check.ok) return NextResponse.json({ error: check.problems[0], problems: check.problems }, { status: 400 });
  if (await compare(b.password!, ctx.user.passwordHash)) return badRequest('Choose a password different from the current one');
  updateUser(ctx.user.id, { passwordHash: await hash(b.password!, 12), passwordChangedAt: new Date().toISOString() });
  audit(ctx, { eventType: 'auth.password_change', description: 'Changed the account password', resourceType: 'user', resourceId: ctx.user.id });
  return NextResponse.json({ success: true });
}
