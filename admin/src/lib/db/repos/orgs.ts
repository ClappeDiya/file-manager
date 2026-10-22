/**
 * Organizations (tenants) form a tree. A user's role in an organization
 * comes from a direct membership, a group that grants an organization role,
 * or — downward delegation — a membership/group in an ancestor, unless an
 * organization on the way is *detached* (a detached subtree is managed only
 * by its own members and does not inherit the parent's license).
 */
import { db, newId, now, tx, type Row } from '../index';
import { ROLE_HIERARCHY, type Role } from '../../types/auth';

export type OrgStatus = 'active' | 'offboarding';

export interface Organization {
  id: string;
  name: string;
  parentId: string | null;
  detached: boolean;
  protected: boolean;
  status: OrgStatus;
  licensePlan: string | null;
  licenseSeats: number | null;
  offboardAfter: string | null;
  createdAt: string;
  updatedAt: string;
}

function fromRow(r: Row): Organization {
  return {
    id: r.id as string,
    name: r.name as string,
    parentId: (r.parent_id as string) ?? null,
    detached: Boolean(r.detached),
    protected: Boolean(r.protected),
    status: r.status as OrgStatus,
    licensePlan: (r.license_plan as string) ?? null,
    licenseSeats: (r.license_seats as number) ?? null,
    offboardAfter: (r.offboard_after as string) ?? null,
    createdAt: r.created_at as string,
    updatedAt: r.updated_at as string,
  };
}

export class OrgError extends Error {
  constructor(message: string, public status = 400) {
    super(message);
  }
}

export function getOrg(id: string): Organization | null {
  const r = db().prepare('SELECT * FROM organizations WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function listOrgs(): Organization[] {
  return (db().prepare('SELECT * FROM organizations ORDER BY lower(name)').all() as Row[]).map(fromRow);
}

export function childrenOf(id: string): Organization[] {
  return (db().prepare('SELECT * FROM organizations WHERE parent_id = ? ORDER BY lower(name)').all(id) as Row[]).map(fromRow);
}

/** Ancestors, nearest first. */
export function ancestors(id: string): Organization[] {
  const out: Organization[] = [];
  let cur = getOrg(id);
  const seen = new Set<string>();
  while (cur?.parentId && !seen.has(cur.parentId)) {
    seen.add(cur.parentId);
    const p = getOrg(cur.parentId);
    if (!p) break;
    out.push(p);
    cur = p;
  }
  return out;
}

/** The organization and every descendant reachable without crossing a detached boundary. */
export function managedSubtree(id: string): string[] {
  const out = [id];
  for (let i = 0; i < out.length; i++) for (const c of childrenOf(out[i])) if (!c.detached) out.push(c.id);
  return out;
}

/** Every descendant regardless of detachment (for offboarding checks). */
export function fullSubtree(id: string): string[] {
  const out = [id];
  for (let i = 0; i < out.length; i++) for (const c of childrenOf(out[i])) out.push(c.id);
  return out;
}

export interface NewOrg {
  name: string;
  parentId?: string | null;
  detached?: boolean;
  licensePlan?: string | null;
  licenseSeats?: number | null;
}

export function createOrg(o: NewOrg): Organization {
  if (!o.name.trim()) throw new OrgError('Give the organization a name');
  if (o.parentId && !getOrg(o.parentId)) throw new OrgError('The parent organization does not exist', 404);
  const id = newId('org');
  const ts = now();
  db()
    .prepare('INSERT INTO organizations (id, name, parent_id, detached, license_plan, license_seats, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, o.name.trim(), o.parentId ?? null, o.detached ? 1 : 0, o.licensePlan ?? null, o.licenseSeats ?? null, ts, ts);
  return getOrg(id)!;
}

export interface OrgUpdate {
  name?: string;
  parentId?: string | null;
  detached?: boolean;
  protected?: boolean;
  licensePlan?: string | null;
  licenseSeats?: number | null;
}

export function updateOrg(id: string, u: OrgUpdate): Organization {
  const org = getOrg(id);
  if (!org) throw new OrgError('Organization not found', 404);
  if (u.parentId !== undefined && u.parentId !== null) {
    if (u.parentId === id || fullSubtree(id).includes(u.parentId)) throw new OrgError('An organization cannot be moved under itself');
    if (!getOrg(u.parentId)) throw new OrgError('The parent organization does not exist', 404);
  }
  const map: [keyof OrgUpdate, string][] = [
    ['name', 'name'],
    ['parentId', 'parent_id'],
    ['detached', 'detached'],
    ['protected', 'protected'],
    ['licensePlan', 'license_plan'],
    ['licenseSeats', 'license_seats'],
  ];
  const sets: string[] = ['updated_at = ?'];
  const vals: unknown[] = [now()];
  for (const [k, col] of map) {
    const v = u[k];
    if (v === undefined) continue;
    sets.push(`${col} = ?`);
    vals.push(typeof v === 'boolean' ? (v ? 1 : 0) : v);
  }
  db().prepare(`UPDATE organizations SET ${sets.join(', ')} WHERE id = ?`).run(...vals, id);
  return getOrg(id)!;
}

// ── Memberships ──

export interface Membership {
  userId: string;
  orgId: string;
  role: Role;
  createdAt: string;
  /** The membership ends then (see `offboarding.expireMemberships`). */
  expiresAt: string | null;
}

const membershipFromRow = (r: Row): Membership => ({ userId: r.user_id as string, orgId: r.org_id as string, role: r.role as Role, createdAt: r.created_at as string, expiresAt: (r.expires_at as string) ?? null });

export function setMembership(userId: string, orgId: string, role: Role): void {
  db()
    .prepare('INSERT INTO org_memberships (user_id, org_id, role, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(user_id, org_id) DO UPDATE SET role = excluded.role')
    .run(userId, orgId, role, now());
}

export function removeMembership(userId: string, orgId: string): boolean {
  return db().prepare('DELETE FROM org_memberships WHERE user_id = ? AND org_id = ?').run(userId, orgId).changes > 0;
}

export function membershipsOf(userId: string): Membership[] {
  return (db().prepare('SELECT * FROM org_memberships WHERE user_id = ?').all(userId) as Row[]).map(membershipFromRow);
}

export function membersOf(orgId: string): Membership[] {
  return (db().prepare('SELECT * FROM org_memberships WHERE org_id = ?').all(orgId) as Row[]).map(membershipFromRow);
}

const rank = (r: Role | null | undefined) => (r ? (ROLE_HIERARCHY[r] ?? 0) : 0);
const maxRole = (a: Role | null, b: Role | null): Role | null => (rank(b) > rank(a) ? b : a);

/** Role grants in one organization: direct membership plus group-granted roles. */
function grantsIn(userId: string, orgId: string, inheritedOnly: boolean): Role | null {
  // Direct memberships always delegate downward; groups only when they opt in.
  const m = db().prepare('SELECT role FROM org_memberships WHERE user_id = ? AND org_id = ?').get(userId, orgId) as Row | undefined;
  let best: Role | null = (m?.role as Role) ?? null;
  const gs = db()
    .prepare(`SELECT g.org_role FROM groups g JOIN group_members m ON m.group_id = g.id WHERE m.user_id = ? AND g.org_id = ? AND g.org_role IS NOT NULL ${inheritedOnly ? 'AND g.delegate_to_children = 1' : ''}`)
    .all(userId, orgId) as Row[];
  for (const g of gs) best = maxRole(best, g.org_role as Role);
  return best;
}

/** The user's effective role in `orgId`, or null without access. */
export function effectiveRole(userId: string, orgId: string): Role | null {
  const org = getOrg(orgId);
  if (!org) return null;
  let best = grantsIn(userId, orgId, false);
  // Walk up while the organization below is not detached.
  let below = org;
  for (const a of ancestors(orgId)) {
    if (below.detached) break;
    best = maxRole(best, grantsIn(userId, a.id, true));
    below = a;
  }
  return best;
}

/** Every organization the user can open, with their role there. */
export function accessibleOrgs(userId: string): { org: Organization; role: Role }[] {
  const out: { org: Organization; role: Role }[] = [];
  for (const o of listOrgs()) {
    const role = effectiveRole(userId, o.id);
    if (role) out.push({ org: o, role });
  }
  return out;
}

// ── Licensing ──

export interface License {
  plan: string | null;
  seats: number | null;
  /** Organization whose license applies (itself or an ancestor). */
  sourceOrgId: string;
  inherited: boolean;
  /** Machines counted against the seats (license root's managed subtree). */
  used: number;
}

export function license(orgId: string): License {
  const org = getOrg(orgId);
  if (!org) throw new OrgError('Organization not found', 404);
  let src = org;
  if (org.licensePlan === null && org.licenseSeats === null && !org.detached) {
    let below = org;
    for (const a of ancestors(orgId)) {
      if (below.detached) break;
      src = a;
      if (a.licensePlan !== null || a.licenseSeats !== null) break;
      below = a;
    }
  }
  const scope = managedSubtree(src.id);
  const used = (db().prepare(`SELECT COUNT(*) n FROM machines WHERE status IN ('active','suspended') AND org_id IN (${scope.map(() => '?').join(',')})`).get(...scope) as { n: number }).n;
  return { plan: src.licensePlan, seats: src.licenseSeats, sourceOrgId: src.id, inherited: src.id !== org.id, used };
}

// ── Offboarding ──

/** Start a protected-delete: revokes machine access now, deletes data after `graceDays`. */
export function startOffboarding(orgId: string, graceDays: number, confirmName: string): Organization {
  const org = getOrg(orgId);
  if (!org) throw new OrgError('Organization not found', 404);
  if (orgId === 'org_default') throw new OrgError('The default organization cannot be offboarded');
  if (org.protected) throw new OrgError('This organization is protected. Remove the protection first.', 409);
  if (confirmName.trim() !== org.name) throw new OrgError('Type the organization name exactly to confirm');
  const subtree = fullSubtree(orgId);
  const protectedChild = subtree.map(getOrg).find((o) => o && o.id !== orgId && o.protected);
  if (protectedChild) throw new OrgError(`Sub-organization "${protectedChild.name}" is protected`, 409);
  const after = new Date(Date.now() + Math.max(0, graceDays) * 86_400_000).toISOString();
  tx(() => {
    const ph = subtree.map(() => '?').join(',');
    db().prepare(`UPDATE organizations SET status = 'offboarding', offboard_after = ?, updated_at = ? WHERE id IN (${ph})`).run(after, now(), ...subtree);
    db().prepare(`UPDATE machines SET status = 'revoked', token_hash = NULL, revoked_at = ? WHERE org_id IN (${ph}) AND status != 'revoked'`).run(now(), ...subtree);
    db().prepare(`UPDATE registration_links SET revoked_at = ? WHERE org_id IN (${ph}) AND revoked_at IS NULL`).run(now(), ...subtree);
  });
  return getOrg(orgId)!;
}

export function cancelOffboarding(orgId: string): Organization {
  const subtree = fullSubtree(orgId);
  db().prepare(`UPDATE organizations SET status = 'active', offboard_after = NULL, updated_at = ? WHERE id IN (${subtree.map(() => '?').join(',')})`).run(now(), ...subtree);
  return getOrg(orgId)!;
}

/** Delete offboarding organizations whose grace period has passed. Returns deleted ids. */
export function purgeOffboarded(at = new Date()): string[] {
  const due = (db().prepare("SELECT id FROM organizations WHERE status = 'offboarding' AND offboard_after <= ?").all(at.toISOString()) as Row[]).map((r) => r.id as string);
  const deleted: string[] = [];
  tx(() => {
    // Deepest first so parent rows go last.
    const order = due.sort((a, b) => ancestors(b).length - ancestors(a).length);
    for (const id of order) {
      db().prepare('UPDATE users SET organization_id = ? WHERE organization_id = ?').run('org_default', id);
      db().prepare('DELETE FROM backup_reports WHERE org_id = ?').run(id);
      db().prepare('DELETE FROM backups WHERE org_id = ?').run(id);
      db().prepare('DELETE FROM alerts WHERE org_id = ?').run(id);
      db().prepare('DELETE FROM agent_commands WHERE org_id = ?').run(id);
      if (db().prepare('DELETE FROM organizations WHERE id = ?').run(id).changes) deleted.push(id);
    }
  });
  return deleted;
}

export function orgPath(orgId: string): string {
  const o = getOrg(orgId);
  if (!o) return '';
  return [...ancestors(orgId).reverse().map((a) => a.name), o.name].join(' / ');
}
