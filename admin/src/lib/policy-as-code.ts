/**
 * Policy as code: an organization's fleet policies, managed app settings
 * and alert rules as one declarative JSON document, to keep in version
 * control, review, and apply to the same or another organization.
 *
 * Items are matched by name. References are portable: notification
 * channels by name, central credentials as `central-name:<name>` (the
 * secrets themselves never leave the console). `plan` shows what applying
 * would create, update or (with prune) delete; `apply` does it in one
 * transaction, so a document that fails validation changes nothing.
 */
import { tx } from './db';
import { OrgError } from './db/repos/orgs';
import { createFleetPolicy, deleteFleetPolicy, listCredentials, listFleetPolicies, updateFleetPolicy, type PolicyInput } from './db/repos/fleet-policies';
import { getManagedSettings, normalizeManagedSettings, setManagedSettings, type ManagedSettingsBody } from './db/repos/managed-settings';
import { ALERT_KINDS, createRule, deleteRule, listRules, updateRule, type AlertKind, type RuleInput } from './db/repos/alerts';
import { listChannels } from './db/repos/channels';

export const API_VERSION = 'ufop.policy/v1';

export interface PolicyDocument {
  apiVersion: typeof API_VERSION;
  managedSettings: ManagedSettingsBody | null;
  fleetPolicies: (Omit<PolicyInput, 'description'> & { description: string })[];
  alertRules: (Omit<RuleInput, 'channelIds'> & { channels: string[] })[];
}

export type ChangeAction = 'create' | 'update' | 'delete' | 'unchanged';

export interface Change {
  kind: 'managed_settings' | 'fleet_policy' | 'alert_rule';
  name: string;
  action: ChangeAction;
  /** Top-level fields that differ (updates). */
  fields?: string[];
}

export interface Plan {
  changes: Change[];
  /** Counts by action. */
  summary: Record<ChangeAction, number>;
}

const CENTRAL = /^central:(.+)$/;
const CENTRAL_NAME = /^central-name:(.+)$/;

/** Replace every string in `v` through `f` (deep copy). */
function mapStrings(v: unknown, f: (s: string) => string): unknown {
  if (typeof v === 'string') return f(v);
  if (Array.isArray(v)) return v.map((x) => mapStrings(x, f));
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, mapStrings(x, f)]));
  return v;
}

/** Stable JSON for comparisons (sorted keys). */
function canonical(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canonical((v as Record<string, unknown>)[k])}`).join(',')}}`;
  return JSON.stringify(v ?? null);
}

function differing(a: Record<string, unknown>, b: Record<string, unknown>): string[] {
  return [...new Set([...Object.keys(a), ...Object.keys(b)])].filter((k) => canonical(a[k]) !== canonical(b[k])).sort();
}

/** The organization's policies as a document. */
export function exportPolicies(orgId: string): PolicyDocument {
  const credName = new Map(listCredentials(orgId).map((c) => [c.id, c.name]));
  const channelName = new Map(listChannels(orgId).map((c) => [c.id, c.name]));
  const portable = (s: string) => {
    const m = CENTRAL.exec(s);
    return m && credName.has(m[1]) ? `central-name:${credName.get(m[1])}` : s;
  };
  const settings = getManagedSettings(orgId);
  return {
    apiVersion: API_VERSION,
    managedSettings: settings ? { updates: settings.updates, proxy: settings.proxy, ai: settings.ai, transfers: settings.transfers, mounts: settings.mounts, workflows: settings.workflows, backupTemplates: settings.backupTemplates } : null,
    fleetPolicies: listFleetPolicies(orgId).map((p) => {
      const job = mapStrings(p.job, portable) as Record<string, unknown>;
      if (typeof job.passphraseCredentialId === 'string' && credName.has(job.passphraseCredentialId)) job.passphraseCredentialId = `central-name:${credName.get(job.passphraseCredentialId)}`;
      return { name: p.name, description: p.description, target: p.target, job, enabled: p.enabled, inherit: p.inherit, locked: p.locked };
    }),
    alertRules: listRules(orgId).map((r) => ({ name: r.name, kind: r.kind, params: r.params, scope: r.scope, channels: r.channelIds.map((id) => channelName.get(id) ?? id), enabled: r.enabled, cooldownMinutes: r.cooldownMinutes })),
  };
}

interface Resolved {
  settings: ManagedSettingsBody | null;
  policies: PolicyInput[];
  rules: RuleInput[];
}

/** Check the document's shape and resolve its names to this organization's ids. */
function resolve(orgId: string, raw: unknown): Resolved {
  const d = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  if (d.apiVersion !== API_VERSION) throw new OrgError(`apiVersion must be "${API_VERSION}"`);
  const list = (k: string) => {
    const v = d[k] ?? [];
    if (!Array.isArray(v)) throw new OrgError(`${k} must be a list`);
    return v as Record<string, unknown>[];
  };
  const credId = new Map(listCredentials(orgId).map((c) => [c.name.toLowerCase(), c.id]));
  const channelId = new Map(listChannels(orgId).map((c) => [c.name.toLowerCase(), c.id]));
  const local = (s: string) => {
    const m = CENTRAL_NAME.exec(s);
    if (!m) return s;
    const id = credId.get(m[1].toLowerCase());
    if (!id) throw new OrgError(`No central credential named "${m[1]}" in this organization`);
    return `central:${id}`;
  };
  const unique = (items: Record<string, unknown>[], what: string) => {
    const seen = new Set<string>();
    for (const i of items) {
      const n = typeof i.name === 'string' ? i.name.trim().toLowerCase() : '';
      if (!n) throw new OrgError(`Every ${what} needs a name`);
      if (seen.has(n)) throw new OrgError(`Two ${what}s are named "${i.name}"`);
      seen.add(n);
    }
  };
  const policies = list('fleetPolicies');
  const rules = list('alertRules');
  unique(policies, 'fleet policy');
  unique(rules, 'alert rule');
  return {
    settings: d.managedSettings == null ? null : normalizeManagedSettings(d.managedSettings),
    policies: policies.map((p) => {
      const job = mapStrings(p.job, local) as Record<string, unknown>;
      if (typeof job?.passphraseCredentialId === 'string') job.passphraseCredentialId = job.passphraseCredentialId.replace(/^central:/, '');
      return { name: String(p.name).trim(), description: typeof p.description === 'string' ? p.description : '', target: (p.target ?? {}) as PolicyInput['target'], job, enabled: p.enabled !== false, inherit: !!p.inherit, locked: !!p.locked };
    }),
    rules: rules.map((r) => {
      if (!ALERT_KINDS.includes(r.kind as AlertKind)) throw new OrgError(`Alert rule "${r.name}": unknown kind "${String(r.kind)}"`);
      const channels = Array.isArray(r.channels) ? r.channels.map(String) : [];
      const channelIds = channels.map((n) => {
        const id = channelId.get(n.toLowerCase());
        if (!id) throw new OrgError(`Alert rule "${r.name}": no notification channel named "${n}"`);
        return id;
      });
      return { name: String(r.name).trim(), kind: r.kind as AlertKind, params: (r.params ?? {}) as RuleInput['params'], scope: (r.scope ?? {}) as RuleInput['scope'], channelIds, enabled: r.enabled !== false, cooldownMinutes: typeof r.cooldownMinutes === 'number' ? r.cooldownMinutes : 360 };
    }),
  };
}

function compute(orgId: string, want: Resolved, prune: boolean): Plan {
  const changes: Change[] = [];
  const cur = getManagedSettings(orgId);
  const curBody: Record<string, unknown> = cur ? { updates: cur.updates, proxy: cur.proxy, ai: cur.ai, transfers: cur.transfers, mounts: cur.mounts, workflows: cur.workflows, backupTemplates: cur.backupTemplates } : {};
  if (want.settings) {
    const fields = differing(curBody, want.settings as unknown as Record<string, unknown>);
    changes.push({ kind: 'managed_settings', name: 'App settings', action: !cur ? 'create' : fields.length ? 'update' : 'unchanged', ...(cur && fields.length ? { fields } : {}) });
  } else if (cur && prune) {
    changes.push({ kind: 'managed_settings', name: 'App settings', action: 'delete' });
  }
  const existingPolicies = new Map(listFleetPolicies(orgId).map((p) => [p.name.toLowerCase(), p]));
  for (const p of want.policies) {
    const e = existingPolicies.get(p.name.toLowerCase());
    if (!e) {
      changes.push({ kind: 'fleet_policy', name: p.name, action: 'create' });
      continue;
    }
    existingPolicies.delete(p.name.toLowerCase());
    const target = { tags: p.target.tags ?? [], machineIds: p.target.machineIds ?? [], all: !!p.target.all };
    const fields = differing({ description: e.description, target: e.target, job: e.job, enabled: e.enabled, inherit: e.inherit, locked: e.locked }, { description: p.description, target, job: p.job, enabled: p.enabled, inherit: p.inherit, locked: !!p.inherit && !!p.locked });
    changes.push({ kind: 'fleet_policy', name: e.name, action: fields.length ? 'update' : 'unchanged', ...(fields.length ? { fields } : {}) });
  }
  if (prune) for (const e of existingPolicies.values()) changes.push({ kind: 'fleet_policy', name: e.name, action: 'delete' });
  const existingRules = new Map(listRules(orgId).map((r) => [r.name.toLowerCase(), r]));
  for (const r of want.rules) {
    const e = existingRules.get(r.name.toLowerCase());
    if (!e) {
      changes.push({ kind: 'alert_rule', name: r.name, action: 'create' });
      continue;
    }
    existingRules.delete(r.name.toLowerCase());
    const scope = { tags: r.scope?.tags ?? [], machineIds: r.scope?.machineIds ?? [] };
    const fields = differing({ kind: e.kind, params: e.params, scope: e.scope, channelIds: e.channelIds, enabled: e.enabled, cooldownMinutes: e.cooldownMinutes }, { kind: r.kind, params: r.params ?? {}, scope, channelIds: r.channelIds, enabled: r.enabled, cooldownMinutes: r.cooldownMinutes });
    changes.push({ kind: 'alert_rule', name: e.name, action: fields.length ? 'update' : 'unchanged', ...(fields.length ? { fields } : {}) });
  }
  if (prune) for (const e of existingRules.values()) changes.push({ kind: 'alert_rule', name: e.name, action: 'delete' });
  const summary: Record<ChangeAction, number> = { create: 0, update: 0, delete: 0, unchanged: 0 };
  for (const c of changes) summary[c.action]++;
  return { changes, summary };
}

/** What applying `doc` would change (validates names and references; writes nothing). */
export function planPolicies(orgId: string, doc: unknown, prune = false): Plan {
  return compute(orgId, resolve(orgId, doc), prune);
}

/**
 * Apply `doc`: create and update items to match (and with `prune`, delete
 * the ones it doesn't list). All or nothing.
 */
export function applyPolicies(orgId: string, doc: unknown, by: string, prune = false): Plan {
  const want = resolve(orgId, doc);
  return tx(() => {
    const plan = compute(orgId, want, prune);
    const policies = new Map(listFleetPolicies(orgId).map((p) => [p.name.toLowerCase(), p]));
    const rules = new Map(listRules(orgId).map((r) => [r.name.toLowerCase(), r]));
    for (const c of plan.changes) {
      if (c.action === 'unchanged') continue;
      if (c.kind === 'managed_settings') {
        setManagedSettings(orgId, c.action === 'delete' ? {} : want.settings, by);
      } else if (c.kind === 'fleet_policy') {
        const existing = policies.get(c.name.toLowerCase());
        const input = want.policies.find((p) => p.name.toLowerCase() === c.name.toLowerCase());
        if (c.action === 'delete' && existing) deleteFleetPolicy(orgId, existing.id);
        else if (c.action === 'create' && input) createFleetPolicy(orgId, input, by);
        else if (c.action === 'update' && existing && input) updateFleetPolicy(orgId, existing.id, input);
      } else {
        const existing = rules.get(c.name.toLowerCase());
        const input = want.rules.find((r) => r.name.toLowerCase() === c.name.toLowerCase());
        if (c.action === 'delete' && existing) deleteRule(orgId, existing.id);
        else if (c.action === 'create' && input) createRule(orgId, input);
        else if (c.action === 'update' && existing && input) updateRule(orgId, existing.id, input);
      }
    }
    return plan;
  });
}
