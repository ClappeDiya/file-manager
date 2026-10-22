/**
 * Webhook delivery: sends queued deliveries (`db/repos/webhooks`) and
 * records each attempt. The same loop retries the notification outbox.
 *
 * Every request is a JSON POST with
 *   X-UFOP-Event, X-UFOP-Delivery, X-UFOP-Timestamp (unix seconds) and
 *   X-UFOP-Signature: sha256=<hex HMAC-SHA256(secret, "<timestamp>.<body>")>
 * so receivers can verify the sender and reject replays.
 *
 * Addresses are resolved here and checked before connecting (the checked
 * address is the one used, so DNS rebinding cannot slip past): loopback,
 * private, link-local and other internal ranges are refused unless
 * WEBHOOKS_ALLOW_PRIVATE=1. Redirects are not followed.
 */
import crypto from 'crypto';
import dns from 'dns';
import http from 'http';
import https from 'https';
import net from 'net';
import { dueDeliveries, pruneDeliveries, recordAttempt, setEnqueueListener, webhookSecret, type WebhookDelivery } from './db/repos/webhooks';
import { logger } from './utils/logger';
import { processOutbox } from './mailer';

export const TIMEOUT_MS = 10_000;
const MAX_RESPONSE_BYTES = 4096;

/** Internal address ranges a webhook may not reach by default. */
export function isPrivateAddress(ip: string): boolean {
  if (net.isIPv4(ip)) {
    const [a, b] = ip.split('.').map(Number);
    return (
      a === 0 || a === 10 || a === 127 || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 198 && (b === 18 || b === 19)) || a >= 224
    );
  }
  const v6 = ip.toLowerCase();
  if (v6.startsWith('::ffff:')) return isPrivateAddress(v6.slice(7));
  return v6 === '::' || v6 === '::1' || v6.startsWith('fc') || v6.startsWith('fd') || v6.startsWith('fe8') || v6.startsWith('fe9') || v6.startsWith('fea') || v6.startsWith('feb') || v6.startsWith('ff');
}

const allowPrivate = () => process.env.WEBHOOKS_ALLOW_PRIVATE === '1';

/** `dns.lookup` that refuses internal addresses (used by the HTTP agent). */
function guardedLookup(hostname: string, options: dns.LookupOptions, cb: (err: NodeJS.ErrnoException | null, address: string | dns.LookupAddress[], family?: number) => void): void {
  dns.lookup(hostname, { ...options, all: true }, (err, addresses) => {
    if (err) return cb(err, '', 0);
    const list = addresses as dns.LookupAddress[];
    const blocked = !allowPrivate() && list.some((a) => isPrivateAddress(a.address));
    if (blocked) {
      const e: NodeJS.ErrnoException = new Error(`${hostname} resolves to an internal address; set WEBHOOKS_ALLOW_PRIVATE=1 to allow it`);
      e.code = 'EPRIVATE';
      return cb(e, '', 0);
    }
    if (options.all) return cb(null, list);
    cb(null, list[0].address, list[0].family);
  });
}

export function sign(secret: string, timestamp: number, body: string): string {
  return `sha256=${crypto.createHmac('sha256', secret).update(`${timestamp}.${body}`).digest('hex')}`;
}

export interface AttemptResult {
  ok: boolean;
  status: number | null;
  error: string | null;
  durationMs: number;
}

/** POST one delivery. 2xx = delivered; anything else (incl. redirects) is a failure. */
export function send(url: string, headers: Record<string, string>, secret: string, d: Pick<WebhookDelivery, 'id' | 'event' | 'payload'>): Promise<AttemptResult> {
  const started = Date.now();
  const body = JSON.stringify(d.payload);
  const ts = Math.floor(started / 1000);
  return new Promise((resolve) => {
    const done = (r: Omit<AttemptResult, 'durationMs'>) => resolve({ ...r, durationMs: Date.now() - started });
    let u: URL;
    try {
      u = new URL(url);
    } catch {
      return done({ ok: false, status: null, error: 'Invalid URL' });
    }
    // A literal IP skips DNS: check it here.
    const host = u.hostname.replace(/^\[|\]$/g, '');
    if (net.isIP(host) && isPrivateAddress(host) && !allowPrivate()) {
      return done({ ok: false, status: null, error: `${host} is an internal address; set WEBHOOKS_ALLOW_PRIVATE=1 to allow it` });
    }
    const mod = u.protocol === 'https:' ? https : http;
    const req = mod.request(
      u,
      {
        method: 'POST',
        lookup: guardedLookup as unknown as net.LookupFunction,
        timeout: TIMEOUT_MS,
        headers: {
          ...headers,
          'content-type': 'application/json',
          'content-length': Buffer.byteLength(body),
          'user-agent': 'UFOP-Webhooks/1.0',
          'x-ufop-event': d.event,
          'x-ufop-delivery': d.id,
          'x-ufop-timestamp': String(ts),
          'x-ufop-signature': sign(secret, ts, body),
        },
      },
      (res) => {
        let text = '';
        res.setEncoding('utf8');
        res.on('data', (c: string) => {
          if (text.length < MAX_RESPONSE_BYTES) text += c;
        });
        res.on('end', () => {
          const status = res.statusCode ?? 0;
          const ok = status >= 200 && status < 300;
          done({ ok, status, error: ok ? null : `HTTP ${status}${status >= 300 && status < 400 ? ' (redirects are not followed)' : ''}: ${text.slice(0, 200)}` });
        });
      },
    );
    req.on('timeout', () => req.destroy(new Error(`No response within ${TIMEOUT_MS / 1000} s`)));
    req.on('error', (e) => done({ ok: false, status: null, error: e.message }));
    req.end(body);
  });
}

let running: Promise<number> | null = null;

/** Send every due delivery once; returns how many were attempted. */
export async function processDueDeliveries(at = new Date()): Promise<number> {
  if (running) return running;
  running = (async () => {
    let n = 0;
    for (;;) {
      const batch = dueDeliveries(at);
      if (!batch.length) break;
      for (const d of batch) {
        const secret = webhookSecret(d.webhookId);
        const r = secret ? await send(d.webhook.url, d.webhook.headers, secret, d) : { ok: false, status: null, error: 'Webhook removed', durationMs: 0 };
        recordAttempt(d, r, new Date(Math.max(Date.now(), at.getTime())));
        n++;
      }
    }
    return n;
  })().finally(() => {
    running = null;
  });
  return running;
}

let loop: NodeJS.Timeout | null = null;

/** Background sending: every 30 s and right after something is queued. */
export function startDelivery(intervalMs = 30_000): void {
  if (loop) return;
  const tick = () => {
    processDueDeliveries().catch((e) => logger.error('webhooks.delivery_failed', { error: e instanceof Error ? e.message : String(e) }));
  };
  setEnqueueListener(() => setTimeout(tick, 0).unref?.());
  loop = setInterval(() => {
    tick();
    pruneDeliveries();
    processOutbox().catch((e) => logger.error('outbox.failed', { error: e instanceof Error ? e.message : String(e) }));
  }, intervalMs);
  loop.unref?.();
}
