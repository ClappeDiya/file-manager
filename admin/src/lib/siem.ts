/**
 * Forwarding the audit log to SIEMs (destinations in `db/repos/siem`):
 *
 * - syslog: RFC 5424 messages (JSON body) over UDP, TCP (RFC 6587 octet
 *   counting) or TLS (RFC 5425)
 * - hec: Splunk HTTP Event Collector (newline-separated events, `Splunk <token>`)
 * - https: a JSON batch `{events: [...]}`, with the secret as bearer token and
 *   an HMAC signature (`X-UFOP-Signature`, as for webhooks)
 * - jsonl: JSON lines appended to a file in SIEM_JSONL_DIR
 *
 * Every event is the full audit record (sequence and hash-chain fields
 * included), so the receiving side can verify it. Internal addresses are
 * refused unless SIEM_ALLOW_PRIVATE=1 (an internal SIEM is common: the
 * operator opts in), checked on the address actually connected to.
 */
import dgram from 'dgram';
import fs from 'fs';
import net from 'net';
import os from 'os';
import path from 'path';
import tls from 'tls';
import { auditAfter, type AuditRecord } from './db/repos/audit';
import { dueSiemDestinations, recordSiemFailure, recordSiemSuccess, siemOrgIds, siemSecret, type HecConfig, type HttpsConfig, type JsonlConfig, type SiemDestination, type SyslogConfig } from './db/repos/siem';
import { guardedLookup, literalPrivate, postGuarded, sign, TIMEOUT_MS } from './delivery';
import { logger } from './utils/logger';

export const BATCH = 200;
/** Batches per destination per run, so one large backlog does not starve the others. */
const MAX_BATCHES_PER_RUN = 10;
const ENV = 'SIEM_ALLOW_PRIVATE';
const allowPrivate = () => process.env[ENV] === '1';

const SEVERITY: Record<string, number> = { critical: 2, error: 3, warning: 4, info: 6 };
const hostName = () => {
  try {
    return process.env.ADMIN_PUBLIC_URL ? new URL(process.env.ADMIN_PUBLIC_URL).hostname : os.hostname();
  } catch {
    return os.hostname();
  }
};
/** RFC 5424 field: printable US-ASCII without spaces, bounded. */
const field = (s: string, max: number) => s.replace(/[^\x21-\x7e]/g, '_').slice(0, max) || '-';

/** One RFC 5424 syslog message for an audit record. */
export function syslogMessage(e: AuditRecord, facility = 16): string {
  const pri = facility * 8 + (SEVERITY[e.severity] ?? 6);
  return `<${pri}>1 ${e.timestamp} ${field(hostName(), 255)} ufop-console - ${field(e.eventType, 32)} - ${JSON.stringify(e)}`;
}

export interface SendResult {
  ok: boolean;
  error: string | null;
}

function viaSocket(c: SyslogConfig, frames: string[]): Promise<SendResult> {
  return new Promise((resolve) => {
    if (literalPrivate(c.host, allowPrivate())) return resolve({ ok: false, error: `${c.host} is an internal address; set ${ENV}=1 to allow it` });
    const lookup = guardedLookup(allowPrivate(), ENV) as unknown as net.LookupFunction;
    let settled = false;
    const done = (r: SendResult) => {
      if (!settled) {
        settled = true;
        resolve(r);
      }
    };
    if (c.transport === 'udp') {
      lookup(c.host, { family: 0 }, (err, address, family) => {
        if (err) return done({ ok: false, error: err.message });
        const sock = dgram.createSocket(family === 6 ? 'udp6' : 'udp4');
        let left = frames.length;
        sock.on('error', (e) => {
          sock.close();
          done({ ok: false, error: e.message });
        });
        // A datagram per message; 8 KiB keeps under common receiver limits.
        for (const f of frames)
          sock.send(Buffer.from(f).subarray(0, 8192), c.port, address as string, (e) => {
            if (e) {
              sock.close();
              return done({ ok: false, error: e.message });
            }
            if (--left === 0) {
              sock.close();
              done({ ok: true, error: null });
            }
          });
      });
      return;
    }
    const payload = frames.map((f) => `${Buffer.byteLength(f)} ${f}`).join('');
    const onConnect = () => sock.end(payload);
    const sock =
      c.transport === 'tls'
        ? tls.connect({ host: c.host, port: c.port, servername: net.isIP(c.host) ? undefined : c.host, ca: c.caPem || undefined, lookup }, onConnect)
        : net.connect({ host: c.host, port: c.port, lookup }, onConnect);
    sock.setTimeout(TIMEOUT_MS, () => sock.destroy(new Error(`No connection within ${TIMEOUT_MS / 1000} s`)));
    sock.on('error', (e) => done({ ok: false, error: e.message }));
    sock.on('finish', () => done({ ok: true, error: null }));
    sock.on('close', (hadError) => done(hadError ? { ok: false, error: 'Connection closed with an error' } : { ok: true, error: null }));
  });
}

async function viaHttp(url: string, headers: Record<string, string>, body: string): Promise<SendResult> {
  const r = await postGuarded(url, { headers, body, allowPrivate: allowPrivate(), env: ENV, userAgent: 'UFOP-SIEM/1.0' });
  return { ok: r.ok, error: r.error };
}

/** Send a batch of audit records to one destination. */
export async function sendToSiem(d: SiemDestination, events: AuditRecord[]): Promise<SendResult> {
  if (!events.length) return { ok: true, error: null };
  const secret = siemSecret(d.id);
  switch (d.kind) {
    case 'syslog': {
      const c = d.config as SyslogConfig;
      return viaSocket(
        c,
        events.map((e) => syslogMessage(e, c.facility)),
      );
    }
    case 'hec': {
      const c = d.config as HecConfig;
      const body = events
        .map((e) => JSON.stringify({ time: Date.parse(e.timestamp) / 1000, host: hostName(), source: 'ufop-console', sourcetype: c.sourcetype ?? 'ufop:audit', ...(c.index ? { index: c.index } : {}), event: e }))
        .join('\n');
      return viaHttp(c.url, { 'content-type': 'application/json', authorization: `Splunk ${secret ?? ''}` }, body);
    }
    case 'https': {
      const c = d.config as HttpsConfig;
      const body = JSON.stringify({ source: 'ufop-console', events });
      const ts = Math.floor(Date.now() / 1000);
      return viaHttp(c.url, { 'content-type': 'application/json', ...(secret ? { authorization: `Bearer ${secret}`, 'x-ufop-timestamp': String(ts), 'x-ufop-signature': sign(secret, ts, body) } : {}) }, body);
    }
    case 'jsonl': {
      const dir = process.env.SIEM_JSONL_DIR;
      if (!dir) return { ok: false, error: 'SIEM_JSONL_DIR is not set' };
      const file = path.join(dir, path.basename((d.config as JsonlConfig).file));
      try {
        await fs.promises.mkdir(dir, { recursive: true });
        await fs.promises.appendFile(file, events.map((e) => JSON.stringify(e)).join('\n') + '\n', { mode: 0o600 });
        return { ok: true, error: null };
      } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
      }
    }
  }
}

/** Forward what is due to one destination; returns how many entries it accepted. */
export async function forwardDestination(d: SiemDestination, at = new Date()): Promise<number> {
  let cursor = d.cursor;
  let sent = 0;
  const orgIds = siemOrgIds(d);
  for (let i = 0; i < MAX_BATCHES_PER_RUN; i++) {
    const batch = auditAfter(orgIds, cursor, BATCH);
    if (!batch.length) break;
    const r = await sendToSiem(d, batch);
    if (!r.ok) {
      recordSiemFailure(d.id, r.error ?? 'Send failed', at);
      return sent;
    }
    cursor = batch[batch.length - 1].sequence;
    sent += batch.length;
    recordSiemSuccess(d.id, cursor, batch.length, at);
    if (batch.length < BATCH) break;
  }
  return sent;
}

let running: Promise<number> | null = null;

/** Forward to every due destination (one run at a time). */
export function forwardAll(at = new Date()): Promise<number> {
  if (running) return running;
  running = (async () => {
    let n = 0;
    for (const d of dueSiemDestinations(at)) n += await forwardDestination(d, at);
    return n;
  })().finally(() => {
    running = null;
  });
  return running;
}

/** A synthetic event to check a destination (the cursor does not move). */
export function testEvent(d: SiemDestination): AuditRecord {
  const ts = new Date().toISOString();
  return {
    id: 'aud_test',
    sequence: 0,
    eventType: 'siem.test',
    severity: 'info',
    timestamp: ts,
    orgId: d.orgId,
    organizationId: d.orgId,
    userId: '',
    userEmail: '',
    userName: '',
    ipAddress: '',
    userAgent: '',
    description: `Test event for SIEM destination "${d.name}"`,
    metadata: {},
    integrityHash: '',
    previousHash: '',
  };
}

let loop: NodeJS.Timeout | null = null;

/** Background forwarding every `intervalMs` (started with the periodic jobs). */
export function startSiemForwarding(intervalMs = 30_000): void {
  if (loop) return;
  loop = setInterval(() => {
    forwardAll().catch((e) => logger.error('siem.forward_failed', { error: e instanceof Error ? e.message : String(e) }));
  }, intervalMs);
  loop.unref?.();
}
