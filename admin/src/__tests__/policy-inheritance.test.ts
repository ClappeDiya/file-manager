/** Fleet policy inheritance: child organizations inherit, opt out, override by name; locks win. */
import { describe, expect, it } from 'vitest';
import { createOrg, updateOrg } from '@/lib/db/repos/orgs';
import { createCredential, createFleetPolicy, policiesFor, policySetVersion, credentialsForPolicies } from '@/lib/db/repos/fleet-policies';
import { getMachine } from '@/lib/db/repos/machines';
import { json, makeUser, req } from './helpers';
import * as links from '@/app/api/fleet/links/route';
import * as register from '@/app/api/agent/register/route';
import * as fleetPolicies from '@/app/api/fleet/policies/route';
import * as optOut from '@/app/api/fleet/policies/[id]/opt-out/route';

type U = ReturnType<typeof makeUser>;

async function enroll(as: U, name: string, tags: string[] = []) {
  const l = await json<{ token: string }>(await links.POST(await req('POST', '/api/fleet/links', { as, body: {} })));
  const r = await json<{ machineId: string }>(
    await register.POST(await req('POST', '/api/agent/register', { body: { registrationToken: l.body.token, info: { name, hostname: name, os: 'linux', clientVersion: '1.0.0' }, tags } })),
  );
  return getMachine(r.body.machineId)!;
}

const plain = (name: string) => ({ name, sources: ['/data'], destination: { type: 'local', path: '/backup' }, encrypted: false });
const names = (m: Parameters<typeof policiesFor>[0]) => policiesFor(m).map((p) => `${p.name}@${p.orgId}`);

describe('fleet policy inheritance', () => {
  it('applies inheriting policies below, honours opt-outs, overrides and locks, and stops at detached organizations', async () => {
    const parent = createOrg({ name: 'HQ' });
    const child = createOrg({ name: 'Branch', parentId: parent.id });
    const grandchild = createOrg({ name: 'Desk', parentId: child.id });
    const island = createOrg({ name: 'Island', parentId: parent.id, detached: true });
    const hq = makeUser('inh-hq@x.org', 'org_admin', parent.id);
    const br = makeUser('inh-br@x.org', 'org_admin', child.id);
    const de = makeUser('inh-de@x.org', 'org_admin', grandchild.id);
    const is = makeUser('inh-is@x.org', 'org_admin', island.id);
    const mHq = await enroll(hq, 'hq-1');
    const mBr = await enroll(br, 'br-1', ['laptop']);
    const mDe = await enroll(de, 'de-1');
    const mIs = await enroll(is, 'is-1');

    const cred = createCredential(parent.id, { name: 'HQ passphrase', secret: 'hq-secret' }, 't');
    createFleetPolicy(parent.id, { name: 'Local only', target: { all: true }, job: plain('local') }, 't');
    const docs = createFleetPolicy(parent.id, { name: 'Docs', target: { all: true }, job: { ...plain('docs'), encrypted: true, passphraseCredentialId: cred.id }, inherit: true }, 't');
    const baseline = createFleetPolicy(parent.id, { name: 'Baseline', target: { all: true }, job: plain('baseline'), inherit: true, locked: true }, 't');
    createFleetPolicy(parent.id, { name: 'Laptops', target: { tags: ['laptop'] }, job: plain('laptops'), inherit: true }, 't');

    expect(names(mHq)).toEqual([`Baseline@${parent.id}`, `Docs@${parent.id}`, `Local only@${parent.id}`]);
    expect(names(mBr)).toEqual([`Baseline@${parent.id}`, `Docs@${parent.id}`, `Laptops@${parent.id}`]);
    expect(names(mDe)).toEqual([`Baseline@${parent.id}`, `Docs@${parent.id}`]);
    expect(names(mIs)).toEqual([]); // detached: nothing from above
    // Inherited policies bring their own organization's credentials.
    expect(credentialsForPolicies(policiesFor(mDe))).toMatchObject([{ id: cred.id, secret: 'hq-secret' }]);
    expect(policySetVersion(mDe)).toContain(`cred:${cred.id}:1`);

    // The branch replaces Docs with its own; the desk below gets the branch's version.
    const own = createFleetPolicy(child.id, { name: 'docs', target: { all: true }, job: plain('branch docs') }, 't');
    expect(names(mBr)).toContain(`docs@${child.id}`);
    expect(names(mBr)).not.toContain(`Docs@${parent.id}`);
    expect(names(mDe)).toEqual([`Baseline@${parent.id}`, `Docs@${parent.id}`]); // the branch policy does not inherit
    // Locked names cannot be reused below.
    expect(() => createFleetPolicy(child.id, { name: 'BASELINE', target: { all: true }, job: plain('x') }, 't')).toThrow(/locked policy inherited from HQ/);

    // Opting out: the branch leaves Laptops; locked Baseline cannot be left.
    const before = policySetVersion(mBr);
    const post = async (as: U, id: string, value: boolean) =>
      json<{ error?: string; inherited?: { optedOutHere: boolean } }>(await optOut.POST(await req('POST', `/api/fleet/policies/${id}/opt-out`, { as, body: { optOut: value } }), { params: Promise.resolve({ id }) }));
    const laptops = policiesFor(mBr).find((p) => p.name === 'Laptops')!;
    expect((await post(br, laptops.id, true)).body.inherited).toMatchObject({ optedOutHere: true });
    expect(names(mBr)).not.toContain(`Laptops@${parent.id}`);
    expect(policySetVersion(mBr)).not.toBe(before); // the agent re-syncs and removes the job
    expect((await post(br, baseline.id, true)).status).toBe(409);
    expect((await post(br, own.id, true)).status).toBe(404); // not inherited
    // The branch's opt-out of Docs reaches the desk below.
    await post(br, docs.id, true);
    expect(names(mDe)).toEqual([`Baseline@${parent.id}`]);
    const deskView = await json<{ inherited: { policy: { name: string }; optedOut: boolean; optedOutHere: boolean }[] }>(await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: de })));
    expect(deskView.body.inherited.find((i) => i.policy.name === 'Docs')).toMatchObject({ optedOut: true, optedOutHere: false });
    await post(br, docs.id, false);
    expect(names(mDe)).toEqual([`Baseline@${parent.id}`, `Docs@${parent.id}`]);

    // Compliance on the parent counts machines below for inheriting policies only.
    const hqView = await json<{ policies: { name: string; targeted: number }[] }>(await fleetPolicies.GET(await req('GET', '/api/fleet/policies', { as: hq })));
    const count = (n: string) => hqView.body.policies.find((p) => p.name === n)!.targeted;
    expect(count('Baseline')).toBe(3);
    expect(count('Local only')).toBe(1);
    expect(count('Docs')).toBe(2); // HQ and the desk; the branch replaced it

    // Detaching the branch cuts everything above, locks included.
    updateOrg(child.id, { detached: true });
    expect(names(mBr)).toEqual([`docs@${child.id}`]);
    expect(names(mDe)).toEqual([]);
  });

  it('accepts inherit/locked through the API and ignores a lock without inheritance', async () => {
    const org = createOrg({ name: 'API org' });
    const admin = makeUser('inh-api@x.org', 'org_admin', org.id);
    const r = await json<{ policy: { inherit: boolean; locked: boolean } }>(
      await fleetPolicies.POST(await req('POST', '/api/fleet/policies', { as: admin, body: { name: 'P', target: { all: true }, job: plain('p'), locked: true } })),
    );
    expect(r.status).toBe(201);
    expect(r.body.policy).toMatchObject({ inherit: false, locked: false });
  });
});
