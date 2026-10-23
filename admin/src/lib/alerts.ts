/**
 * Alert evaluation.
 *
 * - On every report: backup_failed, changed_files, duration_anomaly; a
 *   successful report also resolves open failed/missed alerts for that backup.
 * - On a timer (`evaluateMissed`): missed_backup — a backup with a known (or
 *   assumed) schedule that has not succeeded within interval + grace; and
 *   expected_arrival — no successful backup between the previous deadline
 *   and the latest one (local time, chosen days, plus grace).
 *
 * Acknowledged or snoozed alerts stay open but are not notified again.
 *
 * One open alert per (rule, machine, backup, kind); repeated triggers
 * re-notify only after the rule's cooldown.
 */
import { renderTemplate } from './branding';
import { getMachine, type Machine } from './db/repos/machines';
import { listBackups, listReports, type StoredReport } from './db/repos/reports';
import { insertAlert, listRules, mayNotify, openAlert, recordNotification, resolveAlert, type Alert, type AlertKind, type AlertRule } from './db/repos/alerts';
import { recentDeadlines } from './zoned-time';
import { deliverTo } from './mailer';
import { appendAudit } from './db/repos/audit';

export function inScope(rule: AlertRule, m: Machine): boolean {
  if (!rule.enabled || rule.orgId !== m.orgId) return false;
  if (rule.scope.machineIds.length && !rule.scope.machineIds.includes(m.id)) return false;
  return rule.scope.tags.every((t) => m.tags.includes(t));
}

const fp = (rule: AlertRule, machineId: string, backupId: string) => `${rule.id}:${machineId}:${backupId}:${rule.kind}`;

async function raise(rule: AlertRule, m: Machine, backupId: string, kind: AlertKind, severity: Alert['severity'], message: string, details: Record<string, unknown>): Promise<Alert> {
  const f = fp(rule, m.id, backupId);
  const existing = openAlert(f);
  const alert = existing ?? insertAlert({ orgId: m.orgId, ruleId: rule.id, machineId: m.id, backupId, kind, severity, message, details, fingerprint: f });
  const due = !existing || ((!existing.lastNotifiedAt || Date.now() - new Date(existing.lastNotifiedAt).getTime() >= rule.cooldownMinutes * 60_000) && mayNotify(existing));
  if (due && rule.channelIds.length) {
    const deliveries = await deliverTo(m.orgId, rule.channelIds, {
      ...renderTemplate(m.orgId, 'alert', {
        severity: severity === 'critical' ? 'Critical' : 'Warning',
        message,
        machine: m.name,
        backup: backupId,
        rule: rule.name,
        time: new Date().toISOString(),
        details: Object.entries(details).map(([k, v]) => `${k}: ${typeof v === 'object' ? JSON.stringify(v) : v}`).join('\n'),
      }),
      data: { event: 'alert', alertId: alert.id, kind, severity, machine: { id: m.id, name: m.name, tags: m.tags }, backupId, rule: { id: rule.id, name: rule.name }, details },
    });
    recordNotification(alert.id, deliveries);
  }
  if (!existing) appendAudit({ eventType: 'alert.raise', severity: severity === 'critical' ? 'error' : 'warning', orgId: m.orgId, description: message, resourceType: 'alert', resourceId: alert.id });
  return alert;
}

async function resolve(rule: AlertRule, m: Machine, backupId: string, reason: string) {
  const a = openAlert(fp(rule, m.id, backupId));
  if (!a) return;
  resolveAlert(a.id);
  if (rule.params.notifyOnResolve && rule.channelIds.length) {
    recordNotification(a.id, await deliverTo(m.orgId, rule.channelIds, { ...renderTemplate(m.orgId, 'alert_resolved', { message: a.message, reason }), data: { event: 'alert_resolved', alertId: a.id, kind: a.kind } }));
  }
}

function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/** Duration statistics of earlier successful runs (excluding `r`). */
export function durationBaseline(r: StoredReport, samples = 20): { median: number; mean: number; std: number; n: number } {
  const prev = listReports({ orgIds: [r.orgId], machineId: r.machineId, backupId: r.backupId, limit: samples + 1 })
    .filter((x) => x.id !== r.id && x.operation === 'backup' && (x.status === 'success' || x.status === 'warning') && x.durationMs > 0)
    .slice(0, samples)
    .map((x) => x.durationMs);
  if (!prev.length) return { median: 0, mean: 0, std: 0, n: 0 };
  const mean = prev.reduce((a, b) => a + b, 0) / prev.length;
  const std = Math.sqrt(prev.reduce((a, b) => a + (b - mean) ** 2, 0) / prev.length);
  return { median: median(prev), mean, std, n: prev.length };
}

export async function evaluateReport(machine: Machine, r: StoredReport): Promise<Alert[]> {
  if (r.operation !== 'backup') return [];
  const raised: Alert[] = [];
  const ok = r.status === 'success' || r.status === 'warning';
  for (const rule of listRules(machine.orgId).filter((x) => inScope(x, machine))) {
    const p = rule.params;
    if (rule.kind === 'backup_failed') {
      const bad = r.status === 'error' || (p.includeWarnings && r.status === 'warning');
      if (bad) raised.push(await raise(rule, machine, r.backupId, 'backup_failed', r.status === 'error' ? 'critical' : 'warning', `Backup "${r.backupName}" on ${machine.name} ${r.status === 'error' ? 'failed' : 'finished with warnings'}`, { errors: r.errors, warnings: r.warnings, message: r.message.slice(0, 500) }));
      else await resolve(rule, machine, r.backupId, 'the next backup succeeded');
    } else if (rule.kind === 'missed_backup' || rule.kind === 'expected_arrival') {
      if (ok) await resolve(rule, machine, r.backupId, 'the backup ran');
    } else if (rule.kind === 'changed_files' && ok) {
      const changed = r.filesAdded + r.filesModified + r.filesDeleted;
      const pct = r.filesExamined ? (changed / r.filesExamined) * 100 : 0;
      const hits: string[] = [];
      if (p.maxChangedFiles && changed > p.maxChangedFiles) hits.push(`${changed} files changed (limit ${p.maxChangedFiles})`);
      if (p.maxChangedPercent && pct > p.maxChangedPercent) hits.push(`${pct.toFixed(1)}% of files changed (limit ${p.maxChangedPercent}%)`);
      if (p.maxDeletedFiles && r.filesDeleted > p.maxDeletedFiles) hits.push(`${r.filesDeleted} files deleted (limit ${p.maxDeletedFiles})`);
      if (hits.length) raised.push(await raise(rule, machine, r.backupId, 'changed_files', 'warning', `Unusual change volume in "${r.backupName}" on ${machine.name}: ${hits.join('; ')}`, { added: r.filesAdded, modified: r.filesModified, deleted: r.filesDeleted, examined: r.filesExamined, reportId: r.id }));
      else await resolve(rule, machine, r.backupId, 'change volume back to normal');
    } else if (rule.kind === 'duration_anomaly' && ok && r.durationMs > 0) {
      const b = durationBaseline(r);
      if (b.n < (p.minSamples ?? 5)) continue;
      const factor = p.factor ?? 2;
      const k = p.stdDevs ?? 3;
      const long = r.durationMs > Math.max(b.median * factor, b.mean + k * b.std);
      const short = !!p.detectShort && r.durationMs < b.median / factor;
      const fmt = (ms: number) => `${Math.round(ms / 1000)}s`;
      if (long || short) raised.push(await raise(rule, machine, r.backupId, 'duration_anomaly', 'warning', `"${r.backupName}" on ${machine.name} took ${fmt(r.durationMs)}, usually ${fmt(b.median)}`, { durationMs: r.durationMs, medianMs: Math.round(b.median), meanMs: Math.round(b.mean), stdMs: Math.round(b.std), samples: b.n, direction: long ? 'longer' : 'shorter', reportId: r.id }));
      else await resolve(rule, machine, r.backupId, 'duration back to normal');
    }
  }
  return raised;
}

/** Timer job: raise missed-backup alerts across all organizations. */
export async function evaluateMissed(at = Date.now()): Promise<Alert[]> {
  const raised: Alert[] = [];
  const rules = listRules().filter((r) => (r.kind === 'missed_backup' || r.kind === 'expected_arrival') && r.enabled);
  const byOrg = new Map<string, AlertRule[]>();
  for (const r of rules) byOrg.set(r.orgId, [...(byOrg.get(r.orgId) ?? []), r]);
  for (const [orgId, orgRules] of byOrg) {
    for (const b of listBackups([orgId])) {
      const m = getMachine(b.machineId);
      if (!m || m.status !== 'active') continue;
      for (const rule of orgRules.filter((r) => inScope(r, m))) {
        if (rule.kind === 'expected_arrival') {
          const p = rule.params;
          if (p.backupIds?.length && !p.backupIds.includes(b.backupId)) continue;
          const tz = p.timeZone || 'UTC';
          // The latest deadline whose grace has passed, and the one before it.
          const [latest, previous] = recentDeadlines(new Date(at - (p.graceMinutes ?? 0) * 60_000), { expectedBy: p.expectedBy ?? '00:00', days: p.days ?? [], timeZone: tz });
          if (!latest || !previous) continue;
          // Machines registered after the window opened get their first deadline next time.
          if (new Date(m.registeredAt).getTime() > previous.getTime()) continue;
          const last = b.lastSuccessAt ? new Date(b.lastSuccessAt).getTime() : 0;
          if (last <= previous.getTime()) {
            raised.push(await raise(rule, m, b.backupId, 'expected_arrival', 'critical', `Backup "${b.name}" on ${m.name} did not arrive by ${p.expectedBy} (${tz})`, { expectedBy: p.expectedBy, timeZone: tz, deadline: latest.toISOString(), lastSuccessAt: b.lastSuccessAt }));
          }
          continue;
        }
        const intervalMin = b.scheduleIntervalMinutes ?? ((rule.params.defaultIntervalHours ?? 0) * 60 || null);
        if (!intervalMin) continue;
        const since = b.lastSuccessAt ?? b.lastRunAt ?? m.registeredAt;
        const overdue = at - new Date(since).getTime() - (intervalMin + (rule.params.graceMinutes ?? 60)) * 60_000;
        if (overdue > 0) {
          raised.push(await raise(rule, m, b.backupId, 'missed_backup', 'critical', `Backup "${b.name}" on ${m.name} has not succeeded since ${since}`, { expectedEveryMinutes: intervalMin, lastSuccessAt: b.lastSuccessAt, overdueMinutes: Math.round(overdue / 60_000) }));
        }
      }
    }
  }
  return raised;
}
