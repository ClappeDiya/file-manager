/** Server-side CSV user import: parsing, preview, role ceiling, groups, invitations. */
import { describe, expect, it } from 'vitest';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { createGroup } from '@/lib/db/repos/groups';
import { createUser } from '@/lib/db/repos/users';
import { listInvitations } from '@/lib/db/repos/invitations';
import { listAudit } from '@/lib/db/repos/audit';
import { parseCsv, MAX_IMPORT_ROWS } from '@/lib/csv-import';
import { json, makeUser, req } from './helpers';
import * as importRoute from '@/app/api/users/import/route';

type Res = { rows: { line: number; email: string; action: string; message: string }[]; summary: Record<string, number>; error?: string };
const post = async (as: ReturnType<typeof makeUser>, csv: string, dryRun: boolean) => json<Res & { error?: string }>(await importRoute.POST(await req('POST', '/api/users/import', { as, body: { csv, dryRun } })));

describe('CSV user import', () => {
  it('parses RFC 4180 CSV', () => {
    expect(parseCsv('﻿email,name\r\n"a@x.org","Lima, Ana"\n"b@x.org","Say ""hi""\nthere"\n\n')).toEqual([
      ['email', 'name'],
      ['a@x.org', 'Lima, Ana'],
      ['b@x.org', 'Say "hi"\nthere'],
    ]);
    expect(() => parseCsv('email\n"open')).toThrow(/not closed/);
  });

  it('previews, enforces the role ceiling and invites valid rows', async () => {
    const org = createOrg({ name: 'Import org' });
    const admin = makeUser('imp-admin@x.org', 'org_admin', org.id);
    createGroup(org.id, { name: 'Night Shift' });
    const member = createUser({ email: 'member@x.org', role: 'user', organizationId: org.id });
    setMembership(member.id, org.id, 'user');
    const csv = [
      'Email,Role,Group,Group Role',
      'new1@x.org,,,',
      'NEW2@x.org,operator,night shift,owner',
      'new1@x.org,user,,',
      'bad-address,user,,',
      'boss@x.org,org_owner,,',
      'who@x.org,wizard,,',
      'g@x.org,user,No Such Group,',
      'member@x.org,user,,',
      'member@x.org,user,Night Shift,',
    ].join('\n');

    const preview = await post(admin, csv, true);
    expect(preview.status).toBe(200);
    expect(preview.body.rows.map((r) => [r.line, r.action])).toEqual([
      [2, 'invite'],
      [3, 'invite'],
      [4, 'error'],
      [5, 'error'],
      [6, 'error'],
      [7, 'error'],
      [8, 'error'],
      [9, 'skip'],
      [10, 'error'],
    ]);
    expect(preview.body.rows[3].message).toContain('Not a valid');
    expect(preview.body.rows[4].message).toContain('cannot invite');
    expect(preview.body.rows[6].message).toContain('No group');
    expect(preview.body.rows[8].message).toBe('Listed more than once');
    expect(listInvitations(org.id)).toHaveLength(0); // dry run changes nothing

    const done = await post(admin, csv, false);
    expect(done.body.summary).toMatchObject({ invited: 2, skipped: 1, errors: 6 });
    const inv = listInvitations(org.id);
    expect(inv.map((i) => i.email).sort()).toEqual(['new1@x.org', 'new2@x.org']);
    const n2 = inv.find((i) => i.email === 'new2@x.org')!;
    expect(n2).toMatchObject({ role: 'operator', groupRole: 'owner' });
    expect(done.body.rows[0].message).toContain('Invited');
    expect(listAudit({ eventType: 'admin.user_import' }).entries[0].description).toContain('2 invited');
  });

  it('rejects bad files and callers without the invite permission', async () => {
    const org = createOrg({ name: 'Import errors' });
    const admin = makeUser('imp2@x.org', 'org_admin', org.id);
    expect((await post(admin, 'name\nx', true)).status).toBe(400);
    expect((await post(admin, '   ', true)).status).toBe(400);
    expect((await post(admin, 'email\n"unterminated', true)).body.error).toMatch(/not closed/);
    const big = ['email', ...Array.from({ length: MAX_IMPORT_ROWS + 1 }, (_, i) => `u${i}@x.org`)].join('\n');
    expect((await post(admin, big, true)).body.error).toMatch(/At most/);
    const viewer = makeUser('imp-view@x.org', 'read_only_auditor', org.id);
    expect((await post(viewer, 'email\na@x.org', true)).status).toBe(403);
  });
});
