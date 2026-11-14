/**
 * AI insights against a local server that speaks the Anthropic Messages API
 * (streaming SSE), through the real SDK client the console builds.
 */
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import http from 'http';
import type { AddressInfo } from 'net';
import { db } from '@/lib/db';
import { createOrg } from '@/lib/db/repos/orgs';
import { createLink, registerWithToken } from '@/lib/db/repos/machines';
import { ingestReport, normalizeReport } from '@/lib/db/repos/reports';
import { createChannel } from '@/lib/db/repos/channels';
import { createCredential } from '@/lib/db/repos/fleet-policies';
import { runScheduledSummaries } from '@/lib/ai';
import { listAudit, verifyAudit } from '@/lib/db/repos/audit';
import { json, makeUser, req } from './helpers';
import * as settings from '@/app/api/ai/settings/route';
import * as analyzeRoute from '@/app/api/ai/analyze/route';
import * as transcript from '@/app/api/ai/transcripts/[id]/route';

type Mode = { kind: 'text'; text: string } | { kind: 'refusal' } | { kind: 'status'; status: number; type: string } | { kind: 'max_tokens' };
let mode: Mode = { kind: 'text', text: 'All good.' };
let calls: { path: string; headers: http.IncomingHttpHeaders; body: Record<string, unknown> }[] = [];
let hooks: Record<string, unknown>[] = [];
let api: http.Server;
let base = '';

const sse = (event: string, data: unknown) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;

function stream(rs: http.ServerResponse, model: string, text: string | null, stop: string, stopDetails: unknown = null) {
  rs.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', 'request-id': 'req_test' });
  rs.write(sse('message_start', { type: 'message_start', message: { id: 'msg_test', type: 'message', role: 'assistant', model, content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 1234, output_tokens: 1 } } }));
  // Adaptive thinking comes first; it must not end up in the report.
  rs.write(sse('content_block_start', { type: 'content_block_start', index: 0, content_block: { type: 'thinking', thinking: '', signature: '' } }));
  rs.write(sse('content_block_delta', { type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'PRIVATE-REASONING looking at the snapshot' } }));
  rs.write(sse('content_block_delta', { type: 'content_block_delta', index: 0, delta: { type: 'signature_delta', signature: 'sig' } }));
  rs.write(sse('content_block_stop', { type: 'content_block_stop', index: 0 }));
  if (text !== null) {
    rs.write(sse('content_block_start', { type: 'content_block_start', index: 1, content_block: { type: 'text', text: '' } }));
    // Several deltas per block, as the API sends them.
    for (const part of text.match(/[\s\S]{1,7}/g) ?? []) rs.write(sse('content_block_delta', { type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: part } }));
    rs.write(sse('content_block_stop', { type: 'content_block_stop', index: 1 }));
  }
  rs.write(sse('message_delta', { type: 'message_delta', delta: { stop_reason: stop, stop_sequence: null, stop_details: stopDetails }, usage: { output_tokens: 321 } }));
  rs.write(sse('message_stop', { type: 'message_stop' }));
  rs.end();
}

beforeAll(async () => {
  api = http.createServer((rq, rs) => {
    let raw = '';
    rq.on('data', (c) => (raw += c));
    rq.on('end', () => {
      if (rq.url === '/hook') {
        hooks.push(JSON.parse(raw));
        rs.end('ok');
        return;
      }
      const body = JSON.parse(raw || '{}');
      calls.push({ path: rq.url ?? '', headers: rq.headers, body });
      if (mode.kind === 'status') {
        rs.writeHead(mode.status, { 'content-type': 'application/json', 'request-id': 'req_err' });
        rs.end(JSON.stringify({ type: 'error', error: { type: mode.type, message: `mock ${mode.type}` } }));
      } else if (mode.kind === 'refusal') stream(rs, body.model, null, 'refusal', { type: 'refusal', category: 'cyber', explanation: null });
      else if (mode.kind === 'max_tokens') stream(rs, body.model, null, 'max_tokens');
      else stream(rs, body.model, mode.text, 'end_turn');
    });
  });
  await new Promise<void>((r) => api.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${(api.address() as AddressInfo).port}`;
});

afterAll(() => new Promise<void>((r) => api.close(() => r())));

afterEach(() => {
  calls = [];
  hooks = [];
  mode = { kind: 'text', text: 'All good.' };
});

function seedFleet(orgId = 'org_default') {
  const { token } = createLink(orgId, {}, 't');
  const { machine } = registerWithToken(token, { name: 'accounting-pc', os: 'windows' }, ['finance']);
  ingestReport(orgId, machine.id, normalizeReport({ backup_id: 'b1', name: 'Ledger', result: 'error', message: 'Access denied', stats: { backend_size: 1000 } }), {});
  return machine;
}

const KEY = 'sk-ant-api03-test-key-0123456789';

async function configure(as: ReturnType<typeof makeUser>, extra: Record<string, unknown> = {}) {
  const r = await json<{ settings: Record<string, unknown> }>(await settings.PUT(await req('PUT', '/api/ai/settings', { as, body: { enabled: true, apiKey: KEY, apiBase: base, instructions: 'Mention the finance team for finance machines.', ...extra } })));
  expect(r.status).toBe(200);
  return r.body.settings;
}

describe('AI settings', () => {
  it('stores the API key sealed and never returns it', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const op = makeUser('op@x.org', 'operator');
    const s = await configure(admin, { model: 'claude-opus-5-5', schedule: 'weekly' });
    expect(s).toMatchObject({ enabled: true, hasApiKey: true, model: 'claude-opus-5-5', schedule: 'weekly', apiBase: base });
    expect(JSON.stringify(s)).not.toContain(KEY);
    const got = await settings.GET(await req('GET', '/api/ai/settings', { as: op }));
    expect(await got.text()).not.toContain(KEY);
    const row = db().prepare('SELECT api_key_ciphertext k FROM ai_settings WHERE org_id = ?').get('org_default') as { k: string };
    expect(row.k).toMatch(/^v1\./);
    expect(row.k).not.toContain('test-key');
    // Not in the audit log either.
    expect(JSON.stringify(listAudit({}).entries)).not.toContain(KEY);
    expect(listAudit({ eventType: 'ai.settings' }).entries[0].metadata).toMatchObject({ keyChanged: true });
    // Omitting the key keeps it; null removes it.
    expect((await json<{ settings: { hasApiKey: boolean } }>(await settings.PUT(await req('PUT', '/api/ai/settings', { as: admin, body: { schedule: 'daily' } })))).body.settings.hasApiKey).toBe(true);
    expect((await json<{ settings: { hasApiKey: boolean } }>(await settings.PUT(await req('PUT', '/api/ai/settings', { as: admin, body: { apiKey: null } })))).body.settings.hasApiKey).toBe(false);
    // Validation and permissions.
    const bad = async (body: unknown) => (await settings.PUT(await req('PUT', '/api/ai/settings', { as: admin, body }))).status;
    expect(await bad({ schedule: 'hourly' })).toBe(400);
    expect(await bad({ model: 'Claude Opus!' })).toBe(400);
    expect(await bad({ apiBase: 'file:///etc/passwd' })).toBe(400);
    expect(await bad({ instructions: 'x'.repeat(4001) })).toBe(400);
    expect((await settings.PUT(await req('PUT', '/api/ai/settings', { as: op, body: { enabled: false } }))).status).toBe(403);
    // Settings are per organization.
    const other = createOrg({ name: 'Other' });
    const oa = makeUser('oa@other.org', 'org_admin', other.id);
    expect((await json<{ settings: { hasApiKey: boolean; enabled: boolean } }>(await settings.GET(await req('GET', '/api/ai/settings', { as: oa })))).body.settings).toMatchObject({ hasApiKey: false, enabled: false });
  });
});

describe('AI analysis', () => {
  it('streams an analysis from the Messages API with a credential-free snapshot', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const m = seedFleet();
    createCredential('org_default', { name: 'S3', secret: 'central-secret-value' }, 'x');
    createChannel('org_default', { name: 'Hook', kind: 'webhook', config: { url: `${base}/hook` }, secret: 'channel-secret-value' });
    await configure(admin);
    mode = { kind: 'text', text: '## Health\nOne backup is failing on accounting-pc: Access denied.' };
    const r = await json<{ report: { id: string; content: string; error: string | null; inputTokens: number; outputTokens: number; model: string; kind: string; scope: Record<string, unknown> } }>(
      await analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as: admin, body: { question: 'Why is the ledger failing?', days: 500 } })),
    );
    expect(r.status).toBe(201);
    expect(r.body.report).toMatchObject({ content: '## Health\nOne backup is failing on accounting-pc: Access denied.', error: null, inputTokens: 1234, outputTokens: 321, model: 'claude-opus-5-5', kind: 'analysis', scope: { days: 90, question: 'Why is the ledger failing?', machineId: null } });

    expect(calls).toHaveLength(1);
    const c = calls[0];
    expect(c.path).toBe('/v1/messages?beta=true');
    expect(c.headers['x-api-key']).toBe(KEY);
    expect(String(c.headers['anthropic-beta'])).toContain('server-side-fallback-2026-07-01');
    expect(c.body).toMatchObject({ model: 'claude-opus-5-5', stream: true, max_tokens: 16000, thinking: { type: 'adaptive' }, output_config: { effort: 'medium' }, fallbacks: 'default' });
    expect(c.body).not.toHaveProperty('betas');
    expect(String(c.body.system)).toContain('backup operations analyst');
    const prompt = (c.body.messages as { role: string; content: string }[])[0];
    expect(prompt.role).toBe('user');
    expect(prompt.content).toContain('Mention the finance team');
    expect(prompt.content).toContain('Question: Why is the ledger failing?');
    expect(prompt.content).toContain('accounting-pc');
    expect(prompt.content).toContain('Access denied');
    for (const secret of ['central-secret-value', 'channel-secret-value', KEY]) expect(JSON.stringify(c.body)).not.toContain(secret);
    expect(r.body.report.content).not.toContain('PRIVATE-REASONING');

    const list = await json<{ reports: { id: string }[] }>(await analyzeRoute.GET(await req('GET', '/api/ai/analyze', { as: makeUser('aud@x.org', 'read_only_auditor') })));
    expect(list.body.reports.map((x) => x.id)).toEqual([r.body.report.id]);
    expect(listAudit({ eventType: 'ai.analysis' }).total).toBe(1);

    // One machine only.
    const scoped = await json<{ report: { scope: { machineId: string } } }>(await analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as: admin, body: { machineId: m.id } })));
    expect(scoped.body.report.scope.machineId).toBe(m.id);
    expect((calls[1].body.messages as { content: string }[])[0].content).toContain('Analyze the current state.');
  });

  it('records refusals, API errors and empty answers as failed reports', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    seedFleet();
    await configure(admin);
    const run = async () => json<{ report: { content: string; error: string } }>(await analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as: admin, body: {} })));

    mode = { kind: 'refusal' };
    const refused = await run();
    expect(refused.status).toBe(502);
    expect(refused.body.report).toMatchObject({ content: '', error: 'The model declined this request (cyber)' });

    mode = { kind: 'max_tokens' };
    expect((await run()).body.report.error).toBe('The answer was cut off (output limit reached)');

    mode = { kind: 'status', status: 401, type: 'authentication_error' };
    expect((await run()).body.report.error).toBe('The Anthropic API key was rejected');

    mode = { kind: 'status', status: 400, type: 'invalid_request_error' };
    expect((await run()).body.report.error).toMatch(/^Anthropic API error 400: /);

    expect(listAudit({ eventType: 'ai.analysis' }).entries.every((e) => e.severity === 'warning')).toBe(true);
  });

  it('refuses when disabled, without a key, for machine owners and for foreign machines', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    const other = createOrg({ name: 'Other' });
    const foreign = seedFleet(other.id);
    const post = async (as: ReturnType<typeof makeUser>, body: unknown = {}) => analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as, body }));
    expect((await post(admin)).status).toBe(409);
    await settings.PUT(await req('PUT', '/api/ai/settings', { as: admin, body: { enabled: true } }));
    expect(await json(await post(admin))).toMatchObject({ status: 409, body: { error: 'Add an Anthropic API key in AI settings first' } });
    await configure(admin);
    expect((await post(admin, { machineId: foreign.id })).status).toBe(404);
    expect((await post(makeUser('u@x.org', 'user'))).status).toBe(403);
    expect(calls).toHaveLength(0);
  });
});

describe('scheduled summaries', () => {
  it('runs when due, delivers to channels and records the run', async () => {
    const admin = makeUser('admin@x.org', 'org_admin');
    seedFleet();
    const ch = createChannel('org_default', { name: 'Hook', kind: 'webhook', config: { url: `${base}/hook` } });
    await configure(admin, { schedule: 'daily', channelIds: [ch.id] });
    // Another organization with summaries off is skipped.
    const other = createOrg({ name: 'Other' });
    await configure(makeUser('oa@other.org', 'org_admin', other.id), { schedule: 'off' });
    mode = { kind: 'text', text: 'Daily: 1 failing backup.' };

    const t0 = new Date('2026-10-22T06:00:00Z');
    expect(await runScheduledSummaries(t0)).toBe(1);
    expect(calls).toHaveLength(1);
    expect((calls[0].body.messages as { content: string }[])[0].content).toContain('Write the daily summary.');
    expect(hooks).toEqual([expect.objectContaining({ subject: '[UFOP] Daily backup summary — Default organization', text: 'Daily: 1 failing backup.', event: 'ai_summary' })]);
    // Not again within the day; again the next day.
    expect(await runScheduledSummaries(new Date(t0.getTime() + 12 * 3600_000))).toBe(0);
    expect(await runScheduledSummaries(new Date(t0.getTime() + 24 * 3600_000))).toBe(1);
    const reports = await json<{ reports: { kind: string; createdBy: string }[] }>(await analyzeRoute.GET(await req('GET', '/api/ai/analyze', { as: admin })));
    expect(reports.body.reports.map((r) => [r.kind, r.createdBy])).toEqual([['summary', 'scheduler'], ['summary', 'scheduler']]);

    // A failed summary is audited and nothing is delivered.
    hooks = [];
    mode = { kind: 'refusal' };
    expect(await runScheduledSummaries(new Date(t0.getTime() + 48 * 3600_000))).toBe(1);
    expect(hooks).toEqual([]);
    expect(listAudit({ eventType: 'ai.summary' }).entries[0]).toMatchObject({ severity: 'warning' });
    expect(listAudit({ eventType: 'ai.summary' }).entries[0].description).toContain('declined');

    // Offboarding organizations get no summaries.
    db().prepare("UPDATE organizations SET status = 'offboarding' WHERE id = 'org_default'").run();
    expect(await runScheduledSummaries(new Date(t0.getTime() + 96 * 3600_000))).toBe(0);
    expect(verifyAudit()).toMatchObject({ ok: true });
  });
});

describe('AI transcripts', () => {
  it('keeps the full exchange, hashes it onto the audit chain and detects later changes', async () => {
    const admin = makeUser('tr-admin@x.org', 'org_admin');
    seedFleet();
    await configure(admin);
    mode = { kind: 'text', text: 'Transcript answer.' };
    const r = await json<{ report: { id: string } }>(await analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as: admin, body: { question: 'What failed?' } })));
    const get = async (id: string, as = admin) => json<{ transcript: { request: string; response: string; systemPrompt: string; model: string; sha256: string; verification: { matches: boolean; chainOk: boolean; auditSequence: number; reason: string | null } } }>(await transcript.GET(await req('GET', `/api/ai/transcripts/${id}`, { as }), { params: Promise.resolve({ id }) }));
    const t = await get(r.body.report.id);
    expect(t.status).toBe(200);
    expect(t.body.transcript).toMatchObject({ response: 'Transcript answer.', model: 'claude-opus-5-5', verification: { matches: true, chainOk: true, reason: null } });
    expect(t.body.transcript.request).toContain('Question: What failed?');
    expect(t.body.transcript.systemPrompt).toContain('backup operations analyst');
    expect(t.body.transcript.request).toBe((calls.at(-1)!.body.messages as { content: string }[])[0].content); // exactly what was sent
    const entry = listAudit({ eventType: 'ai.transcript' }).entries[0];
    expect(entry.metadata).toMatchObject({ sha256: t.body.transcript.sha256, inputTokens: 1234, outputTokens: 321 });
    expect(entry.sequence).toBe(t.body.transcript.verification.auditSequence);

    // Failed runs are kept too.
    mode = { kind: 'refusal' };
    const failed = await json<{ report: { id: string } }>(await analyzeRoute.POST(await req('POST', '/api/ai/analyze', { as: admin, body: {} })));
    expect((await get(failed.body.report.id)).body.transcript.response).toMatch(/^\[error\] The model declined/);

    // Someone edits the stored transcript afterwards.
    db().prepare("UPDATE ai_transcripts SET response = 'Everything is fine.' WHERE report_id = ?").run(r.body.report.id);
    const tampered = await get(r.body.report.id);
    expect(tampered.body.transcript.verification).toMatchObject({ matches: false, chainOk: true, reason: 'The transcript was changed after it was recorded' });

    // Needs audit read as well as AI read; other organizations cannot see it.
    expect((await transcript.GET(await req('GET', `/api/ai/transcripts/${r.body.report.id}`, { as: makeUser('tr-op@x.org', 'approver') }), { params: Promise.resolve({ id: r.body.report.id }) })).status).toBe(403);
    const other = makeUser('tr-other@x.org', 'org_admin', createOrg({ name: 'Transcript other' }).id);
    expect((await get(r.body.report.id, other)).status).toBe(404);
    expect(listAudit({ eventType: 'ai.transcript_view' }).total).toBeGreaterThan(0);
  });
});
