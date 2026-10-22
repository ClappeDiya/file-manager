/**
 * Message delivery for alerts, summaries and invitations: SMTP email (nodemailer),
 * Slack incoming webhooks and generic JSON webhooks (HMAC-SHA256 signed when
 * the channel has a secret: `X-UFOP-Signature: sha256=<hex of body>`).
 */
import crypto from 'crypto';
import nodemailer from 'nodemailer';
import { channelSecret, getChannel, listChannels, markDelivery, type Channel, type EmailConfig, type WebhookConfig } from './db/repos/channels';
import { addToOutbox, dueOutbox, getOutboxEntry, pruneOutbox, recordOutboxAttempt } from './db/repos/outbox';

export interface Message {
  subject: string;
  text: string;
  /** Structured payload for webhooks. */
  data?: Record<string, unknown>;
  /** Override recipients (invitations). */
  to?: string[];
}

export interface Delivery {
  channelId: string;
  ok: boolean;
  error?: string;
}

async function sendEmail(ch: Channel, m: Message): Promise<void> {
  const c = ch.config as EmailConfig;
  const password = channelSecret(ch.id);
  const transport = nodemailer.createTransport({
    host: c.host,
    port: c.port || (c.security === 'tls' ? 465 : 587),
    secure: c.security === 'tls',
    requireTLS: c.security === 'starttls',
    ignoreTLS: c.security === 'none',
    auth: c.username ? { user: c.username, pass: password ?? '' } : undefined,
    tls: c.caPem ? { ca: [c.caPem] } : undefined,
    connectionTimeout: 15_000,
    greetingTimeout: 15_000,
  });
  const to = m.to ?? c.to;
  if (!to.length) throw new Error('No recipients');
  await transport.sendMail({ from: c.from, to, subject: m.subject, text: m.text });
}

async function postJson(url: string, body: string, headers: Record<string, string>): Promise<void> {
  const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json', ...headers }, body, signal: AbortSignal.timeout(15_000) });
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).slice(0, 200)}`);
}

export async function deliver(ch: Channel, m: Message): Promise<Delivery> {
  try {
    if (!ch.enabled) throw new Error('Channel is disabled');
    if (ch.kind === 'email') await sendEmail(ch, m);
    else if (ch.kind === 'slack') {
      const url = channelSecret(ch.id);
      if (!url) throw new Error('The Slack webhook URL is missing');
      await postJson(url, JSON.stringify({ text: `*${m.subject}*\n${m.text}` }), {});
    } else {
      const c = ch.config as WebhookConfig;
      const body = JSON.stringify({ subject: m.subject, text: m.text, ...m.data, sentAt: new Date().toISOString() });
      const secret = channelSecret(ch.id);
      const sig: Record<string, string> = secret ? { 'x-ufop-signature': `sha256=${crypto.createHmac('sha256', secret).update(body).digest('hex')}` } : {};
      await postJson(c.url, body, { ...(c.headers ?? {}), ...sig });
    }
    markDelivery(ch.id, null);
    return { channelId: ch.id, ok: true };
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    markDelivery(ch.id, error);
    return { channelId: ch.id, ok: false, error };
  }
}

/**
 * Send through the outbox: the message is recorded, tried once now (the
 * result is returned), and retried later if that attempt fails.
 */
async function sendQueued(ch: Channel, m: Message, context: string): Promise<Delivery & { outboxId?: string }> {
  if (!ch.enabled) return { channelId: ch.id, ok: false, error: 'Channel is disabled' };
  const entry = addToOutbox(ch.orgId, ch.id, ch.kind, m, context);
  const d = await deliver(ch, m);
  recordOutboxAttempt(entry.id, d.ok ? null : (d.error ?? 'failed'));
  return { ...d, outboxId: entry.id };
}

export async function deliverTo(orgId: string, channelIds: string[], m: Message, context = ''): Promise<Delivery[]> {
  const out: Delivery[] = [];
  for (const id of channelIds) {
    const ch = getChannel(orgId, id);
    if (ch) out.push(await sendQueued(ch, m, context || String(m.data?.event ?? '')));
    else out.push({ channelId: id, ok: false, error: 'Channel not found' });
  }
  return out;
}

/** Retry due outbox messages; returns how many were attempted. */
export async function processOutbox(at = new Date()): Promise<number> {
  let n = 0;
  for (;;) {
    const batch = dueOutbox(at);
    if (!batch.length) break;
    for (const e of batch) {
      const ch = getChannel(e.orgId, e.channelId);
      const d = ch ? await deliver(ch, e.message) : { ok: false, error: 'Channel removed' };
      recordOutboxAttempt(e.id, d.ok ? null : (d.error ?? 'failed'), new Date(Math.max(Date.now(), at.getTime())));
      n++;
    }
  }
  pruneOutbox();
  return n;
}

/** Retry one outbox message now (from the status page); returns its new status. */
export async function sendOutboxNow(orgId: string, id: string): Promise<Delivery | null> {
  const e = getOutboxEntry(orgId, id);
  const ch = e && getChannel(orgId, e.channelId);
  if (!e || !ch) return null;
  const d = await deliver(ch, e.message);
  recordOutboxAttempt(e.id, d.ok ? null : (d.error ?? 'failed'));
  return d;
}

/** Email an invitation through the organization's email channel marked for invitations. */
export async function sendInvitationEmail(orgId: string, to: string, link: string, inviter: string): Promise<{ sent: boolean; error?: string }> {
  const ch = listChannels(orgId).find((c) => c.kind === 'email' && c.enabled && (c.config as EmailConfig).useForInvitations);
  if (!ch) return { sent: false };
  const d = await sendQueued(
    ch,
    {
      to: [to],
      subject: 'You are invited to the UFOP backup console',
      text: `${inviter} invited you to the UFOP backup console.\n\nOpen this link within 7 days to accept:\n${link}\n\nIf you did not expect this, ignore this email.`,
    },
    'invitation',
  );
  return { sent: d.ok, error: d.error };
}
