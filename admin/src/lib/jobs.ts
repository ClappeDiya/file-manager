/**
 * Periodic console work, started once per server process from
 * src/instrumentation.ts: missed-backup alerts, offboarding purges, membership
 * expiry and removed-member purges, retiring inactive members and API tokens, a daily signed audit checkpoint and
 * scheduled AI summaries.
 */
import { evaluateMissed } from './alerts';
import { purgeOffboarded } from './db/repos/orgs';
import { anchorAudit, appendAudit, listAnchors } from './db/repos/audit';
import { runScheduledSummaries } from './ai';
import { logger } from './utils/logger';
import { startDelivery } from './delivery';
import { startSiemForwarding } from './siem';
import { retireInactive } from './retirement';
import { expireMemberships, purgeRemovedMembers } from './offboarding';
import { applyRetention } from './retention';
import { runDueSchedules } from './report-runner';

let started = false;

export async function runJobsOnce(at = new Date()): Promise<{ missed: number; purged: string[]; retired: number; anchored: boolean; summaries: number }> {
  const missed = (await evaluateMissed(at.getTime())).length;
  const purged = purgeOffboarded(at);
  for (const id of purged) appendAudit({ eventType: 'org.purge', severity: 'critical', description: `Deleted offboarded organization ${id} and its data`, resourceType: 'organization', resourceId: id });
  expireMemberships(at);
  purgeRemovedMembers(at);
  const r = retireInactive(at);
  const retired = r.members.length + r.tokens.length;
  const last = listAnchors()[0];
  const anchored = !last || at.getTime() - new Date(last.createdAt).getTime() > 86_400_000 ? anchorAudit() !== null : false;
  try {
    applyRetention(at);
  } catch (e) {
    // A chain that does not verify is never pruned; the alert is in the log.
    logger.error('retention.failed', { error: e instanceof Error ? e.message : String(e) });
  }
  const summaries = await runScheduledSummaries(at);
  await runDueSchedules(at);
  return { missed, purged, retired, anchored, summaries };
}

export function startJobs(intervalMs = 5 * 60_000): void {
  if (started || process.env.ADMIN_DISABLE_JOBS === '1') return;
  started = true;
  startDelivery();
  startSiemForwarding();
  const tick = () =>
    runJobsOnce().catch((e) => logger.error('jobs.failed', { error: e instanceof Error ? e.message : String(e) }));
  setTimeout(tick, 30_000).unref?.();
  setInterval(tick, intervalMs).unref?.();
}
