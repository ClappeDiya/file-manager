/**
 * Server-side CSV user import: parse (RFC 4180), check every row, and
 * invite the valid ones through the normal invitation flow.
 *
 * Columns (header row, any order, case-insensitive): `email` (required),
 * `role` (default `user`), `group` (name of an existing group in the
 * organization, optional), `group_role` (`member`|`owner`).
 */
import { ROLE_HIERARCHY, type Role } from './types/auth';
import { canManageRole } from './utils/permissions';
import { findUserByEmail } from './db/repos/users';
import { listGroups, type Group } from './db/repos/groups';
import { db } from './db';

export const MAX_IMPORT_ROWS = 1000;
const EMAIL = /^[^@\s]+@[^@\s]+\.[^@\s]+$/;

/** RFC 4180 CSV: quoted fields with "" escapes, commas and line breaks inside quotes, CRLF/LF, BOM. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  const s = text.replace(/^\uFEFF/, '');
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quoted) {
      if (c === '"' && s[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"' && field === '') quoted = true;
    else if (c === ',') {
      row.push(field);
      field = '';
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && s[i + 1] === '\n') i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else field += c;
  }
  if (quoted) throw new Error('A quoted field is not closed');
  if (field !== '' || row.length) {
    row.push(field);
    rows.push(row);
  }
  return rows.filter((r) => r.some((f) => f.trim() !== ''));
}

export interface ImportRow {
  line: number;
  email: string;
  role: Role;
  group: string | null;
  groupId: string | null;
  groupRole: 'member' | 'owner';
  action: 'invite' | 'skip' | 'error';
  message: string;
}

/** Check every row against the organization and the importer's role. */
export function planImport(orgId: string, importerRole: Role, csv: string): { rows: ImportRow[]; error?: string } {
  let table: string[][];
  try {
    table = parseCsv(csv);
  } catch (e) {
    return { rows: [], error: (e as Error).message };
  }
  if (!table.length) return { rows: [], error: 'The file is empty' };
  const header = table[0].map((h) => h.trim().toLowerCase().replace(/\s+/g, '_'));
  const col = (name: string) => header.indexOf(name);
  if (col('email') < 0) return { rows: [], error: 'The first row must name the columns and include "email"' };
  if (table.length - 1 > MAX_IMPORT_ROWS) return { rows: [], error: `At most ${MAX_IMPORT_ROWS} rows per import` };
  const groups = new Map<string, Group>(listGroups(orgId).map((g) => [g.name.toLowerCase(), g]));
  const seen = new Set<string>();
  const rows: ImportRow[] = table.slice(1).map((r, i) => {
    const get = (name: string) => (col(name) >= 0 ? (r[col(name)] ?? '').trim() : '');
    const email = get('email').toLowerCase();
    const roleRaw = get('role').toLowerCase().replace(/\s+/g, '_') || 'user';
    const group = get('group') || null;
    const groupRoleRaw = get('group_role').toLowerCase() || 'member';
    const base: ImportRow = { line: i + 2, email, role: roleRaw as Role, group, groupId: null, groupRole: groupRoleRaw === 'owner' ? 'owner' : 'member', action: 'invite', message: 'Will be invited' };
    const fail = (message: string): ImportRow => ({ ...base, action: 'error', message });
    if (!EMAIL.test(email)) return fail('Not a valid e-mail address');
    if (seen.has(email)) return fail('Listed more than once');
    seen.add(email);
    if (!(roleRaw in ROLE_HIERARCHY)) return fail(`Unknown role "${roleRaw}"`);
    if (roleRaw !== 'user' && importerRole !== 'org_owner' && !canManageRole(importerRole, roleRaw as Role)) return fail(`You cannot invite someone as ${roleRaw}`);
    if (!['member', 'owner'].includes(groupRoleRaw)) return fail('group_role is member or owner');
    let groupId: string | null = null;
    if (group) {
      const g = groups.get(group.toLowerCase());
      if (!g) return fail(`No group named "${group}"`);
      groupId = g.id;
    }
    const existing = findUserByEmail(email);
    const member = existing && db().prepare('SELECT 1 FROM org_memberships WHERE user_id = ? AND org_id = ?').get(existing.id, orgId);
    if (member && !groupId) return { ...base, action: 'skip', message: 'Already a member' };
    return { ...base, groupId, message: member ? `Already a member; will be invited to "${group}"` : existing ? 'Has an account; will be invited' : 'Will be invited' };
  });
  return { rows };
}
