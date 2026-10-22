/**
 * Groups inside an organization. Owners manage membership and invitations;
 * members only receive what the group grants: an optional organization role
 * (delegated to sub-organizations when `delegateToChildren`) and access to
 * specific machines, backups or machine tags (access scoping).
 */
import { db, newId, now, type Row } from '../index';
import type { Role } from '../../types/auth';
import { OrgError } from './orgs';

export type GroupRole = 'owner' | 'member';
export type ResourceType = 'machine' | 'tag' | 'backup';

export interface Group {
  id: string;
  orgId: string;
  name: string;
  description: string;
  orgRole: Role | null;
  delegateToChildren: boolean;
  createdAt: string;
}

function fromRow(r: Row): Group {
  return { id: r.id as string, orgId: r.org_id as string, name: r.name as string, description: r.description as string, orgRole: (r.org_role as Role) ?? null, delegateToChildren: Boolean(r.delegate_to_children), createdAt: r.created_at as string };
}

export function listGroups(orgId: string): Group[] {
  return (db().prepare('SELECT * FROM groups WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

export function getGroup(id: string): Group | null {
  const r = db().prepare('SELECT * FROM groups WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export interface GroupInput {
  name: string;
  description?: string;
  orgRole?: Role | null;
  delegateToChildren?: boolean;
}

export function createGroup(orgId: string, g: GroupInput): Group {
  if (!g.name?.trim()) throw new OrgError('Give the group a name');
  if (db().prepare('SELECT 1 FROM groups WHERE org_id = ? AND lower(name) = lower(?)').get(orgId, g.name.trim())) throw new OrgError('A group with this name already exists', 409);
  const id = newId('grp');
  db().prepare('INSERT INTO groups (id, org_id, name, description, org_role, delegate_to_children, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)').run(id, orgId, g.name.trim(), g.description ?? '', g.orgRole ?? null, g.delegateToChildren ? 1 : 0, now());
  return getGroup(id)!;
}

export function updateGroup(id: string, g: Partial<GroupInput>): Group | null {
  const cur = getGroup(id);
  if (!cur) return null;
  const next = { ...cur, ...g, name: g.name?.trim() || cur.name };
  db().prepare('UPDATE groups SET name = ?, description = ?, org_role = ?, delegate_to_children = ? WHERE id = ?').run(next.name, next.description ?? '', next.orgRole ?? null, next.delegateToChildren ? 1 : 0, id);
  return getGroup(id);
}

export function deleteGroup(id: string): boolean {
  return db().prepare('DELETE FROM groups WHERE id = ?').run(id).changes > 0;
}

export interface GroupMember {
  userId: string;
  role: GroupRole;
  addedAt: string;
}

export function groupMembers(groupId: string): GroupMember[] {
  return (db().prepare('SELECT * FROM group_members WHERE group_id = ? ORDER BY added_at').all(groupId) as Row[]).map((r) => ({ userId: r.user_id as string, role: r.role as GroupRole, addedAt: r.added_at as string }));
}

export function setGroupMember(groupId: string, userId: string, role: GroupRole): void {
  db().prepare('INSERT INTO group_members (group_id, user_id, role, added_at) VALUES (?, ?, ?, ?) ON CONFLICT(group_id, user_id) DO UPDATE SET role = excluded.role').run(groupId, userId, role, now());
}

/** Remove a member; the last owner cannot be removed or demoted. */
export function removeGroupMember(groupId: string, userId: string): boolean {
  const m = groupMembers(groupId);
  const target = m.find((x) => x.userId === userId);
  if (!target) return false;
  if (target.role === 'owner' && m.filter((x) => x.role === 'owner').length === 1) throw new OrgError('A group needs at least one owner', 409);
  return db().prepare('DELETE FROM group_members WHERE group_id = ? AND user_id = ?').run(groupId, userId).changes > 0;
}

export function demoteCheck(groupId: string, userId: string, next: GroupRole): void {
  if (next === 'owner') return;
  const m = groupMembers(groupId);
  if (m.find((x) => x.userId === userId)?.role === 'owner' && m.filter((x) => x.role === 'owner').length === 1) throw new OrgError('A group needs at least one owner', 409);
}

export function groupsOfUser(userId: string): (Group & { memberRole: GroupRole })[] {
  return (db().prepare('SELECT g.*, m.role AS member_role FROM groups g JOIN group_members m ON m.group_id = g.id WHERE m.user_id = ?').all(userId) as Row[]).map((r) => ({ ...fromRow(r), memberRole: r.member_role as GroupRole }));
}

export function isGroupOwner(groupId: string, userId: string): boolean {
  return !!db().prepare("SELECT 1 FROM group_members WHERE group_id = ? AND user_id = ? AND role = 'owner'").get(groupId, userId);
}

// ── Access grants (machine / backup / tag scoping) ──

export interface Grant {
  resourceType: ResourceType;
  resourceId: string;
}

export function groupGrants(groupId: string): Grant[] {
  return (db().prepare('SELECT resource_type, resource_id FROM group_access WHERE group_id = ? ORDER BY resource_type, resource_id').all(groupId) as Row[]).map((r) => ({ resourceType: r.resource_type as ResourceType, resourceId: r.resource_id as string }));
}

export function setGroupGrants(groupId: string, grants: Grant[]): void {
  const d = db();
  d.transaction(() => {
    d.prepare('DELETE FROM group_access WHERE group_id = ?').run(groupId);
    const ins = d.prepare('INSERT OR IGNORE INTO group_access (group_id, resource_type, resource_id) VALUES (?, ?, ?)');
    for (const g of grants) {
      if (!['machine', 'tag', 'backup'].includes(g.resourceType) || !g.resourceId) continue;
      ins.run(groupId, g.resourceType, g.resourceId);
    }
  })();
}

/** Grants a user holds in `orgId` through their groups (for access scoping). */
export function userGrants(userId: string, orgId: string): Grant[] {
  return (db()
    .prepare('SELECT DISTINCT a.resource_type, a.resource_id FROM group_access a JOIN groups g ON g.id = a.group_id JOIN group_members m ON m.group_id = g.id WHERE m.user_id = ? AND g.org_id = ?')
    .all(userId, orgId) as Row[]).map((r) => ({ resourceType: r.resource_type as ResourceType, resourceId: r.resource_id as string }));
}
