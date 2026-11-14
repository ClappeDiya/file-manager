/** SIEM forwarding: real syslog (TCP/UDP), HEC, HTTPS and JSON-lines receivers; cursor, backlog, retry, scope. */
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import dgram from 'dgram';
import fs from 'fs';
import http from 'http';
import net from 'net';
import os from 'os';
import path from 'path';
import tls from 'tls';
import { db } from '@/lib/db';
import { appendAudit } from '@/lib/db/repos/audit';
import { createOrg } from '@/lib/db/repos/orgs';
import { createSiemDestination, getSiemDestination, siemBacklog } from '@/lib/db/repos/siem';
import { forwardAll, forwardDestination, syslogMessage, BATCH } from '@/lib/siem';
import { json, makeUser, req } from './helpers';
import * as siem from '@/app/api/siem/route';
import * as siemById from '@/app/api/siem/[id]/route';
import * as siemTest from '@/app/api/siem/[id]/test/route';

let tcp: net.Server;
let tcpPort = 0;
const tcpData: string[] = [];
let udp: dgram.Socket;
let udpPort = 0;
const udpData: string[] = [];
let web: http.Server;
let webPort = 0;
let webStatus = 200;
const webReqs: { url: string; headers: http.IncomingHttpHeaders; body: string }[] = [];
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'siem-'));

beforeAll(async () => {
  process.env.SIEM_ALLOW_PRIVATE = '1';
  process.env.SIEM_JSONL_DIR = dir;
  tcp = net.createServer((s) => {
    let buf = '';
    s.on('data', (c) => (buf += c.toString()));
    s.on('end', () => tcpData.push(buf));
  });
  await new Promise<void>((r) => tcp.listen(0, '127.0.0.1', r));
  tcpPort = (tcp.address() as net.AddressInfo).port;
  udp = dgram.createSocket('udp4');
  udp.on('message', (m) => udpData.push(m.toString()));
  await new Promise<void>((r) => udp.bind(0, '127.0.0.1', r));
  udpPort = udp.address().port;
  web = http.createServer((rq, rs) => {
    let body = '';
    rq.on('data', (c) => (body += c));
    rq.on('end', () => {
      webReqs.push({ url: rq.url ?? '', headers: rq.headers, body });
      rs.writeHead(webStatus).end('{}');
    });
  });
  await new Promise<void>((r) => web.listen(0, '127.0.0.1', r));
  webPort = (web.address() as net.AddressInfo).port;
});

afterAll(() => {
  tcp.close();
  udp.close();
  web.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

afterEach(() => {
  process.env.SIEM_ALLOW_PRIVATE = '1';
  webStatus = 200;
});

const until = async (f: () => boolean) => {
  for (let i = 0; i < 100 && !f(); i++) await new Promise((r) => setTimeout(r, 10));
};

describe('SIEM forwarding', () => {
  it('forwards over syslog TCP (octet counting) and UDP as RFC 5424 with the full record', async () => {
    const org = createOrg({ name: 'Syslog org' });
    const t = createSiemDestination(org.id, { name: 'tcp', kind: 'syslog', config: { host: '127.0.0.1', port: tcpPort, transport: 'tcp' } }, 'x');
    const u = createSiemDestination(org.id, { name: 'udp', kind: 'syslog', config: { host: '127.0.0.1', port: udpPort, transport: 'udp' } }, 'x');
    appendAudit({ eventType: 'machine.register', orgId: org.id, severity: 'warning', description: 'one' });
    appendAudit({ eventType: 'user login', orgId: org.id, description: 'two' });
    expect(await forwardDestination(getSiemDestination(org.id, t.id)!)).toBe(2);
    await until(() => tcpData.length > 0);
    const frames = tcpData.join('');
    const first = /^(\d+) /.exec(frames)!;
    const msg1 = frames.slice(first[0].length, first[0].length + Number(first[1]));
    expect(msg1).toMatch(/^<132>1 \S+ \S+ ufop-console - machine.register - \{/); // local0 (16) * 8 + warning (4)
    const rec = JSON.parse(msg1.slice(msg1.indexOf('{')));
    expect(rec).toMatchObject({ description: 'one', orgId: org.id, sequence: expect.any(Number), integrityHash: expect.any(String) });
    expect(frames).toContain(' user_login - '); // MSGID without spaces
    expect(await forwardDestination(getSiemDestination(org.id, u.id)!)).toBe(2);
    await until(() => udpData.length >= 2);
    expect(udpData.map((d) => d.slice(0, 7)).sort()).toEqual(['<132>1 ', '<134>1 ']);
    expect(getSiemDestination(org.id, t.id)).toMatchObject({ forwardedTotal: 2, failures: 0, lastError: null });
    expect(siemBacklog(getSiemDestination(org.id, t.id)!)).toBe(0);
    expect(syslogMessage({ ...rec, severity: 'critical' }, 4)).toMatch(/^<34>1 /);
  });

  it('forwards over syslog TLS, verifying the server against the configured CA', async () => {
    const fx = (f: string) => fs.readFileSync(path.join(__dirname, 'fixtures', f), 'utf8');
    const got: string[] = [];
    const server = tls.createServer({ key: fx('localhost-key.pem'), cert: fx('localhost-cert.pem') }, (s) => {
      let buf = '';
      s.on('data', (c) => (buf += c.toString()));
      s.on('end', () => got.push(buf));
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    const port = (server.address() as net.AddressInfo).port;
    try {
      const org = createOrg({ name: 'TLS org' });
      const untrusted = createSiemDestination(org.id, { name: 'no ca', kind: 'syslog', config: { host: 'localhost', port, transport: 'tls' } }, 'x');
      const trusted = createSiemDestination(org.id, { name: 'ca', kind: 'syslog', config: { host: 'localhost', port, transport: 'tls', caPem: fx('localhost-cert.pem') } }, 'x');
      appendAudit({ eventType: 'tls.event', orgId: org.id, description: 'over tls' });
      expect(await forwardDestination(getSiemDestination(org.id, untrusted.id)!)).toBe(0);
      expect(getSiemDestination(org.id, untrusted.id)!.lastError).toMatch(/self.signed|certificate/i);
      expect(await forwardDestination(getSiemDestination(org.id, trusted.id)!)).toBe(1);
      await until(() => got.length > 0);
      expect(got.join('')).toMatch(/^\d+ <134>1 .* tls.event - .*"over tls"/);
    } finally {
      server.close();
    }
  });

  it('starts from now unless backfilling, batches the backlog, and only forwards its organizations', async () => {
    const parent = createOrg({ name: 'SIEM parent' });
    const child = createOrg({ name: 'SIEM child', parentId: parent.id });
    const other = createOrg({ name: 'SIEM other' });
    for (let i = 0; i < BATCH + 5; i++) appendAudit({ eventType: 'bulk', orgId: parent.id, description: `old ${i}` });
    const fresh = createSiemDestination(parent.id, { name: 'fresh', kind: 'jsonl', config: { file: 'fresh.jsonl' } }, 'x');
    const all = createSiemDestination(parent.id, { name: 'all', kind: 'jsonl', config: { file: 'all.jsonl' }, backfill: true, includeChildren: true }, 'x');
    expect(siemBacklog(fresh)).toBe(0);
    expect(siemBacklog(all)).toBe(BATCH + 5);
    appendAudit({ eventType: 'new', orgId: parent.id, description: 'parent new' });
    appendAudit({ eventType: 'new', orgId: child.id, description: 'child new' });
    appendAudit({ eventType: 'new', orgId: other.id, description: 'other org' });
    expect(await forwardDestination(getSiemDestination(parent.id, fresh.id)!)).toBe(1);
    expect(await forwardDestination(getSiemDestination(parent.id, all.id)!)).toBe(BATCH + 7);
    const lines = fs
      .readFileSync(path.join(dir, 'all.jsonl'), 'utf8')
      .trim()
      .split('\n')
      .map((l) => JSON.parse(l));
    expect(lines).toHaveLength(BATCH + 7);
    expect(lines.map((l) => l.sequence)).toEqual([...lines.map((l) => l.sequence)].sort((a, b) => a - b));
    expect(lines.some((l) => l.description === 'child new')).toBe(true);
    expect(lines.some((l) => l.description === 'other org')).toBe(false);
    expect(
      fs
        .readFileSync(path.join(dir, 'fresh.jsonl'), 'utf8')
        .trim()
        .split('\n')
        .map((l) => JSON.parse(l).description),
    ).toEqual(['parent new']);
  });

  it('keeps the cursor on failure, backs off, and catches up without duplicates (HEC and HTTPS)', async () => {
    const org = createOrg({ name: 'HEC org' });
    const hec = createSiemDestination(org.id, { name: 'splunk', kind: 'hec', config: { url: `http://127.0.0.1:${webPort}/services/collector/event`, index: 'sec' }, secret: 'hec-token' }, 'x');
    const hook = createSiemDestination(org.id, { name: 'https', kind: 'https', config: { url: `http://127.0.0.1:${webPort}/ingest` }, secret: 'shh' }, 'x');
    appendAudit({ eventType: 'a', orgId: org.id, description: 'first' });
    webStatus = 503;
    webReqs.length = 0;
    expect(await forwardDestination(getSiemDestination(org.id, hec.id)!)).toBe(0);
    const failed = getSiemDestination(org.id, hec.id)!;
    expect(failed).toMatchObject({ failures: 1, lastError: expect.stringContaining('HTTP 503') });
    expect(new Date(failed.nextAttemptAt!).getTime()).toBeGreaterThan(Date.now() + 50_000);
    expect(siemBacklog(failed)).toBe(1);
    // Not due yet: the run skips it.
    await forwardAll();
    expect(webReqs.filter((r) => r.url.startsWith('/services')).length).toBe(1);

    webStatus = 200;
    appendAudit({ eventType: 'b', orgId: org.id, description: 'second' });
    await forwardAll(new Date(Date.now() + 3600_000));
    const hecReq = webReqs.filter((r) => r.url.startsWith('/services')).at(-1)!;
    expect(hecReq.headers.authorization).toBe('Splunk hec-token');
    const events = hecReq.body.split('\n').map((l) => JSON.parse(l));
    expect(events.map((e) => e.event.description)).toEqual(['first', 'second']);
    expect(events[0]).toMatchObject({ sourcetype: 'ufop:audit', index: 'sec', source: 'ufop-console' });
    expect(getSiemDestination(org.id, hec.id)).toMatchObject({ failures: 0, lastError: null, forwardedTotal: 2 });
    const httpsReq = webReqs.filter((r) => r.url === '/ingest').at(-1)!; // the retry after the 503
    expect(httpsReq.headers.authorization).toBe('Bearer shh');
    expect(httpsReq.headers['x-ufop-signature']).toMatch(/^sha256=/);
    expect(JSON.parse(httpsReq.body).events.map((e: { description: string }) => e.description)).toEqual(['first', 'second']);
    // Nothing new: nothing sent again.
    const before = webReqs.length;
    await forwardAll(new Date(Date.now() + 7200_000));
    expect(webReqs.length).toBe(before);
    expect(getSiemDestination(org.id, hook.id)!.forwardedTotal).toBe(2);
  });

  it('refuses internal addresses unless allowed, and validates configuration through the API', async () => {
    const org = createOrg({ name: 'SIEM API org' });
    const admin = makeUser('siem-admin@x.org', 'org_admin', org.id);
    const post = async (body: Record<string, unknown>) => json<{ destination?: { id: string }; error?: string }>(await siem.POST(await req('POST', '/api/siem', { as: admin, body })));
    expect((await post({ name: 'x', kind: 'syslog', config: { host: '', port: 514 } })).status).toBe(400);
    expect((await post({ name: 'x', kind: 'syslog', config: { host: 'h', port: 70000 } })).status).toBe(400);
    expect((await post({ name: 'x', kind: 'hec', config: { url: 'https://s/collector' } })).body.error).toMatch(/HEC token/);
    expect((await post({ name: 'x', kind: 'jsonl', config: { file: '../etc/passwd' } })).status).toBe(400);
    expect((await post({ name: 'x', kind: 'carrier-pigeon', config: {} })).status).toBe(400);
    const created = await post({ name: 'internal', kind: 'syslog', config: { host: '127.0.0.1', port: tcpPort, transport: 'tcp' } });
    expect(created.status).toBe(201);
    const id = created.body.destination!.id;
    const list = await (await siem.GET(await req('GET', '/api/siem', { as: admin }))).text();
    expect(list).toContain('"backlog"');

    process.env.SIEM_ALLOW_PRIVATE = '0';
    const t = await json<{ ok: boolean; error: string }>(await siemTest.POST(await req('POST', `/api/siem/${id}/test`, { as: admin }), { params: Promise.resolve({ id }) }));
    expect(t.status).toBe(502);
    expect(t.body.error).toMatch(/internal address; set SIEM_ALLOW_PRIVATE=1/);
    process.env.SIEM_ALLOW_PRIVATE = '1';
    expect((await siemTest.POST(await req('POST', `/api/siem/${id}/test`, { as: admin }), { params: Promise.resolve({ id }) })).status).toBe(200);
    await until(() => tcpData.some((d) => d.includes('siem.test')));
    expect(getSiemDestination(org.id, id)!.forwardedTotal).toBe(0); // the test does not move the cursor

    // Pausing stops forwarding; the cursor is kept.
    expect((await siemById.PATCH(await req('PATCH', `/api/siem/${id}`, { as: admin, body: { enabled: false } }), { params: Promise.resolve({ id }) })).status).toBe(200);
    appendAudit({ eventType: 'x', orgId: org.id, description: 'while paused' });
    await forwardAll(new Date(Date.now() + 86_400_000));
    expect(getSiemDestination(org.id, id)!.forwardedTotal).toBe(0);
    expect((await siemById.DELETE(await req('DELETE', `/api/siem/${id}`, { as: admin }), { params: Promise.resolve({ id }) })).status).toBe(200);
    expect(db().prepare('SELECT COUNT(*) AS n FROM siem_destinations WHERE id = ?').get(id)).toEqual({ n: 0 });
  });
});
