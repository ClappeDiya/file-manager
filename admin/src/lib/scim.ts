/**
 * SCIM 2.0 service provider (RFC 7643/7644 subset used by Entra ID, Okta
 * and others): Users and Groups of one organization, chosen by the bearer
 * token.
 *
 * - Users: `userName` is the e-mail address; `active: false` (or DELETE)
 *   removes the membership in this organization (and disables the account
 *   when it has no other organization); reactivation restores it with the
 *   token's default role.
 * - Groups: console groups the identity provider created. Their members
 *   are console users of the organization. SCIM never sets roles.
 *
 * Safety: SCIM cannot change users who hold an admin role directly in the
 * organization, and cannot change groups it did not create (an existing
 * group may grant an organization role).
 */
import { NextResponse } from 'next/server';
import { createUser, findUserByEmail, getUser, listUsers, updateUser, type UserRecord } from './db/repos/users';
import { membershipsOf, membersOf, OrgError, removeMembership, setMembership } from './db/repos/orgs';
import { createGroup, deleteGroup, getGroup, groupMembers, groupsOfUser, listGroups, removeGroupMember, setGroupMember, updateGroup, type Group } from './db/repos/groups';
import { db } from './db';
import { dropLink, findByExternalId, getLink, touchLink, type ScimResource, type ScimToken } from './db/repos/scim';
import { appendAudit } from './db/repos/audit';
import type { Role } from './types/auth';

export const SCIM_CONTENT_TYPE = 'application/scim+json';
const USER_SCHEMA = 'urn:ietf:params:scim:schemas:core:2.0:User';
const GROUP_SCHEMA = 'urn:ietf:params:scim:schemas:core:2.0:Group';
const LIST_SCHEMA = 'urn:ietf:params:scim:api:messages:2.0:ListResponse';
const PATCH_SCHEMA = 'urn:ietf:params:scim:api:messages:2.0:PatchOp';
const ERROR_SCHEMA = 'urn:ietf:params:scim:api:messages:2.0:Error';
export const MAX_RESULTS = 200;
/** Roles held directly in the organization that SCIM may not change. */
const PROTECTED_ROLES: Role[] = ['org_owner', 'org_admin', 'security_admin'];

export class ScimError extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly scimType?: string,
  ) {
    super(message);
  }
}

export function scimJson(body: unknown, status = 200, headers: Record<string, string> = {}): NextResponse {
  return new NextResponse(status === 204 ? null : JSON.stringify(body), { status, headers: { 'content-type': SCIM_CONTENT_TYPE, ...headers } });
}

export function scimErrorResponse(e: unknown): NextResponse {
  const err = e instanceof ScimError ? e : e instanceof OrgError ? new ScimError(e.status, e.message, e.status === 409 ? 'uniqueness' : undefined) : null;
  if (!err) throw e;
  return scimJson({ schemas: [ERROR_SCHEMA], status: String(err.status), ...(err.scimType ? { scimType: err.scimType } : {}), detail: err.message }, err.status);
}

// ── Filters: `attr eq "value"` joined by `and` ──

export interface Clause {
  attr: string;
  value: string;
}

export function parseFilter(filter: string | null): Clause[] {
  if (!filter?.trim()) return [];
  return filter.split(/\s+and\s+/i).map((part) => {
    const m = /^\s*([A-Za-z][\w.:]*)\s+eq\s+"((?:[^"\\]|\\.)*)"\s*$/i.exec(part);
    if (!m) throw new ScimError(400, `Unsupported filter '${part.trim()}' (use attribute eq "value")`, 'invalidFilter');
    return { attr: m[1].toLowerCase().replace(/^urn:ietf:params:scim:schemas:core:2\.0:(user|group):/, ''), value: m[2].replace(/\\(.)/g, '$1') };
  });
}

function page<T>(items: T[], q: URLSearchParams): { startIndex: number; slice: T[] } {
  const startIndex = Math.max(1, Number(q.get('startIndex')) || 1);
  const count = Math.min(MAX_RESULTS, Math.max(0, q.get('count') === null ? 100 : Number(q.get('count')) || 0));
  return { startIndex, slice: items.slice(startIndex - 1, startIndex - 1 + count) };
}

function listResponse(resources: unknown[], total: number, startIndex: number) {
  return { schemas: [LIST_SCHEMA], totalResults: total, startIndex, itemsPerPage: resources.length, Resources: resources };
}

const truthy = (v: unknown): boolean => (typeof v === 'string' ? v.toLowerCase() === 'true' : Boolean(v));

function audit(t: ScimToken, eventType: string, description: string, resourceType: string, resourceId: string) {
  appendAudit({ eventType, orgId: t.orgId, userEmail: `scim:${t.name}`, description, resourceType, resourceId, metadata: { scimTokenId: t.id } });
}

// ── Users ──

function directRole(userId: string, orgId: string): Role | null {
  const r = db().prepare('SELECT role FROM org_memberships WHERE user_id = ? AND org_id = ?').get(userId, orgId) as { role: Role } | undefined;
  return r?.role ?? null;
}

/** Users this organization's SCIM can see: members, and accounts it provisioned (also when deactivated). */
function orgUserIds(orgId: string): string[] {
  const linked = (db().prepare("SELECT resource_id FROM scim_links WHERE org_id = ? AND resource_type = 'User'").all(orgId) as { resource_id: string }[]).map((r) => r.resource_id);
  return [...new Set([...membersOf(orgId).map((m) => m.userId), ...linked])];
}

function visibleUser(orgId: string, id: string): UserRecord {
  const u = getUser(id);
  if (!u || !orgUserIds(orgId).includes(id)) throw new ScimError(404, `User ${id} not found`);
  return u;
}

function splitName(full: string): { givenName: string; familyName: string } {
  const parts = full.trim().split(/\s+/);
  return { givenName: parts.slice(0, -1).join(' ') || parts[0] || '', familyName: parts.length > 1 ? parts[parts.length - 1] : '' };
}

export function userResource(orgId: string, u: UserRecord, base: string) {
  const link = getLink(orgId, 'User', u.id);
  const active = u.isActive && directRole(u.id, orgId) !== null;
  return {
    schemas: [USER_SCHEMA],
    id: u.id,
    ...(link?.externalId ? { externalId: link.externalId } : {}),
    userName: u.email,
    name: { formatted: u.name, ...splitName(u.name) },
    displayName: u.name,
    emails: [{ value: u.email, type: 'work', primary: true }],
    active,
    groups: groupsOfUser(u.id)
      .filter((g) => g.orgId === orgId)
      .map((g) => ({ value: g.id, display: g.name, $ref: `${base}/Groups/${g.id}` })),
    meta: { resourceType: 'User', created: link?.createdAt ?? u.createdAt, lastModified: link?.updatedAt ?? u.createdAt, location: `${base}/Users/${u.id}` },
  };
}

interface UserFields {
  userName?: string;
  name?: string;
  externalId?: string | null;
  active?: boolean;
}

/** The fields a SCIM user body sets. */
function userFields(body: Record<string, unknown>): UserFields {
  const f: UserFields = {};
  if (typeof body.userName === 'string') f.userName = body.userName;
  const name = body.name as { givenName?: string; familyName?: string; formatted?: string } | undefined;
  const fromParts = name && [name.givenName, name.familyName].filter(Boolean).join(' ');
  const display = (typeof body.displayName === 'string' && body.displayName) || name?.formatted || fromParts;
  if (display) f.name = display;
  if (body.externalId !== undefined) f.externalId = body.externalId === null ? null : String(body.externalId);
  if (body.active !== undefined) f.active = truthy(body.active);
  const emails = body.emails as { value?: string; primary?: boolean }[] | undefined;
  if (!f.userName && Array.isArray(emails)) f.userName = (emails.find((e) => e.primary) ?? emails[0])?.value;
  return f;
}

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function setActive(t: ScimToken, u: UserRecord, active: boolean) {
  if (active) {
    if (!directRole(u.id, t.orgId)) setMembership(u.id, t.orgId, t.defaultRole);
    if (!u.isActive) updateUser(u.id, { isActive: true });
  } else {
    removeMembership(u.id, t.orgId);
    // Only this organization's account: disable it so it cannot sign in.
    if (membershipsOf(u.id).length === 0) updateUser(u.id, { isActive: false });
  }
}

function guardUser(t: ScimToken, u: UserRecord) {
  const role = directRole(u.id, t.orgId);
  if (role && PROTECTED_ROLES.includes(role)) throw new ScimError(403, `${u.email} is an administrator of this organization and is managed in the console, not by SCIM`);
}

function applyUser(t: ScimToken, u: UserRecord, f: UserFields) {
  guardUser(t, u);
  if (f.userName !== undefined && f.userName.toLowerCase() !== u.email) {
    if (!EMAIL.test(f.userName)) throw new ScimError(400, 'userName must be an e-mail address', 'invalidValue');
    const other = findUserByEmail(f.userName);
    if (other && other.id !== u.id) throw new ScimError(409, `${f.userName} is already a user`, 'uniqueness');
    updateUser(u.id, { email: f.userName.trim().toLowerCase() });
  }
  if (f.name !== undefined && f.name.trim() && f.name.trim() !== u.name) updateUser(u.id, { name: f.name.trim().slice(0, 200) });
  touchLink(t.orgId, 'User', u.id, f.externalId);
  if (f.active !== undefined) setActive(t, getUser(u.id)!, f.active);
}

export function listScimUsers(t: ScimToken, q: URLSearchParams, base: string) {
  const clauses = parseFilter(q.get('filter'));
  let users = listUsers(orgUserIds(t.orgId)).sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id));
  for (const c of clauses) {
    if (c.attr === 'username' || c.attr === 'emails.value' || c.attr === 'emails') users = users.filter((u) => u.email === c.value.toLowerCase());
    else if (c.attr === 'externalid') users = users.filter((u) => getLink(t.orgId, 'User', u.id)?.externalId === c.value);
    else if (c.attr === 'id') users = users.filter((u) => u.id === c.value);
    else if (c.attr === 'displayname') users = users.filter((u) => u.name === c.value);
    else if (c.attr === 'active') users = users.filter((u) => (u.isActive && directRole(u.id, t.orgId) !== null) === truthy(c.value));
    else throw new ScimError(400, `Filtering on '${c.attr}' is not supported`, 'invalidFilter');
  }
  const { startIndex, slice } = page(users, q);
  return listResponse(slice.map((u) => userResource(t.orgId, u, base)), users.length, startIndex);
}

export function getScimUser(t: ScimToken, id: string, base: string) {
  return userResource(t.orgId, visibleUser(t.orgId, id), base);
}

export function createScimUser(t: ScimToken, body: Record<string, unknown>, base: string) {
  const f = userFields(body);
  if (!f.userName || !EMAIL.test(f.userName)) throw new ScimError(400, 'userName (an e-mail address) is required', 'invalidValue');
  if (f.externalId && findByExternalId(t.orgId, 'User', f.externalId)) throw new ScimError(409, `A user with externalId '${f.externalId}' exists`, 'uniqueness');
  let u = findUserByEmail(f.userName);
  if (u && orgUserIds(t.orgId).includes(u.id)) throw new ScimError(409, `${f.userName} is already provisioned`, 'uniqueness');
  if (u) {
    // An account from another organization: link it, keep its name and sign-in.
    guardUser(t, u);
  } else {
    u = createUser({ email: f.userName, name: f.name, role: t.defaultRole, organizationId: t.orgId, ssoProvider: 'scim' });
  }
  touchLink(t.orgId, 'User', u.id, f.externalId ?? null);
  setActive(t, getUser(u.id)!, f.active ?? true);
  audit(t, 'scim.user_create', `SCIM provisioned ${u.email}`, 'user', u.id);
  return userResource(t.orgId, getUser(u.id)!, base);
}

export function replaceScimUser(t: ScimToken, id: string, body: Record<string, unknown>, base: string) {
  const u = visibleUser(t.orgId, id);
  const f = userFields(body);
  applyUser(t, u, { ...f, active: f.active ?? true, externalId: f.externalId ?? null });
  audit(t, 'scim.user_update', `SCIM replaced ${getUser(id)!.email}`, 'user', id);
  return userResource(t.orgId, getUser(id)!, base);
}

interface PatchOp {
  op?: string;
  path?: string;
  value?: unknown;
}

function patchOps(body: Record<string, unknown>): PatchOp[] {
  const schemas = body.schemas as string[] | undefined;
  if (schemas && !schemas.includes(PATCH_SCHEMA)) throw new ScimError(400, 'Not a PatchOp request', 'invalidSyntax');
  const ops = body.Operations as PatchOp[] | undefined;
  if (!Array.isArray(ops) || !ops.length) throw new ScimError(400, 'Operations are required', 'invalidSyntax');
  return ops.map((o) => {
    const op = String(o.op ?? '').toLowerCase();
    if (!['add', 'replace', 'remove'].includes(op)) throw new ScimError(400, `Unknown operation '${o.op}'`, 'invalidSyntax');
    return { ...o, op };
  });
}

export function patchScimUser(t: ScimToken, id: string, body: Record<string, unknown>, base: string) {
  const u = visibleUser(t.orgId, id);
  const f: UserFields = {};
  for (const o of patchOps(body)) {
    // No path: the value is a partial user.
    if (!o.path) {
      if (o.op === 'remove') throw new ScimError(400, 'remove needs a path', 'noTarget');
      Object.assign(f, userFields((o.value ?? {}) as Record<string, unknown>));
      continue;
    }
    const path = o.path.toLowerCase().replace(/^urn:ietf:params:scim:schemas:core:2\.0:user:/, '');
    const v = o.op === 'remove' ? null : o.value;
    if (path === 'active') f.active = o.op === 'remove' ? false : truthy(v);
    else if (path === 'username') f.userName = String(v ?? '');
    else if (path === 'displayname' || path === 'name.formatted') f.name = String(v ?? '');
    else if (path === 'name.givenname') f.name = `${String(v ?? '')} ${splitName(f.name ?? u.name).familyName}`.trim();
    else if (path === 'name.familyname') f.name = `${splitName(f.name ?? u.name).givenName} ${String(v ?? '')}`.trim();
    else if (path === 'externalid') f.externalId = v === null ? null : String(v);
    else if (/^emails(\[.*\])?(\.value)?$/.test(path) && v !== null) f.userName = Array.isArray(v) ? (v as { value?: string }[])[0]?.value : typeof v === 'object' ? (v as { value?: string }).value : String(v);
    else if (path.startsWith('name') || path.startsWith('emails') || path.startsWith('title') || path.startsWith('phonenumbers') || path.startsWith('addresses') || path.startsWith('urn:')) continue; // attributes the console does not keep
    else throw new ScimError(400, `Unsupported path '${o.path}'`, 'invalidPath');
  }
  applyUser(t, u, f);
  audit(t, 'scim.user_update', `SCIM updated ${getUser(id)!.email}`, 'user', id);
  return userResource(t.orgId, getUser(id)!, base);
}

export function deleteScimUser(t: ScimToken, id: string) {
  const u = visibleUser(t.orgId, id);
  guardUser(t, u);
  setActive(t, u, false);
  for (const g of groupsOfUser(u.id).filter((g) => g.orgId === t.orgId && getLink(t.orgId, 'Group', g.id))) removeGroupMember(g.id, u.id);
  dropLink(t.orgId, 'User', u.id);
  audit(t, 'scim.user_delete', `SCIM deprovisioned ${u.email}`, 'user', id);
}

// ── Groups ──

export function groupResource(orgId: string, g: Group, base: string, withMembers = true) {
  const link = getLink(orgId, 'Group', g.id);
  return {
    schemas: [GROUP_SCHEMA],
    id: g.id,
    ...(link?.externalId ? { externalId: link.externalId } : {}),
    displayName: g.name,
    ...(withMembers ? { members: groupMembers(g.id).map((m) => ({ value: m.userId, display: getUser(m.userId)?.email ?? m.userId, $ref: `${base}/Users/${m.userId}` })) } : {}),
    meta: { resourceType: 'Group', created: link?.createdAt ?? g.createdAt, lastModified: link?.updatedAt ?? g.createdAt, location: `${base}/Groups/${g.id}` },
  };
}

function visibleGroup(orgId: string, id: string): Group {
  const g = getGroup(id);
  if (!g || g.orgId !== orgId) throw new ScimError(404, `Group ${id} not found`);
  return g;
}

/** Only groups SCIM created, and only without an organization role. */
function guardGroup(t: ScimToken, g: Group) {
  if (!getLink(t.orgId, 'Group', g.id)) throw new ScimError(403, `Group "${g.name}" was created in the console and is not managed by SCIM`);
  if (g.orgRole) throw new ScimError(403, `Group "${g.name}" grants an organization role and is managed in the console`);
}

function memberIds(value: unknown): string[] {
  const list = Array.isArray(value) ? value : value ? [value] : [];
  return list.map((m) => (typeof m === 'string' ? m : String((m as { value?: unknown }).value ?? ''))).filter(Boolean);
}

function addMembers(t: ScimToken, g: Group, ids: string[]) {
  const allowed = orgUserIds(t.orgId);
  for (const id of ids) {
    if (!allowed.includes(id)) throw new ScimError(400, `User ${id} is not provisioned in this organization`, 'invalidValue');
    if (!groupMembers(g.id).some((m) => m.userId === id)) setGroupMember(g.id, id, 'member');
  }
}

function removeMembers(g: Group, ids: string[]) {
  for (const id of ids) removeGroupMember(g.id, id);
}

export function listScimGroups(t: ScimToken, q: URLSearchParams, base: string) {
  const clauses = parseFilter(q.get('filter'));
  let groups = listGroups(t.orgId).sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id));
  for (const c of clauses) {
    if (c.attr === 'displayname') groups = groups.filter((g) => g.name.toLowerCase() === c.value.toLowerCase());
    else if (c.attr === 'externalid') groups = groups.filter((g) => getLink(t.orgId, 'Group', g.id)?.externalId === c.value);
    else if (c.attr === 'id') groups = groups.filter((g) => g.id === c.value);
    else if (c.attr === 'members.value' || c.attr === 'members') groups = groups.filter((g) => groupMembers(g.id).some((m) => m.userId === c.value));
    else throw new ScimError(400, `Filtering on '${c.attr}' is not supported`, 'invalidFilter');
  }
  const withMembers = !(q.get('excludedAttributes') ?? '').toLowerCase().split(',').includes('members');
  const { startIndex, slice } = page(groups, q);
  return listResponse(slice.map((g) => groupResource(t.orgId, g, base, withMembers)), groups.length, startIndex);
}

export function getScimGroup(t: ScimToken, id: string, q: URLSearchParams, base: string) {
  const withMembers = !(q.get('excludedAttributes') ?? '').toLowerCase().split(',').includes('members');
  return groupResource(t.orgId, visibleGroup(t.orgId, id), base, withMembers);
}

export function createScimGroup(t: ScimToken, body: Record<string, unknown>, base: string) {
  const name = typeof body.displayName === 'string' ? body.displayName.trim() : '';
  if (!name) throw new ScimError(400, 'displayName is required', 'invalidValue');
  const externalId = body.externalId === undefined || body.externalId === null ? null : String(body.externalId);
  if (externalId && findByExternalId(t.orgId, 'Group', externalId)) throw new ScimError(409, `A group with externalId '${externalId}' exists`, 'uniqueness');
  const g = createGroup(t.orgId, { name, description: 'Provisioned by SCIM' });
  touchLink(t.orgId, 'Group', g.id, externalId);
  addMembers(t, g, memberIds(body.members));
  audit(t, 'scim.group_create', `SCIM created group "${g.name}"`, 'group', g.id);
  return groupResource(t.orgId, getGroup(g.id)!, base);
}

export function replaceScimGroup(t: ScimToken, id: string, body: Record<string, unknown>, base: string) {
  const g = visibleGroup(t.orgId, id);
  guardGroup(t, g);
  const name = typeof body.displayName === 'string' ? body.displayName.trim() : g.name;
  if (name && name !== g.name) updateGroup(g.id, { name });
  touchLink(t.orgId, 'Group', g.id, body.externalId === undefined ? undefined : body.externalId === null ? null : String(body.externalId));
  const want = memberIds(body.members);
  removeMembers(g, groupMembers(g.id).map((m) => m.userId).filter((u) => !want.includes(u)));
  addMembers(t, g, want);
  audit(t, 'scim.group_update', `SCIM replaced group "${name}"`, 'group', g.id);
  return groupResource(t.orgId, getGroup(g.id)!, base);
}

export function patchScimGroup(t: ScimToken, id: string, body: Record<string, unknown>, base: string) {
  const g = visibleGroup(t.orgId, id);
  guardGroup(t, g);
  for (const o of patchOps(body)) {
    const path = (o.path ?? '').toLowerCase();
    const filtered = /^members\[value eq "([^"]+)"\]$/i.exec(o.path ?? '');
    if (!o.path) {
      const v = (o.value ?? {}) as Record<string, unknown>;
      if (typeof v.displayName === 'string' && v.displayName.trim()) updateGroup(g.id, { name: v.displayName.trim() });
      if (v.externalId !== undefined) touchLink(t.orgId, 'Group', g.id, v.externalId === null ? null : String(v.externalId));
      if (v.members !== undefined) {
        if (o.op === 'replace') removeMembers(g, groupMembers(g.id).map((m) => m.userId));
        addMembers(t, g, memberIds(v.members));
      }
    } else if (path === 'displayname') {
      if (o.op === 'remove' || typeof o.value !== 'string' || !o.value.trim()) throw new ScimError(400, 'displayName cannot be removed', 'mutability');
      updateGroup(g.id, { name: o.value.trim() });
    } else if (path === 'externalid') {
      touchLink(t.orgId, 'Group', g.id, o.op === 'remove' ? null : String(o.value));
    } else if (filtered && o.op === 'remove') {
      removeMembers(g, [filtered[1]]);
    } else if (path === 'members') {
      if (o.op === 'remove') removeMembers(g, o.value === undefined ? groupMembers(g.id).map((m) => m.userId) : memberIds(o.value));
      else {
        if (o.op === 'replace') removeMembers(g, groupMembers(g.id).map((m) => m.userId));
        addMembers(t, g, memberIds(o.value));
      }
    } else {
      throw new ScimError(400, `Unsupported path '${o.path}'`, 'invalidPath');
    }
  }
  touchLink(t.orgId, 'Group', g.id);
  audit(t, 'scim.group_update', `SCIM updated group "${getGroup(g.id)!.name}"`, 'group', g.id);
  return groupResource(t.orgId, getGroup(g.id)!, base);
}

export function deleteScimGroup(t: ScimToken, id: string) {
  const g = visibleGroup(t.orgId, id);
  guardGroup(t, g);
  deleteGroup(g.id);
  dropLink(t.orgId, 'Group', g.id);
  audit(t, 'scim.group_delete', `SCIM deleted group "${g.name}"`, 'group', g.id);
}

// ── Discovery ──

export function serviceProviderConfig(base: string) {
  return {
    schemas: ['urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig'],
    documentationUri: `${base.replace(/\/api\/scim\/v2$/, '')}/docs/admin-console`,
    patch: { supported: true },
    bulk: { supported: false, maxOperations: 0, maxPayloadSize: 0 },
    filter: { supported: true, maxResults: MAX_RESULTS },
    changePassword: { supported: false },
    sort: { supported: false },
    etag: { supported: false },
    authenticationSchemes: [{ type: 'oauthbearertoken', name: 'Bearer token', description: 'A SCIM token from Users → Provisioning', primary: true }],
    meta: { resourceType: 'ServiceProviderConfig', location: `${base}/ServiceProviderConfig` },
  };
}

export function resourceTypes(base: string) {
  const rt = (name: ScimResource, schema: string) => ({ schemas: ['urn:ietf:params:scim:schemas:core:2.0:ResourceType'], id: name, name, endpoint: `/${name}s`, schema, meta: { resourceType: 'ResourceType', location: `${base}/ResourceTypes/${name}` } });
  return listResponse([rt('User', USER_SCHEMA), rt('Group', GROUP_SCHEMA)], 2, 1);
}

export function schemas() {
  const attr = (name: string, type = 'string', extra: Record<string, unknown> = {}) => ({ name, type, multiValued: false, required: false, mutability: 'readWrite', returned: 'default', uniqueness: 'none', ...extra });
  return listResponse(
    [
      { schemas: ['urn:ietf:params:scim:schemas:core:2.0:Schema'], id: USER_SCHEMA, name: 'User', attributes: [attr('userName', 'string', { required: true, uniqueness: 'server' }), attr('displayName'), attr('name', 'complex'), attr('emails', 'complex', { multiValued: true }), attr('active', 'boolean'), attr('externalId'), attr('groups', 'complex', { multiValued: true, mutability: 'readOnly' })] },
      { schemas: ['urn:ietf:params:scim:schemas:core:2.0:Schema'], id: GROUP_SCHEMA, name: 'Group', attributes: [attr('displayName', 'string', { required: true }), attr('externalId'), attr('members', 'complex', { multiValued: true })] },
    ],
    2,
    1,
  );
}
