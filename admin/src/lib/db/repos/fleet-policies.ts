/**
 * Fleet backup policies and central credentials.
 *
 * A policy is a backup job template targeted at machines (by tag or id).
 * Agents fetch the policies that apply to them and create/update a managed
 * job per policy (`policy-<id>`), tracking the version they applied.
 *
 * Inheritance: a policy marked `inherit` also applies to machines of child
 * organizations (not across a detached organization). A child organization
 * can opt out of an inherited policy or replace it with its own policy of the
 * same name — unless the parent `locked` it, in which case the inherited
 * policy always applies and same-named child policies are refused.
 *
 * Central credentials are sealed at rest and delivered only to machines a
 * policy that references them applies to. The agent stores each one in its
 * secure store under the credential id `central:<id>`, which templates use as
 * a destination `credential_id` or as `passphraseCredentialId`.
 */
import { db, newId, now, parseJson, tx, type Row } from '../index';
import { open, seal } from '../../crypto';
import { ancestors, getOrg, OrgError, type Organization } from './orgs';
import { normalizeTags, type Machine } from './machines';
import { managedSettingsForAgent } from './managed-settings';

export interface PolicyTarget {
  tags: string[];
  machineIds: string[];
  /** Apply to every machine of the organization. */
  all: boolean;
}

export interface FleetPolicy {
  id: string;
  orgId: string;
  name: string;
  description: string;
  target: PolicyTarget;
  job: Record<string, unknown>;
  credentialIds: string[];
  enabled: boolean;
  /** Also applies to machines of child organizations. */
  inherit: boolean;
  /** With inherit: child organizations cannot opt out or replace it. */
  locked: boolean;
  version: number;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
}

function fromRow(r: Row): FleetPolicy {
  const t = parseJson<Partial<PolicyTarget>>(r.target, {});
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    description: r.description as string,
    target: { tags: t.tags ?? [], machineIds: t.machineIds ?? [], all: !!t.all },
    job: parseJson(r.job, {}),
    credentialIds: parseJson<string[]>(r.credential_ids, []),
    enabled: Boolean(r.enabled),
    inherit: Boolean(r.inherit),
    locked: Boolean(r.inherit) && Boolean(r.locked),
    version: Number(r.version),
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    updatedAt: r.updated_at as string,
  };
}

/** Fields a template may set; identity, stats and secrets are controlled by the agent. */
const JOB_FIELDS = ['name', 'description', 'sources', 'filter', 'destination', 'encrypted', 'options', 'retention', 'schedule', 'scripts', 'reports', 'throttle', 'retry', 'soft_delete_prefix', 'network_timeout_secs', 'local_transfer', 'replicas', 'notifications', 'log_outputs', 'secret_providers', 'prevent_sleep', 'notify_on', 'passphraseCredentialId'];

function validateJob(job: Record<string, unknown>, orgId: string, credentialIds: string[]) {
  if (!job || typeof job !== 'object' || Array.isArray(job)) throw new OrgError('The job template must be an object');
  for (const k of Object.keys(job)) if (!JOB_FIELDS.includes(k)) throw new OrgError(`Unsupported job field "${k}"`);
  if (typeof job.name !== 'string' || !job.name.trim()) throw new OrgError('The job template needs a name');
  if (!Array.isArray(job.sources) || job.sources.length === 0) throw new OrgError('The job template needs at least one source');
  if (!job.destination || typeof job.destination !== 'object') throw new OrgError('The job template needs a destination');
  if (job.encrypted !== false && !job.passphraseCredentialId) throw new OrgError('Encrypted policies need a central credential for the passphrase (or turn encryption off)');
  const referenced = new Set<string>();
  const walk = (v: unknown) => {
    if (typeof v === 'string' && v.startsWith('central:')) referenced.add(v.slice(8));
    else if (Array.isArray(v)) v.forEach(walk);
    else if (v && typeof v === 'object') Object.values(v).forEach(walk);
  };
  walk(job);
  if (typeof job.passphraseCredentialId === 'string') referenced.add(job.passphraseCredentialId.replace(/^central:/, ''));
  for (const id of referenced) {
    if (!credentialIds.includes(id)) credentialIds.push(id);
    if (!db().prepare('SELECT 1 FROM central_credentials WHERE org_id = ? AND id = ?').get(orgId, id)) throw new OrgError(`Credential ${id} does not exist in this organization`);
  }
}

export interface PolicyInput {
  name: string;
  description?: string;
  target: Partial<PolicyTarget>;
  job: Record<string, unknown>;
  enabled?: boolean;
  inherit?: boolean;
  locked?: boolean;
}

/** A locked inherited policy owns its name in this organization. */
function assertNameFree(orgId: string, name: string) {
  const lock = inheritedPolicies(orgId).find((i) => i.policy.locked && i.policy.name.toLowerCase() === name.trim().toLowerCase());
  if (lock) throw new OrgError(`"${lock.policy.name}" is a locked policy inherited from ${lock.sourceOrgName}; it cannot be replaced here`, 409);
}

export function createFleetPolicy(orgId: string, p: PolicyInput, by: string): FleetPolicy {
  if (!p.name?.trim()) throw new OrgError('Give the policy a name');
  assertNameFree(orgId, p.name);
  const creds: string[] = [];
  validateJob(p.job, orgId, creds);
  const id = newId('fpl');
  const ts = now();
  const target: PolicyTarget = { tags: normalizeTags(p.target?.tags ?? []), machineIds: (p.target?.machineIds ?? []).map(String), all: !!p.target?.all };
  db()
    .prepare('INSERT INTO fleet_policies (id, org_id, name, description, target, job, credential_ids, enabled, inherit, locked, version, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)')
    .run(id, orgId, p.name.trim(), p.description ?? '', JSON.stringify(target), JSON.stringify(p.job), JSON.stringify(creds), p.enabled === false ? 0 : 1, p.inherit ? 1 : 0, p.inherit && p.locked ? 1 : 0, by, ts, ts);
  return getFleetPolicy(orgId, id)!;
}

export function updateFleetPolicy(orgId: string, id: string, p: Partial<PolicyInput>): FleetPolicy | null {
  const cur = getFleetPolicy(orgId, id);
  if (!cur) return null;
  const job = p.job ?? cur.job;
  const creds: string[] = [];
  validateJob(job, orgId, creds);
  const target: PolicyTarget = p.target ? { tags: normalizeTags(p.target.tags ?? []), machineIds: (p.target.machineIds ?? []).map(String), all: !!p.target.all } : cur.target;
  if (p.name?.trim() && p.name.trim().toLowerCase() !== cur.name.toLowerCase()) assertNameFree(orgId, p.name);
  const inherit = p.inherit ?? cur.inherit;
  const locked = inherit && (p.locked ?? cur.locked);
  db()
    .prepare('UPDATE fleet_policies SET name = ?, description = ?, target = ?, job = ?, credential_ids = ?, enabled = ?, inherit = ?, locked = ?, version = version + 1, updated_at = ? WHERE org_id = ? AND id = ?')
    .run(p.name?.trim() || cur.name, p.description ?? cur.description, JSON.stringify(target), JSON.stringify(job), JSON.stringify(creds), (p.enabled ?? cur.enabled) ? 1 : 0, inherit ? 1 : 0, locked ? 1 : 0, now(), orgId, id);
  return getFleetPolicy(orgId, id);
}

export function getFleetPolicy(orgId: string, id: string): FleetPolicy | null {
  const r = db().prepare('SELECT * FROM fleet_policies WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function listFleetPolicies(orgId: string): FleetPolicy[] {
  return (db().prepare('SELECT * FROM fleet_policies WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

export function deleteFleetPolicy(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM fleet_policies WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

/** The policy's target matches the machine (organization rules aside). */
function targets(p: FleetPolicy, m: Machine): boolean {
  return p.enabled && (p.target.all || p.target.machineIds.includes(m.id) || (p.target.tags.length > 0 && p.target.tags.every((t) => m.tags.includes(t))));
}

/** The organization and the ancestors whose inherited policies reach it (stopping at a detached organization), closest first. */
function inheritanceChain(orgId: string): Organization[] {
  const org = getOrg(orgId);
  if (!org) return [];
  const chain = [org];
  let below = org;
  for (const a of ancestors(orgId)) {
    if (below.detached) break;
    chain.push(a);
    below = a;
  }
  return chain;
}

export interface InheritedPolicy {
  policy: FleetPolicy;
  sourceOrgId: string;
  sourceOrgName: string;
  /** Opted out here or by an organization in between (ignored when locked). */
  optedOut: boolean;
  /** This organization's own opt-out (what the opt-out switch shows). */
  optedOutHere: boolean;
}

/** Policies that reach `orgId` from its ancestors (closest ancestor first). */
export function inheritedPolicies(orgId: string): InheritedPolicy[] {
  const chain = inheritanceChain(orgId);
  const out: InheritedPolicy[] = [];
  for (let depth = 1; depth < chain.length; depth++) {
    const src = chain[depth];
    const between = chain.slice(0, depth).map((o) => o.id);
    for (const p of listFleetPolicies(src.id).filter((x) => x.inherit)) {
      const outs = new Set((db().prepare(`SELECT org_id FROM policy_opt_outs WHERE policy_id = ? AND org_id IN (${between.map(() => '?').join(',')})`).all(p.id, ...between) as Row[]).map((r) => r.org_id as string));
      out.push({ policy: p, sourceOrgId: src.id, sourceOrgName: src.name, optedOut: outs.size > 0, optedOutHere: outs.has(orgId) });
    }
  }
  return out;
}

/** Opt a child organization out of (or back into) an inherited policy. */
export function setPolicyOptOut(orgId: string, policyId: string, optOut: boolean, by: string): InheritedPolicy {
  const i = inheritedPolicies(orgId).find((x) => x.policy.id === policyId);
  if (!i) throw new OrgError('Not a policy inherited by this organization', 404);
  if (optOut && i.policy.locked) throw new OrgError(`"${i.policy.name}" is locked by ${i.sourceOrgName}`, 409);
  if (optOut) db().prepare('INSERT OR IGNORE INTO policy_opt_outs (org_id, policy_id, opted_out_by, created_at) VALUES (?, ?, ?, ?)').run(orgId, policyId, by, now());
  else db().prepare('DELETE FROM policy_opt_outs WHERE org_id = ? AND policy_id = ?').run(orgId, policyId);
  return inheritedPolicies(orgId).find((x) => x.policy.id === policyId)!;
}

/**
 * The policies a machine runs: its organization's own, plus inherited ones
 * not opted out. Per name, a locked inherited policy wins (the top-most lock);
 * otherwise the closest organization's policy replaces inherited ones.
 */
export function policiesFor(m: Machine): FleetPolicy[] {
  const own = listFleetPolicies(m.orgId).map((p) => ({ p, depth: 0, locked: false }));
  const inherited = inheritedPolicies(m.orgId)
    .filter((i) => i.policy.locked || !i.optedOut)
    .map((i) => ({ p: i.policy, depth: inheritanceChain(m.orgId).findIndex((o) => o.id === i.sourceOrgId), locked: i.policy.locked }));
  const byName = new Map<string, { p: FleetPolicy; depth: number; locked: boolean }[]>();
  for (const c of [...own, ...inherited]) {
    if (!targets(c.p, m)) continue;
    const k = c.p.name.toLowerCase();
    byName.set(k, [...(byName.get(k) ?? []), c]);
  }
  const out: FleetPolicy[] = [];
  for (const group of byName.values()) {
    const locks = group.filter((c) => c.locked);
    const pick = locks.length ? locks.reduce((a, b) => (b.depth > a.depth ? b : a)) : group.reduce((a, b) => (b.depth < a.depth ? b : a));
    // Same-named policies of one organization all run (names are not unique within an organization).
    out.push(...group.filter((c) => c.depth === pick.depth && c.locked === pick.locked).map((c) => c.p));
  }
  return out.sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

export function policyApplies(p: FleetPolicy, m: Machine): boolean {
  return policiesFor(m).some((x) => x.id === p.id);
}

/** Policy set version the agent compares against (changes when any applicable policy changes). */
export function policySetVersion(m: Machine): string {
  const policies = policiesFor(m);
  const parts = policies.map((p) => `${p.id}:${p.version}`);
  for (const c of credentialVersionsFor(policies)) parts.push(`cred:${c.id}:${c.version}`);
  const settings = managedSettingsForAgent(m.orgId);
  if (settings) parts.push(`settings:${settings.version}`);
  return parts.sort().join(',');
}

// ── Central credentials ──

export interface CentralCredential {
  id: string;
  orgId: string;
  name: string;
  kind: string;
  description: string;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
  /** Increases with every rotation; agents report the version they hold. */
  version: number;
}

/** One rotation (no secret material is kept for old versions). */
export interface CredentialVersion {
  version: number;
  createdBy: string;
  createdAt: string;
}

/** Key under which agents report a credential's version among applied policies. */
export const credentialKey = (id: string) => `credential:${id}`;

function credFromRow(r: Row): CentralCredential {
  return { id: r.id as string, orgId: r.org_id as string, name: r.name as string, kind: r.kind as string, description: r.description as string, createdBy: r.created_by as string, createdAt: r.created_at as string, updatedAt: r.updated_at as string, version: Number(r.version ?? 1) };
}

export function credentialHistory(orgId: string, id: string): CredentialVersion[] {
  return (
    db()
      .prepare('SELECT v.version, v.created_by, v.created_at FROM credential_versions v JOIN central_credentials c ON c.id = v.credential_id WHERE c.org_id = ? AND c.id = ? ORDER BY v.version DESC')
      .all(orgId, id) as Row[]
  ).map((r) => ({ version: Number(r.version), createdBy: r.created_by as string, createdAt: r.created_at as string }));
}

export function listCredentials(orgId: string): CentralCredential[] {
  return (db().prepare('SELECT * FROM central_credentials WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(credFromRow);
}

export function createCredential(orgId: string, c: { name: string; kind?: string; description?: string; secret: string }, by: string): CentralCredential {
  if (!c.name?.trim()) throw new OrgError('Give the credential a name');
  if (!c.secret) throw new OrgError('Enter the secret value');
  if (db().prepare('SELECT 1 FROM central_credentials WHERE org_id = ? AND lower(name) = lower(?)').get(orgId, c.name.trim())) throw new OrgError('A credential with this name exists', 409);
  const id = newId('crd');
  const ts = now();
  db().prepare('INSERT INTO central_credentials (id, org_id, name, kind, description, ciphertext, created_by, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)').run(id, orgId, c.name.trim(), c.kind ?? 'secret', c.description ?? '', seal(c.secret), by, ts, ts);
  db().prepare('INSERT INTO credential_versions (credential_id, version, created_by, created_at) VALUES (?, 1, ?, ?)').run(id, by, ts);
  return credFromRow(db().prepare('SELECT * FROM central_credentials WHERE id = ?').get(id) as Row);
}

/**
 * Replace the secret (rotation). The credential's version goes up; it is
 * part of the policy set version of every machine that uses it, so their
 * agents fetch the new secret on the next heartbeat (policies themselves
 * are not re-versioned, so jobs are not rewritten).
 */
export function rotateCredential(orgId: string, id: string, secret: string, by = ''): CentralCredential | null {
  if (!secret) throw new OrgError('Enter the new secret value');
  return tx(() => {
    const ts = now();
    const changed = db().prepare('UPDATE central_credentials SET ciphertext = ?, updated_at = ?, version = version + 1 WHERE org_id = ? AND id = ?').run(seal(secret), ts, orgId, id).changes > 0;
    if (!changed) return null;
    const c = credFromRow(db().prepare('SELECT * FROM central_credentials WHERE id = ?').get(id) as Row);
    db().prepare('INSERT INTO credential_versions (credential_id, version, created_by, created_at) VALUES (?, ?, ?, ?)').run(id, c.version, by, ts);
    return c;
  });
}

export function deleteCredential(orgId: string, id: string): boolean {
  const used = listFleetPolicies(orgId).filter((p) => p.credentialIds.includes(id));
  if (used.length) throw new OrgError(`Used by policy "${used[0].name}"`, 409);
  return db().prepare('DELETE FROM central_credentials WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

/** Credential rows referenced by these policies, each from its policy's own organization (inherited policies use their parent's). */
function credentialRows(policies: FleetPolicy[]): Row[] {
  const pairs = new Map<string, string>();
  for (const p of policies) for (const id of p.credentialIds) pairs.set(id, p.orgId);
  if (!pairs.size) return [];
  const ids = [...pairs.keys()];
  return (db().prepare(`SELECT * FROM central_credentials WHERE id IN (${ids.map(() => '?').join(',')})`).all(...ids) as Row[]).filter((r) => pairs.get(r.id as string) === r.org_id);
}

/** Decrypted credentials referenced by these policies (agent delivery only). */
export function credentialsForPolicies(policies: FleetPolicy[]): { id: string; credentialId: string; name: string; secret: string; updatedAt: string; version: number }[] {
  return credentialRows(policies).map((r) => ({ id: r.id as string, credentialId: `central:${r.id}`, name: r.name as string, secret: open(r.ciphertext as string), updatedAt: r.updated_at as string, version: Number(r.version ?? 1) }));
}

/** Versions of the credentials a machine's policies use (no secrets). */
function credentialVersionsFor(policies: FleetPolicy[]): { id: string; version: number }[] {
  return credentialRows(policies).map((r) => ({ id: r.id as string, version: Number(r.version ?? 1) }));
}

/** Per credential: machines whose policies use it, and which hold the current version. */
export function credentialPropagation(orgId: string, machines: Machine[]): Record<string, { targeted: number; upToDate: number; pending: { id: string; name: string; version: number | null }[] }> {
  const out: Record<string, { targeted: number; upToDate: number; pending: { id: string; name: string; version: number | null }[] }> = {};
  const creds = new Map(listCredentials(orgId).map((c) => [c.id, c.version]));
  for (const m of machines) {
    for (const id of new Set(policiesFor(m).flatMap((p) => p.credentialIds))) {
      const want = creds.get(id);
      if (want === undefined) continue;
      const o = (out[id] ??= { targeted: 0, upToDate: 0, pending: [] });
      o.targeted += 1;
      const have = m.appliedPolicies[credentialKey(id)] ?? null;
      if (have === want) o.upToDate += 1;
      else o.pending.push({ id: m.id, name: m.name, version: have });
    }
  }
  return out;
}
