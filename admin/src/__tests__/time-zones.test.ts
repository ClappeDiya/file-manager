/** User time zones: the profile setting and how the console formats times with it. */
import { afterEach, describe, expect, it } from 'vitest';
import { getUser } from '@/lib/db/repos/users';
import { formatDate, formatDateTime, getDisplayTimeZone, setDisplayTimeZone } from '@/lib/utils/format';
import { json, makeUser, req } from './helpers';
import * as profile from '@/app/api/account/profile/route';
import * as session from '@/app/api/auth/session/route';

afterEach(() => setDisplayTimeZone(null));

describe('user time zones', () => {
  it('stores a valid IANA zone (or none) and returns it with the session', async () => {
    const u = makeUser('tz@x.org', 'operator');
    const set = async (body: Record<string, unknown>) => json<{ error?: string; user?: { timeZone: string | null; name: string } }>(await profile.PATCH(await req('PATCH', '/api/account/profile', { as: u, body })));
    expect((await set({ timeZone: 'Mars/Olympus' })).status).toBe(400);
    expect((await set({ timeZone: 'EST' })).status).toBe(400);
    expect((await set({ timeZone: 42 })).status).toBe(400);
    expect((await set({})).status).toBe(400);
    expect((await set({ name: '' })).status).toBe(400);
    const ok = await set({ timeZone: 'Asia/Kolkata', name: '  Tee Zed ' });
    expect(ok.body.user).toMatchObject({ timeZone: 'Asia/Kolkata', name: 'Tee Zed' });
    expect(getUser(u.id)!.timeZone).toBe('Asia/Kolkata');
    const s = await json<{ user: { timeZone: string } }>(await session.GET(await req('GET', '/api/auth/session', { as: getUser(u.id)! })));
    expect(s.body.user.timeZone).toBe('Asia/Kolkata');
    expect((await set({ timeZone: 'UTC' })).body.user!.timeZone).toBe('UTC');
    expect((await set({ timeZone: null })).body.user!.timeZone).toBeNull();
  });

  it('formats in the chosen zone', () => {
    const at = '2026-03-01T23:30:00Z';
    setDisplayTimeZone('Asia/Tokyo');
    expect(getDisplayTimeZone()).toBe('Asia/Tokyo');
    expect(formatDate(at)).toBe('Mar 2, 2026');
    expect(formatDateTime(at)).toMatch(/Mar 2, 2026, 08:30 AM GMT\+9/);
    setDisplayTimeZone('America/Los_Angeles');
    expect(formatDate(at)).toBe('Mar 1, 2026');
    expect(formatDateTime(at)).toMatch(/03:30 PM PST/);
    setDisplayTimeZone(null);
    expect(getDisplayTimeZone()).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone);
  });
});
