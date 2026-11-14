/**
 * The report catalog: named, parameterized reports over the organization's
 * data, rendered as CSV, JSON or a plain-text table. Runs, schedules and
 * delivery live in `report-runner.ts`.
 */
import { db, type Row } from './db';
import { listMachines } from './db/repos/machines';
import { listBackups, listReports } from './db/repos/reports';
import { membersOf } from './db/repos/orgs';
import { getUser } from './db/repos/users';
import { hasPasskeys } from './db/repos/passkeys';
import { listApiTokens } from './db/repos/api-tokens';
import { roleSources } from './db/repos/orgs';
import { lastActivity } from './retirement';
import type { Permission } from './types/auth';

export type Cell = string | number | boolean | null;

export interface ReportResult {
  columns: { key: string; label: string }[];
  rows: Record<string, Cell>[];
  /** One line for emails and the run list. */
  summary: string;
}

export interface ReportDefinition {
  id: string;
  name: string;
  description: string;
  category: 'Backups' | 'Machines' | 'People' | 'Audit';
  permission: Permission;
  /** Look-back window in days (default; 1–365). */
  defaultDays: number;
  run(orgId: string, days: number): ReportResult;
}

const since = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();
const machineNames = (orgId: string) => new Map(listMachines({ orgIds: [orgId] }).map((m) => [m.id, m.name]));
const pct = (n: number, d: number) => (d ? Math.round((n / d) * 1000) / 10 : null);

export const REPORTS: ReportDefinition[] = [
  {
    id: 'backup_status',
    name: 'Backup status',
    description: 'Every backup job: last result, last success and success rate over the period.',
    category: 'Backups',
    permission: 'fleet.read',
    defaultDays: 30,
    run(orgId, days) {
      const names = machineNames(orgId);
      const runs = listReports({ orgIds: [orgId], since: since(days), limit: 5000 }).filter((r) => r.operation === 'backup');
      const rows = listBackups([orgId]).map((b) => {
        const mine = runs.filter((r) => r.machineId === b.machineId && r.backupId === b.backupId);
        const ok = mine.filter((r) => r.status === 'success' || r.status === 'warning').length;
        return {
          machine: names.get(b.machineId) ?? b.machineId,
          backup: b.name,
          lastStatus: b.lastStatus,
          lastRunAt: b.lastRunAt,
          lastSuccessAt: b.lastSuccessAt,
          runs: mine.length,
          successRate: pct(ok, mine.length),
          backendSize: b.backendSize,
        };
      });
      const failing = rows.filter((r) => r.lastStatus === 'error').length;
      return {
        columns: [
          { key: 'machine', label: 'Machine' },
          { key: 'backup', label: 'Backup' },
          { key: 'lastStatus', label: 'Last result' },
          { key: 'lastRunAt', label: 'Last run' },
          { key: 'lastSuccessAt', label: 'Last success' },
          { key: 'runs', label: 'Runs' },
          { key: 'successRate', label: 'Success %' },
          { key: 'backendSize', label: 'Stored bytes' },
        ],
        rows,
        summary: `${rows.length} backup(s), ${failing} failing at their last run`,
      };
    },
  },
  {
    id: 'failed_backups',
    name: 'Failed backups',
    description: 'Every failed run in the period, with its message.',
    category: 'Backups',
    permission: 'fleet.read',
    defaultDays: 7,
    run(orgId, days) {
      const names = machineNames(orgId);
      const rows = listReports({ orgIds: [orgId], status: 'error', since: since(days), limit: 5000 }).map((r) => ({
        finishedAt: r.finishedAt,
        machine: names.get(r.machineId) ?? r.machineId,
        backup: r.backupName,
        operation: r.operation,
        errors: r.errors,
        message: r.message,
      }));
      return {
        columns: [
          { key: 'finishedAt', label: 'Finished' },
          { key: 'machine', label: 'Machine' },
          { key: 'backup', label: 'Backup' },
          { key: 'operation', label: 'Operation' },
          { key: 'errors', label: 'Errors' },
          { key: 'message', label: 'Message' },
        ],
        rows,
        summary: `${rows.length} failed run(s) in ${days} day(s)`,
      };
    },
  },
  {
    id: 'storage_growth',
    name: 'Storage growth',
    description: 'Stored size per backup at the start and end of the period.',
    category: 'Backups',
    permission: 'fleet.read',
    defaultDays: 30,
    run(orgId, days) {
      const names = machineNames(orgId);
      const runs = listReports({ orgIds: [orgId], since: since(days), limit: 5000 })
        .filter((r) => r.operation === 'backup' && r.backendSize > 0)
        .reverse();
      const by = new Map<string, { machine: string; backup: string; first: number; last: number }>();
      for (const r of runs) {
        const k = `${r.machineId}/${r.backupId}`;
        const e = by.get(k) ?? { machine: names.get(r.machineId) ?? r.machineId, backup: r.backupName, first: r.backendSize, last: r.backendSize };
        e.last = r.backendSize;
        by.set(k, e);
      }
      const rows = [...by.values()].map((e) => ({ ...e, change: e.last - e.first, changePct: pct(e.last - e.first, e.first) })).sort((a, b) => b.change - a.change);
      return {
        columns: [
          { key: 'machine', label: 'Machine' },
          { key: 'backup', label: 'Backup' },
          { key: 'first', label: 'Start bytes' },
          { key: 'last', label: 'End bytes' },
          { key: 'change', label: 'Change bytes' },
          { key: 'changePct', label: 'Change %' },
        ],
        rows,
        summary: `${rows.length} backup(s); total change ${rows.reduce((a, r) => a + r.change, 0)} bytes`,
      };
    },
  },
  {
    id: 'machine_inventory',
    name: 'Machine inventory',
    description: 'Every enrolled machine with its system, client version, tags and last contact.',
    category: 'Machines',
    permission: 'fleet.read',
    defaultDays: 30,
    run(orgId) {
      const rows = listMachines({ orgIds: [orgId] }).map((m) => ({
        name: m.name,
        hostname: m.hostname,
        os: `${m.os} ${m.osVersion}`.trim(),
        clientVersion: m.clientVersion,
        status: m.status,
        connectivity: m.connectivity,
        lastSeenAt: m.lastSeenAt,
        tags: m.tags.join(' '),
      }));
      return {
        columns: [
          { key: 'name', label: 'Name' },
          { key: 'hostname', label: 'Hostname' },
          { key: 'os', label: 'System' },
          { key: 'clientVersion', label: 'Client' },
          { key: 'status', label: 'Status' },
          { key: 'connectivity', label: 'Connectivity' },
          { key: 'lastSeenAt', label: 'Last seen' },
          { key: 'tags', label: 'Tags' },
        ],
        rows,
        summary: `${rows.length} machine(s), ${rows.filter((r) => r.connectivity === 'online').length} online`,
      };
    },
  },
  {
    id: 'stale_machines',
    name: 'Machines not seen',
    description: 'Active machines that have not contacted the console within the period.',
    category: 'Machines',
    permission: 'fleet.read',
    defaultDays: 7,
    run(orgId, days) {
      const cutoff = Date.now() - days * 86_400_000;
      const rows = listMachines({ orgIds: [orgId], status: 'active' })
        .filter((m) => !m.lastSeenAt || new Date(m.lastSeenAt).getTime() < cutoff)
        .map((m) => ({ name: m.name, hostname: m.hostname, lastSeenAt: m.lastSeenAt, lastIp: m.lastIp, tags: m.tags.join(' ') }));
      return {
        columns: [
          { key: 'name', label: 'Name' },
          { key: 'hostname', label: 'Hostname' },
          { key: 'lastSeenAt', label: 'Last seen' },
          { key: 'lastIp', label: 'Last address' },
          { key: 'tags', label: 'Tags' },
        ],
        rows,
        summary: `${rows.length} machine(s) silent for ${days}+ day(s)`,
      };
    },
  },
  {
    id: 'access_review',
    name: 'Access review',
    description: 'Every member: effective role and where it comes from, two-factor, API tokens and last activity.',
    category: 'People',
    permission: 'users.read',
    defaultDays: 90,
    run(orgId, days) {
      const cutoff = Date.now() - days * 86_400_000;
      const tokens = listApiTokens(orgId, null).filter((t) => !t.revokedAt && new Date(t.expiresAt) > new Date());
      const rows = membersOf(orgId).flatMap((m) => {
        const u = getUser(m.userId);
        if (!u) return [];
        const applying = roleSources(m.userId, orgId).filter((s) => s.applies);
        const active = lastActivity(m.userId, m.createdAt);
        return [
          {
            email: u.email,
            name: u.name,
            role: m.role,
            grants: applying.map((s) => (s.via === 'membership' ? `${s.role} (membership${s.inherited ? `, ${s.orgName}` : ''})` : `${s.role} (group ${s.groupName})`)).join('; '),
            accountActive: u.isActive,
            twoFactor: u.mfaEnabled || hasPasskeys(u.id),
            apiTokens: tokens.filter((t) => t.userId === u.id).length,
            lastActivity: active ? new Date(active).toISOString() : null,
            inactive: active < cutoff,
            membershipEnds: m.expiresAt,
          },
        ];
      });
      return {
        columns: [
          { key: 'email', label: 'Email' },
          { key: 'name', label: 'Name' },
          { key: 'role', label: 'Direct role' },
          { key: 'grants', label: 'Applying grants' },
          { key: 'accountActive', label: 'Active' },
          { key: 'twoFactor', label: 'Two-factor' },
          { key: 'apiTokens', label: 'API tokens' },
          { key: 'lastActivity', label: 'Last activity' },
          { key: 'inactive', label: `Inactive ${days}d` },
          { key: 'membershipEnds', label: 'Membership ends' },
        ],
        rows,
        summary: `${rows.length} member(s); ${rows.filter((r) => !r.twoFactor).length} without two-factor; ${rows.filter((r) => r.inactive).length} inactive for ${days}+ days`,
      };
    },
  },
  {
    id: 'audit_summary',
    name: 'Audit summary',
    description: 'Audit events in the period by type and severity.',
    category: 'Audit',
    permission: 'audit.read',
    defaultDays: 7,
    run(orgId, days) {
      const rows = (
        db()
          .prepare('SELECT event_type, severity, COUNT(*) AS n, MAX(timestamp) AS last FROM audit_entries WHERE org_id = ? AND timestamp >= ? GROUP BY event_type, severity ORDER BY n DESC, event_type')
          .all(orgId, since(days)) as Row[]
      ).map((r) => ({ eventType: r.event_type as string, severity: r.severity as string, count: Number(r.n), last: r.last as string }));
      return {
        columns: [
          { key: 'eventType', label: 'Event' },
          { key: 'severity', label: 'Severity' },
          { key: 'count', label: 'Count' },
          { key: 'last', label: 'Last' },
        ],
        rows,
        summary: `${rows.reduce((a, r) => a + r.count, 0)} event(s); ${rows.filter((r) => r.severity === 'critical' || r.severity === 'error').reduce((a, r) => a + r.count, 0)} error or critical`,
      };
    },
  },
];

export function reportDefinition(id: string): ReportDefinition | null {
  return REPORTS.find((r) => r.id === id) ?? null;
}

export type ReportFormat = 'csv' | 'json' | 'text';

const csvCell = (v: Cell) => {
  const s = v === null ? '' : String(v);
  // Quote when needed; neutralize spreadsheet formulas.
  const safe = /^[=+\-@\t\r]/.test(s) ? `'${s}` : s;
  return /[",\n\r]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe;
};

export function renderReport(r: ReportResult, format: ReportFormat): string {
  if (format === 'json') return JSON.stringify({ summary: r.summary, columns: r.columns, rows: r.rows }, null, 2);
  if (format === 'csv') return [r.columns.map((c) => csvCell(c.label)).join(','), ...r.rows.map((row) => r.columns.map((c) => csvCell(row[c.key] ?? null)).join(','))].join('\r\n') + '\r\n';
  // Plain-text table (emails, chat): column widths capped, at most 50 rows.
  const shown = r.rows.slice(0, 50);
  const text = (v: Cell) => (v === null ? '' : String(v)).replace(/\s+/g, ' ');
  const width = r.columns.map((c) => Math.min(40, Math.max(c.label.length, ...shown.map((row) => text(row[c.key] ?? null).length))));
  const line = (cells: string[]) =>
    cells
      .map((v, i) => (v.length > width[i] ? `${v.slice(0, width[i] - 1)}…` : v.padEnd(width[i])))
      .join('  ')
      .trimEnd();
  return [
    r.summary,
    '',
    line(r.columns.map((c) => c.label)),
    line(width.map((w) => '-'.repeat(w))),
    ...shown.map((row) => line(r.columns.map((c) => text(row[c.key] ?? null)))),
    ...(r.rows.length > shown.length ? [`… and ${r.rows.length - shown.length} more row(s) in the attachment`] : []),
  ].join('\n');
}
