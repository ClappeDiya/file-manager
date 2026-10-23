import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext, readJson } from '@/lib/session';
import { getUser, publicUser, updateUser } from '@/lib/db/repos/users';
import { isTimeZone } from '@/lib/zoned-time';

/** The console languages (kept in step with `lib/i18n`, which is client-side). */
const SUPPORTED = ['en', 'de', 'es', 'fr', 'ja', 'ko', 'pt', 'zh'];

/** PATCH /api/account/profile {name?, timeZone? (IANA name, or null for the browser's), locale? (en, de, …, or null)} */
export async function PATCH(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ name?: unknown; timeZone?: unknown; locale?: unknown }>(request)) ?? {};
  const patch: { name?: string; timeZone?: string | null; locale?: string | null } = {};
  if (b.name !== undefined) {
    const name = typeof b.name === 'string' ? b.name.trim() : '';
    if (!name || name.length > 100) return badRequest('A name of 1–100 characters is required');
    patch.name = name;
  }
  if (b.timeZone !== undefined) {
    // IANA area/location names (or UTC), not abbreviations like "EST" that some engines accept.
    const named = typeof b.timeZone === 'string' && (b.timeZone === 'UTC' || b.timeZone.includes('/')) && isTimeZone(b.timeZone);
    if (b.timeZone !== null && !named) return badRequest('Choose a time zone such as Europe/Berlin, or none to use the browser’s');
    patch.timeZone = b.timeZone as string | null;
  }
  if (b.locale !== undefined) {
    if (b.locale !== null && !SUPPORTED.includes(b.locale as string)) return badRequest(`The language is one of ${SUPPORTED.join(', ')}, or none to use the browser's`);
    patch.locale = b.locale as string | null;
  }
  if (!Object.keys(patch).length) return badRequest('Nothing to change');
  updateUser(ctx.user.id, patch);
  audit(ctx, { eventType: 'account.profile_update', description: `Updated profile (${Object.keys(patch).join(', ')})`, resourceType: 'user', resourceId: ctx.user.id, metadata: patch });
  return NextResponse.json({ user: publicUser(getUser(ctx.user.id)!) });
}
