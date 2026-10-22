/**
 * SCIM 2.0 provisioning, with request shapes Entra ID and Okta send:
 * token auth and organization scoping, user create/lookup/patch/deactivate,
 * groups with member patches, filters and paging, errors, and the guards
 * that keep SCIM away from administrators and console-made groups.
 */
import { describe, expect, it } from 'vitest';
import { NextRequest } from 'next/server';
import { createOrg, effectiveRole, membershipsOf, setMembership } from '@/lib/db/repos/orgs';
import { createGroup, groupMembers, updateGroup } from '@/lib/db/repos/groups';
import { createUser, findUserByEmail, getUser } from '@/lib/db/repos/users';
import { listAudit } from '@/lib/db/repos/audit';
import { parseFilter } from '@/lib/scim';
import { json, makeUser, req } from './helpers';
import * as scim from '@/app/api/scim/v2/[...path]/route';
import * as tokens from '@/app/api/scim-tokens/route';
import * as tokenById from '@/app/api/scim-tokens/[id]/route';

const PATCH_OP = 'urn:ietf:params:scim:api:messages:2.0:PatchOp';

function call(method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE', path: string, token: string | null, body?: unknown) {
  const url = new URL(`/api/scim/v2/${path}`, 'http://localhost:3001');
  const r = new NextRequest(url, { method, headers: { 'content-type': 'application/scim+json', ...(token ? { authorization: `Bearer ${token}` } : {}), 'x-forwarded-for': '198.51.100.20' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const segments = url.pathname.replace('/api/scim/v2/', '').split('/');
  return scim[method](r, { params: Promise.resolve({ path: segments }) });
}

async function body<T = Record<string, unknown>>(p: Promise<Response>): Promise<{ status: number; body: T; type: string | null }> {
  const r = await p;
  const text = await r.text();
  return { status: r.status, body: (text ? JSON.parse(text) : null) as T, type: r.headers.get('content-type') };
}

async function newToken(orgId: string, role = 'user') {
  const admin = makeUser(`scim-admin-${Math.random().toString(36).slice(2)}@x.org`, 'org_admin', orgId);
  const r = await json<{ secret: string; token: { id: string } }>(await tokens.POST(await req('POST', '/api/scim-tokens', { as: admin, body: { name: 'Entra ID', defaultRole: role } })));
  expect(r.status).toBe(201);
  return { admin, secret: r.body.secret, id: r.body.token.id };
}

describe('SCIM', () => {
  it('parses eq filters joined by and', () => {
    expect(parseFilter('userName eq "a@b.c"')).toEqual([{ attr: 'username', value: 'a@b.c' }]);
    expect(parseFilter('externalId eq "x\\"y" and active eq "true"')).toEqual([
      { attr: 'externalid', value: 'x"y' },
      { attr: 'active', value: 'true' },
    ]);
    expect(() => parseFilter('userName sw "a"')).toThrow(/Unsupported filter/);
  });

  it('authenticates per organization and manages tokens', async () => {
    const org = createOrg({ name: 'SCIM auth' });
    expect((await body(call('GET', 'Users', null))).status).toBe(401);
    expect((await body(call('GET', 'Users', 'ufop_scim_nope'))).status).toBe(401);
    const { admin, secret, id } = await newToken(org.id);
    const viewer = makeUser('scim-viewer@x.org', 'read_only_auditor', org.id);
    expect((await tokens.GET(await req('GET', '/api/scim-tokens', { as: viewer }))).status).toBe(403);
    expect((await tokens.POST(await req('POST', '/api/scim-tokens', { as: admin, body: { name: 'x', defaultRole: 'org_admin' } }))).status).toBe(400);
    const listed = await (await tokens.GET(await req('GET', '/api/scim-tokens', { as: admin }))).text();
    expect(listed).not.toContain(secret);
    expect(JSON.parse(listed).baseUrl).toBe('http://localhost:3001/api/scim/v2');

    const spc = await body(call('GET', 'ServiceProviderConfig', secret));
    expect(spc.status).toBe(200);
    expect(spc.type).toBe('application/scim+json');
    expect(spc.body).toMatchObject({ patch: { supported: true }, filter: { supported: true } });
    expect((await body<{ totalResults: number }>(call('GET', 'ResourceTypes', secret))).body.totalResults).toBe(2);
    expect((await body(call('GET', 'Schemas', secret))).status).toBe(200);
    expect((await body(call('GET', 'Nope', secret))).status).toBe(404);

    expect((await tokenById.DELETE(await req('DELETE', `/api/scim-tokens/${id}`, { as: admin }), { params: Promise.resolve({ id }) })).status).toBe(200);
    const after = await body<{ schemas: string[]; status: string; detail: string }>(call('GET', 'Users', secret));
    expect(after.status).toBe(401);
    expect(after.body.schemas).toEqual(['urn:ietf:params:scim:api:messages:2.0:Error']);
  });

  it('provisions, looks up, updates and deactivates users (Entra ID shapes)', async () => {
    const org = createOrg({ name: 'SCIM users' });
    const { secret } = await newToken(org.id);
    // Entra first checks whether the user exists.
    const none = await body<{ totalResults: number; Resources: unknown[] }>(call('GET', 'Users?filter=' + encodeURIComponent('userName eq "ana@corp.example"'), secret));
    expect(none.body).toMatchObject({ totalResults: 0, Resources: [] });

    const created = await body<{ id: string; userName: string; active: boolean; externalId: string; meta: { location: string } }>(
      call('POST', 'Users', secret, {
        schemas: ['urn:ietf:params:scim:schemas:core:2.0:User', 'urn:ietf:params:scim:schemas:extension:enterprise:2.0:User'],
        externalId: 'e-ana',
        userName: 'Ana@Corp.Example',
        active: true,
        displayName: 'Ana Lima',
        emails: [{ primary: true, type: 'work', value: 'ana@corp.example' }],
        name: { formatted: 'Ana Lima', familyName: 'Lima', givenName: 'Ana' },
      }),
    );
    expect(created.status).toBe(201);
    expect(created.body).toMatchObject({ userName: 'ana@corp.example', active: true, externalId: 'e-ana' });
    const id = created.body.id;
    expect(created.body.meta.location).toBe(`http://localhost:3001/api/scim/v2/Users/${id}`);
    expect(effectiveRole(id, org.id)).toBe('user');
    expect(getUser(id)!.ssoProvider).toBe('scim');

    expect((await body(call('POST', 'Users', secret, { userName: 'ana@corp.example' }))).status).toBe(409);
    expect((await body(call('POST', 'Users', secret, { userName: 'not-an-email' }))).status).toBe(400);
    const byExt = await body<{ totalResults: number }>(call('GET', 'Users?filter=' + encodeURIComponent('externalId eq "e-ana"'), secret));
    expect(byExt.body.totalResults).toBe(1);

    // Entra PATCH: string booleans, paths, and a path-less replace.
    const p1 = await body<{ displayName: string; active: boolean }>(
      call('PATCH', `Users/${id}`, secret, {
        schemas: [PATCH_OP],
        Operations: [
          { op: 'Replace', path: 'displayName', value: 'Ana M. Lima' },
          { op: 'Add', path: 'urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department', value: 'Ops' },
          { op: 'Replace', path: 'active', value: 'False' },
        ],
      }),
    );
    expect(p1.status).toBe(200);
    expect(p1.body).toMatchObject({ displayName: 'Ana M. Lima', active: false });
    expect(effectiveRole(id, org.id)).toBeNull();
    expect(getUser(id)!.isActive).toBe(false); // no other organization
    // Still visible to SCIM while deactivated, so it can be reactivated.
    expect((await body<{ totalResults: number }>(call('GET', 'Users?filter=' + encodeURIComponent('active eq "false"'), secret))).body.totalResults).toBe(1);
    const p2 = await body<{ active: boolean; userName: string }>(call('PATCH', `Users/${id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'replace', value: { active: true, userName: 'ana.lima@corp.example' } }] }));
    expect(p2.body).toMatchObject({ active: true, userName: 'ana.lima@corp.example' });
    expect(getUser(id)!.isActive).toBe(true);
    expect(effectiveRole(id, org.id)).toBe('user');
    expect((await body(call('PATCH', `Users/${id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'move', path: 'x' }] }))).status).toBe(400);
    expect((await body(call('PATCH', `Users/${id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'replace', path: 'password', value: 'x' }] }))).status).toBe(400);

    // PUT replaces; DELETE deprovisions.
    const put = await body<{ displayName: string; externalId?: string }>(call('PUT', `Users/${id}`, secret, { userName: 'ana.lima@corp.example', name: { givenName: 'Ana', familyName: 'Souza' }, active: true }));
    expect(put.body.displayName).toBe('Ana Souza');
    expect(put.body.externalId).toBeUndefined();
    expect((await call('DELETE', `Users/${id}`, secret)).status).toBe(204);
    expect(effectiveRole(id, org.id)).toBeNull();
    expect((await body(call('GET', `Users/${id}`, secret))).status).toBe(404);
    expect(listAudit({}).entries.map((e) => e.eventType)).toEqual(expect.arrayContaining(['scim.user_create', 'scim.user_update', 'scim.user_delete', 'scim.token_create']));
    expect(listAudit({ eventType: 'scim.user_create' }).entries[0].userEmail).toBe('scim:Entra ID');
  });

  it('links accounts from other organizations and never touches administrators', async () => {
    const a = createOrg({ name: 'SCIM A' });
    const b = createOrg({ name: 'SCIM B' });
    const { secret } = await newToken(a.id);
    const shared = createUser({ email: 'shared@corp.example', name: 'Shared', role: 'user', organizationId: b.id });
    setMembership(shared.id, b.id, 'operator');
    const linked = await body<{ id: string }>(call('POST', 'Users', secret, { userName: 'shared@corp.example' }));
    expect(linked.status).toBe(201);
    expect(linked.body.id).toBe(shared.id);
    await call('PATCH', `Users/${shared.id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'replace', path: 'active', value: false }] });
    expect(getUser(shared.id)!.isActive).toBe(true); // still active in B
    expect(membershipsOf(shared.id).map((m) => m.orgId)).toEqual([b.id]);

    const boss = makeUser('boss@corp.example', 'org_admin', a.id);
    const bossView = await body<{ active: boolean }>(call('GET', `Users/${boss.id}`, secret));
    expect(bossView.body.active).toBe(true);
    expect((await body(call('PATCH', `Users/${boss.id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'replace', path: 'active', value: false }] }))).status).toBe(403);
    expect((await call('DELETE', `Users/${boss.id}`, secret)).status).toBe(403);
    expect(effectiveRole(boss.id, a.id)).toBe('org_admin');
    // Organization B's users are invisible to A's token.
    const other = makeUser('only-b@corp.example', 'user', b.id);
    expect((await body(call('GET', `Users/${other.id}`, secret))).status).toBe(404);
  });

  it('manages its own groups and members (Okta and Entra shapes), not console groups', async () => {
    const org = createOrg({ name: 'SCIM groups' });
    const { secret } = await newToken(org.id);
    const mk = async (email: string) => (await body<{ id: string }>(call('POST', 'Users', secret, { userName: email }))).body.id;
    const u1 = await mk('g1@corp.example');
    const u2 = await mk('g2@corp.example');
    const stranger = makeUser('stranger@x.org', 'user');

    const g = await body<{ id: string; displayName: string; members: { value: string }[] }>(call('POST', 'Groups', secret, { schemas: ['urn:ietf:params:scim:schemas:core:2.0:Group'], displayName: 'Backup Operators', externalId: 'okta-g1', members: [{ value: u1 }] }));
    expect(g.status).toBe(201);
    expect(g.body.members.map((m) => m.value)).toEqual([u1]);
    const gid = g.body.id;
    expect((await body(call('POST', 'Groups', secret, { displayName: 'Dup', externalId: 'okta-g1' }))).status).toBe(409);
    expect((await body(call('POST', 'Groups', secret, { displayName: 'Bad', members: [{ value: stranger.id }] }))).status).toBe(400);

    // Entra: add, then remove by filter path.
    await call('PATCH', `Groups/${gid}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'Add', path: 'members', value: [{ value: u2 }] }] });
    expect(groupMembers(gid).map((m) => m.userId).sort()).toEqual([u1, u2].sort());
    await call('PATCH', `Groups/${gid}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'Remove', path: `members[value eq "${u1}"]` }] });
    expect(groupMembers(gid).map((m) => m.userId)).toEqual([u2]);
    // Okta: path-less replace of displayName; remove with a value list.
    await call('PATCH', `Groups/${gid}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'replace', value: { id: gid, displayName: 'Backup Ops' } }] });
    await call('PATCH', `Groups/${gid}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'remove', path: 'members', value: [{ value: u2 }] }] });
    const now = await body<{ displayName: string; members: unknown[] }>(call('GET', `Groups/${gid}`, secret));
    expect(now.body).toMatchObject({ displayName: 'Backup Ops', members: [] });

    // PUT replaces members; list with filter, paging and excludedAttributes.
    await call('PUT', `Groups/${gid}`, secret, { displayName: 'Backup Ops', members: [{ value: u1 }, { value: u2 }] });
    const lst = await body<{ totalResults: number; itemsPerPage: number; Resources: { members?: unknown[] }[] }>(call('GET', 'Groups?filter=' + encodeURIComponent('displayName eq "backup ops"') + '&excludedAttributes=members', secret));
    expect(lst.body.totalResults).toBe(1);
    expect(lst.body.Resources[0].members).toBeUndefined();
    const paged = await body<{ totalResults: number; itemsPerPage: number; startIndex: number }>(call('GET', 'Users?startIndex=2&count=1', secret));
    expect(paged.body).toMatchObject({ startIndex: 2, itemsPerPage: 1 });
    expect(paged.body.totalResults).toBeGreaterThanOrEqual(3);
    const u1View = await body<{ groups: { value: string }[] }>(call('GET', `Users/${u1}`, secret));
    expect(u1View.body.groups.map((x) => x.value)).toEqual([gid]);

    // Console groups (possibly granting a role) are read-only for SCIM.
    const consoleGroup = createGroup(org.id, { name: 'Admins', orgRole: 'org_admin' });
    expect((await body(call('PATCH', `Groups/${consoleGroup.id}`, secret, { schemas: [PATCH_OP], Operations: [{ op: 'add', path: 'members', value: [{ value: u1 }] }] }))).status).toBe(403);
    expect(effectiveRole(u1, org.id)).toBe('user');
    updateGroup(gid, { orgRole: 'operator' }); // an admin gave the SCIM group a role: SCIM may no longer change it
    expect((await call('DELETE', `Groups/${gid}`, secret)).status).toBe(403);
    updateGroup(gid, { orgRole: null });
    expect((await call('DELETE', `Groups/${gid}`, secret)).status).toBe(204);
    expect((await body(call('GET', `Groups/${gid}`, secret))).status).toBe(404);
    expect(findUserByEmail('g1@corp.example')).not.toBeNull();
  });
});
