/** Policy as code: export, portable references, plan, all-or-nothing apply, prune, permissions. */
import { describe, expect, it } from 'vitest';
import { createOrg } from '@/lib/db/repos/orgs';
import { createChannel } from '@/lib/db/repos/channels';
import { createCredential, listFleetPolicies } from '@/lib/db/repos/fleet-policies';
import { listRules } from '@/lib/db/repos/alerts';
import { getManagedSettings } from '@/lib/db/repos/managed-settings';
import { listAudit } from '@/lib/db/repos/audit';
import { json, makeUser, req } from './helpers';
import * as pac from '@/app/api/fleet/policy-as-code/route';
import * as plan from '@/app/api/fleet/policy-as-code/plan/route';
import * as fleetPolicies from '@/app/api/fleet/policies/route';
import * as alertRules from '@/app/api/alerts/rules/route';
import * as managedSettings from '@/app/api/fleet/managed-settings/route';

type Doc = { apiVersion: string; managedSettings: Record<string, unknown> | null; fleetPolicies: Record<string, unknown>[]; alertRules: Record<string, unknown>[] };
type PlanBody = { plan: { changes: { kind: string; name: string; action: string; fields?: string[] }[]; summary: Record<string, number> } };

describe('policy as code', () => {
  it('exports portable documents and applies them to another organization', async () => {
    const src = createOrg({ name: 'Source' });
    const dst = createOrg({ name: 'Target' });
    const admin = makeUser('pac-admin@x.org', 'org_admin', src.id);
    const admin2 = makeUser('pac-admin2@x.org', 'org_admin', dst.id);
    const auditor = makeUser('pac-auditor@x.org', 'read_only_auditor', dst.id);
    // Same names, different ids, in both organizations.
    const credA = createCredential(src.id, { name: 'Backup key', secret: 'one' }, 'x');
    const credB = createCredential(dst.id, { name: 'Backup key', secret: 'two' }, 'x');
    const chA = createChannel(src.id, { name: 'Ops mail', kind: 'webhook', config: { url: 'https://hooks.example.com/a' } });
    const chB = createChannel(dst.id, { name: 'Ops mail', kind: 'webhook', config: { url: 'https://hooks.example.com/b' } });

    const job = { name: 'Docs', sources: ['/home'], destination: { type: 's3', bucket: 'b', credential_id: `central:${credA.id}` }, encrypted: true, passphraseCredentialId: `central:${credA.id}`, schedule: { every_minutes: 60 } };
    expect((await fleetPolicies.POST(await req('POST', '/api/fleet/policies', { as: admin, body: { name: 'Docs nightly', target: { tags: ['laptops'] }, job } }))).status).toBe(201);
    expect((await alertRules.POST(await req('POST', '/api/alerts/rules', { as: admin, body: { name: 'Backup failed', kind: 'backup_failed', channelIds: [chA.id] } }))).status).toBe(201);
    expect((await managedSettings.PUT(await req('PUT', '/api/fleet/managed-settings', { as: admin, body: { ai: { allowCloud: false } } }))).status).toBe(200);

    const exported = await json<Doc>(await pac.GET(await req('GET', '/api/fleet/policy-as-code', { as: admin })));
    expect(exported.status).toBe(200);
    const doc = exported.body;
    expect(doc.apiVersion).toBe('ufop.policy/v1');
    const text = JSON.stringify(doc);
    expect(text).not.toContain(credA.id);
    expect(text).not.toContain('one');
    expect(doc.fleetPolicies[0].job).toMatchObject({ passphraseCredentialId: 'central-name:Backup key', destination: { credential_id: 'central-name:Backup key' } });
    expect(doc.alertRules[0]).toMatchObject({ name: 'Backup failed', channels: ['Ops mail'] });

    // Plan in the target: everything is new; nothing is written.
    const planned = await json<PlanBody>(await plan.POST(await req('POST', '/api/fleet/policy-as-code/plan', { as: auditor, body: { document: doc } })));
    expect(planned.status).toBe(200);
    expect(planned.body.plan.summary).toEqual({ create: 3, update: 0, delete: 0, unchanged: 0 });
    expect(listFleetPolicies(dst.id)).toEqual([]);
    expect((await pac.PUT(await req('PUT', '/api/fleet/policy-as-code', { as: auditor, body: { document: doc } }))).status).toBe(403);

    // Apply: references resolve to the target's own credential and channel.
    const applied = await json<PlanBody>(await pac.PUT(await req('PUT', '/api/fleet/policy-as-code', { as: admin2, body: { document: doc } })));
    expect(applied.status).toBe(200);
    const [p] = listFleetPolicies(dst.id);
    expect(p.job).toMatchObject({ passphraseCredentialId: credB.id, destination: { credential_id: `central:${credB.id}` } });
    expect(listRules(dst.id)[0].channelIds).toEqual([chB.id]);
    expect(getManagedSettings(dst.id)?.ai).toEqual({ enabled: null, allowCloud: false });
    expect(listAudit({}).entries.some((e) => e.eventType === 'fleet.policy_as_code' && e.description.includes('3 created'))).toBe(true);

    // Re-applying is a no-op; an edit is one update naming its fields.
    const again = await json<PlanBody>(await plan.POST(await req('POST', '/api/fleet/policy-as-code/plan', { as: admin2, body: { document: doc } })));
    expect(again.body.plan.summary).toEqual({ create: 0, update: 0, delete: 0, unchanged: 3 });
    const edited: Doc = { ...doc, fleetPolicies: [{ ...doc.fleetPolicies[0], enabled: false, target: { tags: ['desktops'], machineIds: [], all: false } }] };
    const upd = await json<PlanBody>(await plan.POST(await req('POST', '/api/fleet/policy-as-code/plan', { as: admin2, body: { document: edited } })));
    expect(upd.body.plan.changes.find((c) => c.kind === 'fleet_policy')).toEqual({ kind: 'fleet_policy', name: 'Docs nightly', action: 'update', fields: ['enabled', 'target'] });

    // Prune removes what the document leaves out.
    const pruneDoc: Doc = { ...doc, alertRules: [], managedSettings: null };
    const pruned = await json<PlanBody>(await pac.PUT(await req('PUT', '/api/fleet/policy-as-code', { as: admin2, body: { document: pruneDoc, prune: true } })));
    expect(pruned.body.plan.summary).toMatchObject({ delete: 2 });
    expect(listRules(dst.id)).toEqual([]);
    expect(getManagedSettings(dst.id)).toBeNull();
  });

  it('rejects bad documents without changing anything', async () => {
    const org = createOrg({ name: 'Strict' });
    const admin = makeUser('pac-strict@x.org', 'org_admin', org.id);
    createChannel(org.id, { name: 'Hook', kind: 'webhook', config: { url: 'https://hooks.example.com/x' } });
    const good = { name: 'Ok rule', kind: 'backup_failed', channels: ['Hook'] };
    const put = async (document: unknown) => pac.PUT(await req('PUT', '/api/fleet/policy-as-code', { as: admin, body: { document } }));
    const base = { apiVersion: 'ufop.policy/v1', managedSettings: null, fleetPolicies: [] };
    for (const bad of [
      { ...base, apiVersion: 'v0', alertRules: [] },
      { ...base, alertRules: [good, { ...good }] },
      { ...base, alertRules: [good, { name: 'X', kind: 'nope' }] },
      { ...base, alertRules: [good, { name: 'Y', kind: 'backup_failed', channels: ['Missing'] }] },
      { ...base, alertRules: [good], fleetPolicies: [{ name: 'P', target: { all: true }, job: { name: 'j', sources: ['/'], destination: { type: 'local' }, passphraseCredentialId: 'central-name:Nope' } }] },
      // Valid names, but the repository rejects the policy itself: the rule before it must not stay.
      { ...base, alertRules: [good], fleetPolicies: [{ name: 'P', target: { all: true }, job: { name: 'j', sources: [], destination: { type: 'local' }, encrypted: false } }] },
      { ...base, alertRules: [good], managedSettings: { ai: { enabled: 'no' } } },
    ]) {
      expect((await put(bad)).status).toBe(400);
    }
    expect(listRules(org.id)).toEqual([]);
    expect(listFleetPolicies(org.id)).toEqual([]);
  });
});
