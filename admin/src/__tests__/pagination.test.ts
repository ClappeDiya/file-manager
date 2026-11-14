/** One paging convention: limit + opaque cursor (legacy offset/page accepted), `pagination`, X-Total-Count and Link. */
import { describe, expect, it } from 'vitest';
import { appendAudit } from '@/lib/db/repos/audit';
import { createOrg } from '@/lib/db/repos/orgs';
import { decodeCursor, encodeCursor, pageInfo, pageRequest, paged, BadCursor } from '@/lib/pagination';
import { json, makeUser, req } from './helpers';
import * as audit from '@/app/api/audit/route';
import * as users from '@/app/api/users/route';

type Page = { limit: number | null; offset: number; total: number | null; nextCursor: string | null; hasMore: boolean };

describe('pagination helpers', () => {
  it('reads limit, cursor and legacy parameters and builds page info', () => {
    const sp = (q: string) => new URLSearchParams(q);
    expect(pageRequest(sp(''))).toEqual({ limit: 50, offset: 0 });
    expect(pageRequest(sp('limit=10000'))).toEqual({ limit: 500, offset: 0 });
    expect(pageRequest(sp('limit=0'))).toEqual({ limit: 50, offset: 0 });
    expect(pageRequest(sp(`limit=20&cursor=${encodeCursor(40)}`))).toEqual({ limit: 20, offset: 40 });
    expect(pageRequest(sp('offset=7'))).toEqual({ limit: 50, offset: 7 });
    expect(pageRequest(sp('page=3&pageSize=25'))).toEqual({ limit: 25, offset: 50 });
    expect(pageRequest(sp(''), { defaultLimit: Infinity })).toEqual({ limit: Infinity, offset: 0 });
    expect(() => pageRequest(sp('cursor=garbage'))).toThrow(BadCursor);
    expect(decodeCursor(encodeCursor(123))).toBe(123);
    expect(pageInfo({ limit: 10, offset: 0 }, 10, 25)).toMatchObject({ hasMore: true, nextCursor: encodeCursor(10), total: 25 });
    expect(pageInfo({ limit: 10, offset: 20 }, 5, 25)).toMatchObject({ hasMore: false, nextCursor: null });
    expect(pageInfo({ limit: 10, offset: 0 }, 10, null)).toMatchObject({ hasMore: true }); // unknown total: a full page may have more
    expect(paged([1, 2, 3], { limit: Infinity, offset: 1 })).toEqual({ items: [2, 3], page: { limit: null, offset: 1, total: 3, nextCursor: null, hasMore: false } });
  });
});

describe('paged routes', () => {
  it('walks the audit log with cursors and Link headers, without gaps or repeats', async () => {
    const org = createOrg({ name: 'Paging org' });
    const admin = makeUser('page-admin@x.org', 'org_admin', org.id);
    for (let i = 0; i < 23; i++) appendAudit({ eventType: 'paging.test', orgId: org.id, description: `p${i}` });
    const seen: string[] = [];
    let url = '/api/audit?eventType=paging.test&limit=10';
    for (let guard = 0; guard < 10; guard++) {
      const res = await audit.GET(await req('GET', url, { as: admin }));
      const body = (await res.json()) as { entries: { description: string }[]; pagination: Page; total: number };
      expect(res.headers.get('x-total-count')).toBe('23');
      seen.push(...body.entries.map((e) => e.description));
      if (!body.pagination.hasMore) {
        expect(res.headers.get('link')).toBeNull();
        break;
      }
      const link = /<([^>]+)>; rel="next"/.exec(res.headers.get('link') ?? '')![1];
      expect(link).toContain(`cursor=${body.pagination.nextCursor}`);
      url = link;
    }
    expect(seen).toHaveLength(23);
    expect(new Set(seen).size).toBe(23);
    // Legacy page/pageSize still answer the same way.
    const legacy = await json<{ page: number; pageSize: number; totalPages: number; entries: unknown[] }>(await audit.GET(await req('GET', '/api/audit?eventType=paging.test&page=3&pageSize=10', { as: admin })));
    expect(legacy.body).toMatchObject({ page: 3, pageSize: 10, totalPages: 3 });
    expect(legacy.body.entries).toHaveLength(3);
    expect((await audit.GET(await req('GET', '/api/audit?cursor=not-ours', { as: admin }))).status).toBe(400);
  });

  it('keeps whole lists for screens that show everything, and pages them on request', async () => {
    const org = createOrg({ name: 'Paging members' });
    const admin = makeUser('pm-admin@x.org', 'org_admin', org.id);
    for (let i = 0; i < 4; i++) makeUser(`pm${i}@x.org`, 'operator', org.id);
    const all = await json<{ users: unknown[]; pagination: Page }>(await users.GET(await req('GET', '/api/users', { as: admin })));
    expect(all.body.users).toHaveLength(5);
    expect(all.body.pagination).toMatchObject({ limit: null, total: 5, hasMore: false });
    const first = await json<{ users: unknown[]; pagination: Page }>(await users.GET(await req('GET', '/api/users?limit=2', { as: admin })));
    expect(first.body.users).toHaveLength(2);
    expect(first.body.pagination).toMatchObject({ limit: 2, total: 5, hasMore: true });
    const next = await json<{ users: unknown[] }>(await users.GET(await req('GET', `/api/users?limit=2&cursor=${first.body.pagination.nextCursor}`, { as: admin })));
    expect(next.body.users).toHaveLength(2);
  });
});

describe('audit explorer filters on the server', () => {
  it('finds entries beyond the first page (regression: the page filtered only the 50 it had loaded)', async () => {
    const org = createOrg({ name: 'Filter org' });
    const admin = makeUser('filter-admin@x.org', 'org_admin', org.id);
    appendAudit({ eventType: 'admin.role_change', orgId: org.id, description: 'needle in the haystack' });
    for (let i = 0; i < 120; i++) appendAudit({ eventType: 'machine.heartbeat', orgId: org.id, description: `hay ${i}` });
    const found = await json<{ entries: { description: string }[]; total: number }>(await audit.GET(await req('GET', '/api/audit?search=needle', { as: admin })));
    expect(found.body.entries.map((e) => e.description)).toEqual(['needle in the haystack']);
    const admins = await json<{ entries: { eventType: string }[]; total: number }>(await audit.GET(await req('GET', '/api/audit?category=Admin', { as: admin })));
    expect(admins.body.entries.map((e) => e.eventType)).toEqual(['admin.role_change']);
    const counts = await json<{ counts: Record<string, number> }>(await audit.GET(await req('GET', '/api/audit?view=counts', { as: admin })));
    expect(counts.body.counts).toMatchObject({ 'admin.role_change': 1, 'machine.heartbeat': 120 });
  });
});
