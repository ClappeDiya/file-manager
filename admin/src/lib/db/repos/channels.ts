/**
 * Notification channels (email, Slack, generic webhook). Secrets — SMTP
 * password, Slack webhook URL, webhook signing secret — are sealed at rest
 * and never returned by the API.
 */
import { db, newId, now, parseJson, type Row } from '../index';
import { openOpt, sealOpt } from '../../crypto';
import { OrgError } from './orgs';

export type ChannelKind = 'email' | 'slack' | 'webhook';

export interface EmailConfig {
  host: string;
  port: number;
  /** `starttls` (587), `tls` (465) or `none` (local relay). */
  security: 'starttls' | 'tls' | 'none';
  username: string;
  from: string;
  to: string[];
  /** Trusted CA / self-signed server certificate (PEM). */
  caPem?: string | null;
  /** Use this channel for invitation emails too. */
  useForInvitations?: boolean;
}

export interface WebhookConfig {
  url: string;
  /** Extra headers (non-secret). */
  headers?: Record<string, string>;
}

export interface Channel {
  id: string;
  orgId: string;
  name: string;
  kind: ChannelKind;
  config: EmailConfig | WebhookConfig | Record<string, never>;
  hasSecret: boolean;
  enabled: boolean;
  createdAt: string;
  lastError: string | null;
  lastSentAt: string | null;
}

function fromRow(r: Row): Channel {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    kind: r.kind as ChannelKind,
    config: parseJson(r.config, {}),
    hasSecret: !!r.secret_ciphertext,
    enabled: Boolean(r.enabled),
    createdAt: r.created_at as string,
    lastError: (r.last_error as string) ?? null,
    lastSentAt: (r.last_sent_at as string) ?? null,
  };
}

export function listChannels(orgId: string): Channel[] {
  return (db().prepare('SELECT * FROM notification_channels WHERE org_id = ? ORDER BY lower(name)').all(orgId) as Row[]).map(fromRow);
}

export function getChannel(orgId: string, id: string): Channel | null {
  const r = db().prepare('SELECT * FROM notification_channels WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** The decrypted secret, for delivery only. */
export function channelSecret(id: string): string | null {
  const r = db().prepare('SELECT secret_ciphertext FROM notification_channels WHERE id = ?').get(id) as Row | undefined;
  return openOpt(r?.secret_ciphertext);
}

function validate(kind: ChannelKind, config: Record<string, unknown>, secret: string | null | undefined, existing: boolean) {
  if (kind === 'email') {
    const c = config as unknown as EmailConfig;
    if (!c.host?.trim()) throw new OrgError('Enter the SMTP server');
    if (!c.from?.includes('@')) throw new OrgError('Enter the sender address');
    if (!Array.isArray(c.to)) throw new OrgError('Enter at least one recipient');
    if (!['starttls', 'tls', 'none'].includes(c.security)) throw new OrgError('Choose the connection security');
  } else if (kind === 'webhook') {
    const c = config as unknown as WebhookConfig;
    try {
      const u = new URL(c.url);
      if (!['https:', 'http:'].includes(u.protocol)) throw new Error();
    } catch {
      throw new OrgError('Enter a valid http(s) webhook URL');
    }
  } else if (kind === 'slack') {
    if (!secret && !existing) throw new OrgError('Paste the Slack incoming-webhook URL');
    if (secret && !/^https:\/\//.test(secret)) throw new OrgError('The Slack webhook URL must start with https://');
  } else {
    throw new OrgError('Unknown channel type');
  }
}

export interface ChannelInput {
  name: string;
  kind: ChannelKind;
  config: Record<string, unknown>;
  /** SMTP password, Slack webhook URL or webhook HMAC secret; null clears, undefined keeps. */
  secret?: string | null;
  enabled?: boolean;
}

export function createChannel(orgId: string, c: ChannelInput): Channel {
  if (!c.name?.trim()) throw new OrgError('Give the channel a name');
  validate(c.kind, c.config ?? {}, c.secret, false);
  const id = newId('chn');
  db().prepare('INSERT INTO notification_channels (id, org_id, name, kind, config, secret_ciphertext, enabled, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)').run(id, orgId, c.name.trim(), c.kind, JSON.stringify(c.config ?? {}), sealOpt(c.secret), c.enabled === false ? 0 : 1, now());
  return getChannel(orgId, id)!;
}

export function updateChannel(orgId: string, id: string, c: Partial<ChannelInput>): Channel | null {
  const cur = getChannel(orgId, id);
  if (!cur) return null;
  const config = c.config ?? (cur.config as Record<string, unknown>);
  validate(cur.kind, config, c.secret, cur.hasSecret);
  db().prepare('UPDATE notification_channels SET name = ?, config = ?, enabled = ? WHERE id = ?').run(c.name?.trim() || cur.name, JSON.stringify(config), c.enabled === undefined ? (cur.enabled ? 1 : 0) : c.enabled ? 1 : 0, id);
  if (c.secret !== undefined) db().prepare('UPDATE notification_channels SET secret_ciphertext = ? WHERE id = ?').run(sealOpt(c.secret), id);
  return getChannel(orgId, id);
}

export function deleteChannel(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM notification_channels WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

export function markDelivery(id: string, error: string | null): void {
  if (error) db().prepare('UPDATE notification_channels SET last_error = ? WHERE id = ?').run(error.slice(0, 500), id);
  else db().prepare('UPDATE notification_channels SET last_error = NULL, last_sent_at = ? WHERE id = ?').run(now(), id);
}
