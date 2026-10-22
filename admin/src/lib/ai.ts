/**
 * AI backup analysis with Claude: on-demand analysis of the fleet (or one
 * machine) and scheduled summaries delivered to notification channels.
 *
 * The prompt carries a compact snapshot of machines, backups, recent runs and
 * open alerts — no file paths beyond what reports already contain, and no
 * credentials. Each organization brings its own API key (sealed at rest).
 */
import Anthropic from '@anthropic-ai/sdk';
import { db, newId, now, parseJson, type Row } from './db/index';
import { openOpt, sealOpt } from './crypto';
import { getOrg, listOrgs, OrgError } from './db/repos/orgs';
import { listMachines } from './db/repos/machines';
import { listBackups, listReports } from './db/repos/reports';
import { listAlerts } from './db/repos/alerts';
import { deliverTo } from './mailer';
import { appendAudit } from './db/repos/audit';

export const DEFAULT_MODEL = 'claude-opus-5-5';

export interface AiSettings {
  orgId: string;
  enabled: boolean;
  model: string;
  instructions: string;
  schedule: 'off' | 'daily' | 'weekly';
  channelIds: string[];
  hasApiKey: boolean;
  apiBase: string;
  lastRunAt: string | null;
}

export function getAiSettings(orgId: string): AiSettings {
  const r = db().prepare('SELECT * FROM ai_settings WHERE org_id = ?').get(orgId) as Row | undefined;
  return {
    orgId,
    enabled: Boolean(r?.enabled),
    model: (r?.model as string) || DEFAULT_MODEL,
    instructions: (r?.instructions as string) ?? '',
    schedule: ((r?.schedule as string) || 'off') as AiSettings['schedule'],
    channelIds: parseJson<string[]>(r?.channel_ids, []),
    hasApiKey: !!r?.api_key_ciphertext,
    apiBase: (r?.api_base as string) ?? '',
    lastRunAt: (r?.last_run_at as string) ?? null,
  };
}

export interface AiSettingsInput {
  enabled?: boolean;
  model?: string;
  instructions?: string;
  schedule?: AiSettings['schedule'];
  channelIds?: string[];
  /** undefined keeps the stored key, null/'' removes it. */
  apiKey?: string | null;
  apiBase?: string;
}

export function setAiSettings(orgId: string, s: AiSettingsInput): AiSettings {
  const cur = getAiSettings(orgId);
  if (s.schedule && !['off', 'daily', 'weekly'].includes(s.schedule)) throw new OrgError('Schedule is off, daily or weekly');
  if (s.instructions !== undefined && s.instructions.length > 4000) throw new OrgError('Custom instructions are limited to 4000 characters');
  if (s.model !== undefined && !/^[a-z0-9.-]{3,64}$/.test(s.model)) throw new OrgError('Invalid model id');
  if (s.apiBase) {
    try {
      if (!['https:', 'http:'].includes(new URL(s.apiBase).protocol)) throw new Error();
    } catch {
      throw new OrgError('The API base must be an http(s) URL');
    }
  }
  const keyCipher = s.apiKey === undefined ? (db().prepare('SELECT api_key_ciphertext k FROM ai_settings WHERE org_id = ?').get(orgId) as Row | undefined)?.k ?? null : sealOpt(s.apiKey);
  db()
    .prepare(`INSERT INTO ai_settings (org_id, enabled, model, instructions, schedule, channel_ids, api_key_ciphertext, api_base, last_run_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
              ON CONFLICT(org_id) DO UPDATE SET enabled = excluded.enabled, model = excluded.model, instructions = excluded.instructions, schedule = excluded.schedule,
              channel_ids = excluded.channel_ids, api_key_ciphertext = excluded.api_key_ciphertext, api_base = excluded.api_base`)
    .run(orgId, (s.enabled ?? cur.enabled) ? 1 : 0, s.model || cur.model, s.instructions ?? cur.instructions, s.schedule ?? cur.schedule, JSON.stringify(s.channelIds ?? cur.channelIds), keyCipher, s.apiBase ?? cur.apiBase, cur.lastRunAt);
  return getAiSettings(orgId);
}

/** Compact, path-light snapshot of an organization's backup state. */
export function fleetSnapshot(orgId: string, o: { machineId?: string; days?: number } = {}) {
  const days = o.days ?? 7;
  const machines = listMachines({ orgIds: [orgId] }).filter((m) => !o.machineId || m.id === o.machineId);
  const ids = machines.map((m) => m.id);
  const backups = listBackups([orgId], ids);
  const since = new Date(Date.now() - days * 86_400_000).toISOString();
  const reports = listReports({ orgIds: [orgId], machineIds: ids, since, limit: 400 });
  return {
    generatedAt: now(),
    periodDays: days,
    organization: getOrg(orgId)?.name ?? orgId,
    machines: machines.map((m) => ({ id: m.id, name: m.name, os: `${m.os} ${m.osVersion}`.trim(), clientVersion: m.clientVersion, tags: m.tags, connectivity: m.connectivity, lastSeenAt: m.lastSeenAt, status: m.status })),
    backups: backups.map((b) => ({ machine: machines.find((m) => m.id === b.machineId)?.name, backup: b.name, lastStatus: b.lastStatus, lastRunAt: b.lastRunAt, lastSuccessAt: b.lastSuccessAt, everyMinutes: b.scheduleIntervalMinutes, sourceBytes: b.sourceSize, storedBytes: b.backendSize, versions: b.versionCount })),
    runs: reports.map((r) => ({ machine: machines.find((m) => m.id === r.machineId)?.name, backup: r.backupName, op: r.operation, status: r.status, at: r.finishedAt, seconds: Math.round(r.durationMs / 1000), added: r.filesAdded, modified: r.filesModified, deleted: r.filesDeleted, uploadedBytes: r.bytesUploaded, warnings: r.warnings, errors: r.errors, message: r.status === 'success' ? undefined : r.message.slice(0, 300) })),
    openAlerts: listAlerts({ orgIds: [orgId], open: true, machineIds: ids }).map((a) => ({ kind: a.kind, severity: a.severity, message: a.message, since: a.createdAt })),
  };
}

const SYSTEM = `You are a backup operations analyst for the UFOP backup console. You receive a JSON snapshot of an organization's backup clients, backup jobs, recent runs and open alerts.

Write a concise report for an IT administrator:
1. Overall health in one or two sentences.
2. Problems that need action, most urgent first: failing or missed backups, machines offline, unusual change volumes (possible ransomware or mass deletion), durations or storage growing abnormally, outdated clients. For each, say what you see in the data and what to do.
3. Notable trends worth watching.

Base every statement on the snapshot; say so when data is missing rather than guessing. Use plain language and short Markdown sections. Do not invent machine or backup names.`;

export interface AiReport {
  id: string;
  orgId: string;
  kind: 'analysis' | 'summary';
  scope: Record<string, unknown>;
  model: string;
  inputTokens: number;
  outputTokens: number;
  content: string;
  error: string | null;
  createdBy: string;
  createdAt: string;
}

function reportFromRow(r: Row): AiReport {
  return { id: r.id as string, orgId: r.org_id as string, kind: r.kind as AiReport['kind'], scope: parseJson(r.scope, {}), model: r.model as string, inputTokens: Number(r.input_tokens), outputTokens: Number(r.output_tokens), content: r.content as string, error: (r.error as string) ?? null, createdBy: r.created_by as string, createdAt: r.created_at as string };
}

export function listAiReports(orgId: string, limit = 20): AiReport[] {
  return (db().prepare('SELECT * FROM ai_reports WHERE org_id = ? ORDER BY created_at DESC LIMIT ?').all(orgId, limit) as Row[]).map(reportFromRow);
}

function client(s: AiSettings): Anthropic {
  const key = openOpt((db().prepare('SELECT api_key_ciphertext k FROM ai_settings WHERE org_id = ?').get(s.orgId) as Row | undefined)?.k);
  if (!key) throw new OrgError('Add an Anthropic API key in AI settings first', 409);
  return new Anthropic({ apiKey: key, baseURL: s.apiBase || undefined, maxRetries: 2, timeout: 10 * 60_000 });
}

/** Run an analysis now and store the result. */
export async function analyze(orgId: string, o: { kind: AiReport['kind']; machineId?: string; question?: string; days?: number; by: string }): Promise<AiReport> {
  const s = getAiSettings(orgId);
  if (!s.enabled) throw new OrgError('AI analysis is turned off for this organization', 409);
  const snapshot = fleetSnapshot(orgId, { machineId: o.machineId, days: o.days });
  const scope = { machineId: o.machineId ?? null, days: snapshot.periodDays, question: o.question ?? null };
  const user = [
    s.instructions ? `Instructions from the organization's administrators:\n${s.instructions}` : '',
    o.question ? `Question: ${o.question.slice(0, 2000)}` : o.kind === 'summary' ? `Write the ${s.schedule === 'weekly' ? 'weekly' : 'daily'} summary.` : 'Analyze the current state.',
    `Snapshot:\n${JSON.stringify(snapshot)}`,
  ]
    .filter(Boolean)
    .join('\n\n');
  const id = newId('air');
  const store = (content: string, error: string | null, usage?: { input_tokens: number; output_tokens: number }) => {
    db()
      .prepare('INSERT INTO ai_reports (id, org_id, kind, scope, model, input_tokens, output_tokens, content, error, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
      .run(id, orgId, o.kind, JSON.stringify(scope), s.model, usage?.input_tokens ?? 0, usage?.output_tokens ?? 0, content, error, o.by, now());
    return reportFromRow(db().prepare('SELECT * FROM ai_reports WHERE id = ?').get(id) as Row);
  };
  try {
    const stream = client(s).beta.messages.stream({
      model: s.model,
      max_tokens: 16000,
      system: SYSTEM,
      thinking: { type: 'adaptive' },
      output_config: { effort: 'medium' },
      // Re-run on a fallback model if a safety classifier declines.
      betas: ['server-side-fallback-2026-07-01'],
      fallbacks: 'default',
      messages: [{ role: 'user', content: user }],
    });
    const msg = await stream.finalMessage();
    if (msg.stop_reason === 'refusal') return store('', `The model declined this request${msg.stop_details?.category ? ` (${msg.stop_details.category})` : ''}`, msg.usage);
    const text = msg.content.flatMap((b) => (b.type === 'text' ? [b.text] : [])).join('\n').trim();
    if (!text) return store('', msg.stop_reason === 'max_tokens' ? 'The answer was cut off (output limit reached)' : 'The model returned no text', msg.usage);
    return store(text, null, msg.usage);
  } catch (e) {
    if (e instanceof OrgError) throw e;
    const message = e instanceof Anthropic.AuthenticationError ? 'The Anthropic API key was rejected' : e instanceof Anthropic.RateLimitError ? 'Rate limited by the Anthropic API; try again later' : e instanceof Anthropic.APIError ? `Anthropic API error ${e.status ?? ''}: ${e.message}` : e instanceof Error ? e.message : String(e);
    return store('', message);
  }
}

/** Timer job: summaries for organizations whose daily/weekly schedule is due. Returns how many ran. */
export async function runScheduledSummaries(at = new Date()): Promise<number> {
  let n = 0;
  for (const org of listOrgs()) {
    const s = getAiSettings(org.id);
    if (!s.enabled || s.schedule === 'off' || !s.hasApiKey || org.status !== 'active') continue;
    const every = s.schedule === 'daily' ? 86_400_000 : 7 * 86_400_000;
    if (s.lastRunAt && at.getTime() - new Date(s.lastRunAt).getTime() < every - 60_000) continue;
    db().prepare('UPDATE ai_settings SET last_run_at = ? WHERE org_id = ?').run(at.toISOString(), org.id);
    const r = await analyze(org.id, { kind: 'summary', days: s.schedule === 'daily' ? 1 : 7, by: 'scheduler' });
    n++;
    if (r.content && s.channelIds.length) await deliverTo(org.id, s.channelIds, { subject: `[UFOP] ${s.schedule === 'daily' ? 'Daily' : 'Weekly'} backup summary — ${org.name}`, text: r.content, data: { event: 'ai_summary', reportId: r.id } });
    appendAudit({ eventType: 'ai.summary', orgId: org.id, description: r.error ? `Scheduled AI summary failed: ${r.error}` : 'Scheduled AI summary created', resourceType: 'ai_report', resourceId: r.id, severity: r.error ? 'warning' : 'info' });
  }
  return n;
}
