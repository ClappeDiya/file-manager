import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext } from '@/lib/session';
import { identitiesOf, providersForDomain, unlinkIdentity, listProviders } from '@/lib/db/repos/sso';

/** GET /api/account/identities — linked sign-in methods plus the ones that can be linked. */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const linked = identitiesOf(ctx.user.id);
  const domain = ctx.user.email.split('@')[1] ?? '';
  const candidates = new Map([...listProviders(ctx.orgId), ...providersForDomain(domain)].filter((p) => p.enabled).map((p) => [p.id, { id: p.id, name: p.name, kind: p.kind }]));
  for (const l of linked) candidates.delete(l.providerId);
  return NextResponse.json({ hasPassword: !!ctx.user.passwordHash, identities: linked, available: [...candidates.values()] });
}

/** DELETE /api/account/identities?id= — unlink, keeping at least one way to sign in. */
export async function DELETE(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const id = request.nextUrl.searchParams.get('id') ?? '';
  const linked = identitiesOf(ctx.user.id);
  const target = linked.find((l) => l.id === id);
  if (!target) return badRequest('Not found', 404);
  if (!ctx.user.passwordHash && linked.length <= 1) return badRequest('This is your only way to sign in. Link another method or set a password first.', 409);
  unlinkIdentity(ctx.user.id, id);
  audit(ctx, { eventType: 'auth.identity_unlink', description: `Unlinked ${target.providerName} sign-in`, resourceType: 'user', resourceId: ctx.user.id });
  return NextResponse.json({ ok: true });
}
