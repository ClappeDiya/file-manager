import { NextRequest, NextResponse } from 'next/server';
import { hash } from 'bcryptjs';
import { authenticate, clientIp, isContext, readJson } from '@/lib/session';
import { finishSignIn } from '@/lib/signin';
import { acceptInvitation, findByToken } from '@/lib/db/repos/invitations';
import { getOrg, OrgError } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { enrollLimiter } from '@/lib/rate-limit';
import { checkPassword } from '@/lib/password-policy';

/** GET /api/invitations/accept?token= — what the invitation is for (public). */
export async function GET(request: NextRequest) {
  const token = new URL(request.url).searchParams.get('token') ?? '';
  const inv = token ? findByToken(token) : null;
  if (!inv) return NextResponse.json({ error: 'This invitation link is not valid' }, { status: 404 });
  return NextResponse.json({ email: inv.email, role: inv.role, organization: getOrg(inv.orgId)?.name ?? '', status: inv.status, expiresAt: inv.expiresAt });
}

/** POST /api/invitations/accept {token, name, password} — or with a session, join with the current account. */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`invite:${ip}`)) return NextResponse.json({ error: 'Too many attempts. Try again later.' }, { status: 429 });
  const b = await readJson<{ token?: string; name?: string; password?: string }>(request);
  if (!b?.token) return NextResponse.json({ error: 'The invitation token is missing' }, { status: 400 });
  const session = request.cookies.get('ufop-admin-session') ? await authenticate(request) : null;
  try {
    let result;
    if (session && isContext(session)) {
      result = acceptInvitation(b.token, { existing: session.user });
    } else {
      const inv = findByToken(b.token);
      const check = checkPassword(b.password ?? '', { email: inv?.email, name: b.name });
      if (!check.ok) return NextResponse.json({ error: check.problems[0], problems: check.problems }, { status: 400 });
      result = acceptInvitation(b.token, { name: b.name?.trim() || '', passwordHash: await hash(b.password!, 12) });
    }
    appendAudit({ eventType: 'admin.invite_accept', orgId: result.invitation.orgId, userId: result.user.id, userEmail: result.user.email, ipAddress: ip, description: `${result.user.email} joined as ${result.invitation.role}`, metadata: { invitationId: result.invitation.id } });
    return finishSignIn(result.user, { ip, userAgent: request.headers.get('user-agent') ?? '', method: 'pwd' });
  } catch (e) {
    if (e instanceof OrgError) {
      enrollLimiter.fail(`invite:${ip}`);
      return NextResponse.json({ error: e.message }, { status: e.status });
    }
    throw e;
  }
}
