/**
 * Machines (backup clients) and their enrollment.
 *
 * Lifecycle: a client enrolls with a registration link token or through an
 * interactive claim code → `active` (holds an agent token, stored hashed) →
 * optionally `suspended` (token kept, requests refused) → `revoked` (token
 * removed; re-enrollment creates a fresh token). Admins can rotate tokens;
 * the agent picks the new one up on its next request.
 */
import { createHash } from 'node:crypto';
import { db, hashToken, newId, newToken, now, parseJson, tx, type Row } from '../index';
import { open, seal } from '../../crypto';
import { getOrg, license, OrgError } from './orgs';

export type MachineStatus = 'active' | 'suspended' | 'revoked';
export type Connectivity = 'online' | 'stale' | 'offline' | 'never';

export interface JobSummary {
  id: string;
  name: string;
  scheduleIntervalMinutes: number | null;
  lastResult: string | null;
  lastRunAt: string | null;
  managedByPolicy?: string | null;
}

export interface Machine {
  id: string;
  orgId: string;
  name: string;
  hostname: string;
  os: string;
  osVersion: string;
  arch: string;
  clientVersion: string;
  agentKind: string;
  status: MachineStatus;
  heartbeatSeconds: number;
  registeredAt: string;
  registeredVia: string;
  lastSeenAt: string | null;
  lastIp: string;
  lastReportAt: string | null;
  tags: string[];
  notes: string;
  appliedPolicies: Record<string, number>;
  jobs: JobSummary[];
  revokedAt: string | null;
  connectivity: Connectivity;
}

export function connectivity(lastSeen: string | null, heartbeatSeconds: number, at = Date.now()): Connectivity {
  if (!lastSeen) return 'never';
  const age = (at - new Date(lastSeen).getTime()) / 1000;
  if (age <= Math.max(180, heartbeatSeconds * 3)) return 'online';
  if (age <= 86_400) return 'stale';
  return 'offline';
}

function fromRow(r: Row): Machine {
  const hb = Number(r.heartbeat_seconds ?? 60);
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    hostname: r.hostname as string,
    os: r.os as string,
    osVersion: r.os_version as string,
    arch: r.arch as string,
    clientVersion: r.client_version as string,
    agentKind: r.agent_kind as string,
    status: r.status as MachineStatus,
    heartbeatSeconds: hb,
    registeredAt: r.registered_at as string,
    registeredVia: r.registered_via as string,
    lastSeenAt: (r.last_seen_at as string) ?? null,
    lastIp: r.last_ip as string,
    lastReportAt: (r.last_report_at as string) ?? null,
    tags: parseJson<string[]>(r.tags, []),
    notes: r.notes as string,
    appliedPolicies: parseJson(r.applied_policies, {}),
    jobs: parseJson<JobSummary[]>(r.jobs, []),
    revokedAt: (r.revoked_at as string) ?? null,
    connectivity: connectivity((r.last_seen_at as string) ?? null, hb),
  };
}

export function normalizeTags(tags: unknown): string[] {
  if (!Array.isArray(tags)) return [];
  const out = new Set<string>();
  for (const t of tags) {
    const s = String(t).trim().toLowerCase().replace(/\s+/g, '-').slice(0, 40);
    if (/^[a-z0-9][a-z0-9._:-]*$/.test(s)) out.add(s);
  }
  return [...out].sort();
}

export function getMachine(id: string): Machine | null {
  const r = db().prepare('SELECT * FROM machines WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export interface MachineFilter {
  orgIds: string[];
  tags?: string[];
  status?: MachineStatus;
  connectivity?: Connectivity;
  search?: string;
  /** Restrict to these ids (access scoping); undefined = no restriction. */
  ids?: string[];
}

export function listMachines(f: MachineFilter): Machine[] {
  if (f.orgIds.length === 0 || (f.ids && f.ids.length === 0)) return [];
  const where = [`org_id IN (${f.orgIds.map(() => '?').join(',')})`];
  const args: unknown[] = [...f.orgIds];
  if (f.ids) { where.push(`id IN (${f.ids.map(() => '?').join(',')})`); args.push(...f.ids); }
  if (f.status) { where.push('status = ?'); args.push(f.status); }
  else where.push("status != 'revoked'");
  let rows = (db().prepare(`SELECT * FROM machines WHERE ${where.join(' AND ')} ORDER BY lower(name)`).all(...args) as Row[]).map(fromRow);
  if (f.tags?.length) rows = rows.filter((m) => f.tags!.every((t) => m.tags.includes(t)));
  if (f.connectivity) rows = rows.filter((m) => m.connectivity === f.connectivity);
  if (f.search) {
    const q = f.search.toLowerCase();
    rows = rows.filter((m) => m.name.toLowerCase().includes(q) || m.hostname.toLowerCase().includes(q) || m.tags.some((t) => t.includes(q)));
  }
  return rows;
}

export interface ClientInfo {
  name?: string;
  hostname?: string;
  os?: string;
  osVersion?: string;
  arch?: string;
  clientVersion?: string;
  agentKind?: string;
}

const clip = (s: unknown, n = 120) => (typeof s === 'string' ? s.slice(0, n) : '');

function checkSeats(orgId: string) {
  const org = getOrg(orgId);
  if (!org || org.status !== 'active') throw new OrgError('This organization is not accepting new machines', 403);
  const lic = license(orgId);
  if (lic.seats !== null && lic.used >= lic.seats) throw new OrgError(`The license allows ${lic.seats} machine(s) and all are in use`, 402);
}

function insertMachine(orgId: string, info: ClientInfo, via: string, tags: string[]): { machine: Machine; token: string } {
  checkSeats(orgId);
  const id = newId('mch');
  const token = newToken('ufop_agent', 32);
  const ts = now();
  db()
    .prepare(`INSERT INTO machines (id, org_id, name, hostname, os, os_version, arch, client_version, agent_kind, status, token_hash, token_rotated_at, registered_at, registered_via, tags)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?)`)
    .run(id, orgId, clip(info.name) || clip(info.hostname) || 'Unnamed machine', clip(info.hostname), clip(info.os, 40), clip(info.osVersion, 60), clip(info.arch, 20), clip(info.clientVersion, 40), ['desktop', 'server', 'cli'].includes(info.agentKind ?? '') ? info.agentKind : 'desktop', hashToken(token), ts, ts, via, JSON.stringify(normalizeTags(tags)));
  return { machine: getMachine(id)!, token };
}

// ── Registration links ──

export interface RegistrationLink {
  id: string;
  orgId: string;
  label: string;
  tokenPrefix: string;
  createdBy: string;
  createdAt: string;
  expiresAt: string | null;
  maxUses: number | null;
  uses: number;
  revokedAt: string | null;
  defaultTags: string[];
  usable: boolean;
}

function linkFromRow(r: Row): RegistrationLink {
  const expired = !!r.expires_at && new Date(r.expires_at as string) <= new Date();
  const used = r.max_uses !== null && Number(r.uses) >= Number(r.max_uses);
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    label: r.label as string,
    tokenPrefix: r.token_prefix as string,
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    expiresAt: (r.expires_at as string) ?? null,
    maxUses: (r.max_uses as number) ?? null,
    uses: Number(r.uses),
    revokedAt: (r.revoked_at as string) ?? null,
    defaultTags: parseJson<string[]>(r.default_tags, []),
    usable: !r.revoked_at && !expired && !used,
  };
}

export function createLink(orgId: string, o: { label?: string; expiresInHours?: number | null; maxUses?: number | null; defaultTags?: string[] }, by: string): { link: RegistrationLink; token: string } {
  const token = newToken('ufop_reg');
  const id = newId('reg');
  const expires = o.expiresInHours ? new Date(Date.now() + o.expiresInHours * 3_600_000).toISOString() : null;
  db()
    .prepare('INSERT INTO registration_links (id, org_id, label, token_hash, token_prefix, token_ciphertext, created_by, created_at, expires_at, max_uses, default_tags) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, (o.label ?? '').slice(0, 80), hashToken(token), token.slice(0, 16), seal(token), by, now(), expires, o.maxUses ?? null, JSON.stringify(normalizeTags(o.defaultTags ?? [])));
  return { link: getLink(orgId, id)!, token };
}

export function getLink(orgId: string, id: string): RegistrationLink | null {
  const r = db().prepare('SELECT * FROM registration_links WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? linkFromRow(r) : null;
}

/** The raw token (for the preload file / copy link), admins only. */
export function linkToken(orgId: string, id: string): string | null {
  const r = db().prepare('SELECT token_ciphertext FROM registration_links WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? open(r.token_ciphertext as string) : null;
}

export function listLinks(orgId: string): RegistrationLink[] {
  return (db().prepare('SELECT * FROM registration_links WHERE org_id = ? ORDER BY created_at DESC').all(orgId) as Row[]).map(linkFromRow);
}

export function revokeLink(orgId: string, id: string): boolean {
  return db().prepare('UPDATE registration_links SET revoked_at = ? WHERE org_id = ? AND id = ? AND revoked_at IS NULL').run(now(), orgId, id).changes > 0;
}

/** Enroll with a registration token. */
export function registerWithToken(token: string, info: ClientInfo, extraTags: string[] = []): { machine: Machine; token: string } {
  return tx(() => {
    const r = db().prepare('SELECT * FROM registration_links WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
    if (!r) throw new OrgError('The registration token is not valid', 401);
    const link = linkFromRow(r);
    if (!link.usable) throw new OrgError(link.revokedAt ? 'This registration link was revoked' : 'This registration link has expired or reached its use limit', 410);
    db().prepare('UPDATE registration_links SET uses = uses + 1 WHERE id = ?').run(link.id);
    return insertMachine(link.orgId, info, `link:${link.id}`, [...link.defaultTags, ...extraTags]);
  });
}

/**
 * Report-only clients (no agent): a report URL carries a registration token as
 * report key; the machine is identified by its reported name and created on
 * its first report. Returns null when the token is not a usable link.
 */
export function machineForReport(token: string, machineName: string): Machine | null {
  const r = db().prepare('SELECT * FROM registration_links WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
  if (!r) return null;
  const link = linkFromRow(r);
  if (link.revokedAt || (link.expiresAt && new Date(link.expiresAt) <= new Date())) return null;
  const name = clip(machineName) || 'Unnamed machine';
  const existing = db().prepare("SELECT * FROM machines WHERE org_id = ? AND registered_via = ? AND lower(hostname) = lower(?) AND status != 'revoked'").get(link.orgId, `report:${link.id}`, name) as Row | undefined;
  if (existing) return fromRow(existing);
  return tx(() => {
    checkSeats(link.orgId);
    const id = newId('mch');
    const ts = now();
    db()
      .prepare(`INSERT INTO machines (id, org_id, name, hostname, agent_kind, status, registered_at, registered_via, tags) VALUES (?, ?, ?, ?, 'report', 'active', ?, ?, ?)`)
      .run(id, link.orgId, name, name, ts, `report:${link.id}`, JSON.stringify(link.defaultTags));
    return getMachine(id)!;
  });
}

// ── Interactive claiming ──

const CLAIM_MINUTES = 30;
const CODE_ALPHABET = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';

function newCode(): string {
  const b = crypto.getRandomValues(new Uint8Array(8));
  const c = Array.from(b, (x) => CODE_ALPHABET[x % CODE_ALPHABET.length]).join('');
  return `${c.slice(0, 4)}-${c.slice(4)}`;
}

export interface Claim {
  id: string;
  code: string;
  hostname: string;
  os: string;
  clientVersion: string;
  createdAt: string;
  expiresAt: string;
  status: 'pending' | 'claimed' | 'collected' | 'expired';
  machineId: string | null;
}

function claimFromRow(r: Row): Claim {
  const status = r.status === 'pending' && new Date(r.expires_at as string) <= new Date() ? 'expired' : (r.status as Claim['status']);
  return { id: r.id as string, code: r.code as string, hostname: r.hostname as string, os: r.os as string, clientVersion: r.client_version as string, createdAt: r.created_at as string, expiresAt: r.expires_at as string, status, machineId: (r.machine_id as string) ?? null };
}

/** A client asks to be claimed: it shows `code` to its user and polls with `secret`. */
export function startClaim(info: ClientInfo): { claimId: string; code: string; secret: string; expiresAt: string } {
  const id = newId('clm');
  const secret = newToken('ufop_claim', 24);
  const code = newCode();
  const expiresAt = new Date(Date.now() + CLAIM_MINUTES * 60_000).toISOString();
  db()
    .prepare('INSERT INTO machine_claims (id, code, secret_hash, hostname, os, client_version, agent_kind, created_at, expires_at, pending_token) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, code, hashToken(secret), clip(info.hostname), clip(info.os, 40), clip(info.clientVersion, 40), clip(info.agentKind, 10) || 'desktop', now(), expiresAt, JSON.stringify(info));
  return { claimId: id, code, secret, expiresAt };
}

/** An administrator enters the code shown on the machine. */
export function claimMachine(orgId: string, code: string, o: { name?: string; tags?: string[] }, by: string): Machine {
  return tx(() => {
    const norm = code.trim().toUpperCase().replace(/[^A-Z0-9]/g, '');
    const r = db().prepare("SELECT * FROM machine_claims WHERE replace(code, '-', '') = ?").get(norm) as Row | undefined;
    const c = r ? claimFromRow(r) : null;
    if (!c || c.status !== 'pending') throw new OrgError(c?.status === 'expired' ? 'This code has expired; ask for a new one on the machine' : 'No machine is waiting with this code', 404);
    const info = parseJson<ClientInfo>(r!.pending_token, {});
    const { machine, token } = insertMachine(orgId, { ...info, name: o.name || info.name }, `claim:${by}`, o.tags ?? []);
    // The token waits, sealed, until the machine collects it with its secret.
    db().prepare("UPDATE machine_claims SET status = 'claimed', machine_id = ?, org_id = ?, claimed_by = ?, pending_token = ? WHERE id = ?").run(machine.id, orgId, by, seal(token), c.id);
    return machine;
  });
}

/** The client polls; once claimed it receives its token exactly once. */
export function collectClaim(claimId: string, secret: string): { status: Claim['status']; machineId?: string; token?: string } {
  const r = db().prepare('SELECT * FROM machine_claims WHERE id = ?').get(claimId) as Row | undefined;
  if (!r || r.secret_hash !== hashToken(secret)) throw new OrgError('Unknown claim', 404);
  const c = claimFromRow(r);
  if (c.status !== 'claimed') return { status: c.status };
  const token = open(r.pending_token as string);
  db().prepare("UPDATE machine_claims SET status = 'collected', pending_token = NULL WHERE id = ?").run(claimId);
  return { status: 'collected', machineId: c.machineId!, token };
}

export function pendingClaims(): Claim[] {
  return (db().prepare("SELECT * FROM machine_claims WHERE status = 'pending' AND expires_at > ? ORDER BY created_at DESC").all(now()) as Row[]).map(claimFromRow);
}

// ── Browser sign-in from the desktop (RFC 8252 loopback redirect + PKCE) ──

const CONNECT_MINUTES = 5;

/** Only native-app loopback redirects: `http://127.0.0.1:<port>/…`, `[::1]` or `localhost`. */
export function isLoopbackRedirect(uri: string): boolean {
  try {
    const u = new URL(uri);
    return u.protocol === 'http:' && ['127.0.0.1', '[::1]', 'localhost'].includes(u.hostname) && !!u.port && !u.username && !u.password && !u.hash;
  } catch {
    return false;
  }
}

const S256 = (verifier: string) => createHash('sha256').update(verifier).digest('base64url');

/**
 * A signed-in user approved enrolling a desktop: a one-time code (5 min)
 * bound to the PKCE challenge and the loopback redirect. Returns the URL the
 * browser goes back to.
 */
export function createConnectCode(orgId: string, by: string, o: { redirectUri: string; state: string; codeChallenge: string; name?: string; tags?: string[] }): string {
  if (!isLoopbackRedirect(o.redirectUri)) throw new OrgError('The sign-in must return to this computer (a loopback address)', 400);
  if (!/^[A-Za-z0-9_-]{43,128}$/.test(o.codeChallenge)) throw new OrgError('Invalid code challenge', 400);
  if (!o.state || o.state.length > 200) throw new OrgError('Invalid state', 400);
  checkSeats(orgId);
  const code = newToken('ufop_connect', 24);
  const ts = now();
  db()
    .prepare('INSERT INTO desktop_connect_codes (code_hash, org_id, user_email, code_challenge, redirect_uri, name, tags, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(hashToken(code), orgId, by, o.codeChallenge, o.redirectUri, clip(o.name ?? ''), JSON.stringify(normalizeTags(o.tags ?? [])), ts, new Date(Date.now() + CONNECT_MINUTES * 60_000).toISOString());
  const back = new URL(o.redirectUri);
  back.searchParams.set('code', code);
  back.searchParams.set('state', o.state);
  return back.toString();
}

/** The desktop exchanges the code (once) with its PKCE verifier for an agent token. */
export function redeemConnectCode(code: string, verifier: string, redirectUri: string, info: ClientInfo, extraTags: string[] = []): { machine: Machine; token: string; by: string } {
  const r = db().prepare('SELECT * FROM desktop_connect_codes WHERE code_hash = ?').get(hashToken(code)) as Row | undefined;
  // Burn the code first (outside any transaction): every attempt uses it up,
  // so a stolen code with a wrong verifier cannot be retried.
  const burned = r && db().prepare('UPDATE desktop_connect_codes SET used_at = ? WHERE code_hash = ? AND used_at IS NULL AND expires_at > ?').run(now(), r.code_hash, now()).changes === 1;
  if (!r || !burned) throw new OrgError('The sign-in code is not valid or has expired; sign in again', 401);
  if (r.redirect_uri !== redirectUri || S256(verifier) !== r.code_challenge) throw new OrgError('The sign-in code does not belong to this request', 401);
  const name = (r.name as string) || info.name;
  const { machine, token } = tx(() => insertMachine(r.org_id as string, { ...info, name }, `sso:${r.user_email}`, [...parseJson<string[]>(r.tags, []), ...extraTags]));
  return { machine, token, by: r.user_email as string };
}

// ── Agent authentication & lifecycle ──

/** Resolve an agent bearer token; `null` for unknown, revoked or suspended machines. */
export function machineByToken(token: string): Machine | null {
  if (!token.startsWith('ufop_agent_')) return null;
  const r = db().prepare('SELECT * FROM machines WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function touchMachine(id: string, ip: string, info: ClientInfo & { jobs?: JobSummary[]; heartbeatSeconds?: number }): void {
  const sets = ['last_seen_at = ?', 'last_ip = ?'];
  const vals: unknown[] = [now(), clip(ip, 64)];
  const map: [keyof ClientInfo, string, number][] = [
    ['hostname', 'hostname', 120],
    ['os', 'os', 40],
    ['osVersion', 'os_version', 60],
    ['arch', 'arch', 20],
    ['clientVersion', 'client_version', 40],
  ];
  for (const [k, col, n] of map) if (typeof info[k] === 'string') { sets.push(`${col} = ?`); vals.push(clip(info[k], n)); }
  if (Array.isArray(info.jobs)) { sets.push('jobs = ?'); vals.push(JSON.stringify(info.jobs.slice(0, 200))); }
  if (info.heartbeatSeconds) { sets.push('heartbeat_seconds = ?'); vals.push(Math.min(3600, Math.max(15, Math.floor(info.heartbeatSeconds)))); }
  db().prepare(`UPDATE machines SET ${sets.join(', ')} WHERE id = ?`).run(...vals, id);
}

export function rotateToken(id: string): string {
  const token = newToken('ufop_agent', 32);
  db().prepare("UPDATE machines SET token_hash = ?, token_rotated_at = ? WHERE id = ? AND status != 'revoked'").run(hashToken(token), now(), id);
  return token;
}

export function setMachineStatus(id: string, status: MachineStatus): void {
  if (status === 'revoked') db().prepare("UPDATE machines SET status = 'revoked', token_hash = NULL, revoked_at = ? WHERE id = ?").run(now(), id);
  else db().prepare("UPDATE machines SET status = ? WHERE id = ? AND status != 'revoked'").run(status, id);
}

export function updateMachine(id: string, u: { name?: string; tags?: string[]; notes?: string; orgId?: string }): Machine | null {
  const m = getMachine(id);
  if (!m) return null;
  if (u.orgId && u.orgId !== m.orgId) checkSeats(u.orgId);
  db()
    .prepare('UPDATE machines SET name = ?, tags = ?, notes = ?, org_id = ? WHERE id = ?')
    .run(u.name?.trim().slice(0, 120) || m.name, JSON.stringify(u.tags ? normalizeTags(u.tags) : m.tags), u.notes?.slice(0, 2000) ?? m.notes, u.orgId ?? m.orgId, id);
  if (u.orgId && u.orgId !== m.orgId) {
    for (const t of ['backup_reports', 'backups', 'agent_commands']) db().prepare(`UPDATE ${t} SET org_id = ? WHERE machine_id = ?`).run(u.orgId, id);
  }
  return getMachine(id);
}

export function deleteMachine(id: string): boolean {
  return db().prepare('DELETE FROM machines WHERE id = ?').run(id).changes > 0;
}

export function setAppliedPolicies(id: string, applied: Record<string, number>): void {
  db().prepare('UPDATE machines SET applied_policies = ? WHERE id = ?').run(JSON.stringify(applied), id);
}

// ── Tag colors ──

export function tagColors(orgId: string): Record<string, string> {
  return Object.fromEntries((db().prepare('SELECT tag, color FROM tag_colors WHERE org_id = ?').all(orgId) as Row[]).map((r) => [r.tag as string, r.color as string]));
}

export function setTagColor(orgId: string, tag: string, color: string | null): void {
  const t = normalizeTags([tag])[0];
  if (!t) throw new OrgError('Invalid tag');
  if (!color) {
    db().prepare('DELETE FROM tag_colors WHERE org_id = ? AND tag = ?').run(orgId, t);
    return;
  }
  if (!/^#[0-9a-fA-F]{6}$/.test(color)) throw new OrgError('Colors are #rrggbb');
  db().prepare('INSERT INTO tag_colors (org_id, tag, color) VALUES (?, ?, ?) ON CONFLICT(org_id, tag) DO UPDATE SET color = excluded.color').run(orgId, t, color.toLowerCase());
}

export function allTags(orgIds: string[]): { tag: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const m of listMachines({ orgIds })) for (const t of m.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
  return [...counts.entries()].map(([tag, count]) => ({ tag, count })).sort((a, b) => a.tag.localeCompare(b.tag));
}
