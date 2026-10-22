/**
 * Encryption at rest for secrets the console holds (central credentials,
 * channel secrets, SSO client secrets, TOTP seeds, AI API keys).
 *
 * AES-256-GCM. The key is ADMIN_ENCRYPTION_KEY (32 bytes, base64) when set,
 * otherwise derived from NEXTAUTH_SECRET with HKDF — so rotating
 * NEXTAUTH_SECRET without setting ADMIN_ENCRYPTION_KEY makes stored secrets
 * unreadable (documented in docs/admin-console.md).
 */
import crypto from 'crypto';

let cached: { source: string; key: Buffer } | null = null;

function key(): Buffer {
  const explicit = process.env.ADMIN_ENCRYPTION_KEY;
  const source = explicit ? `k:${explicit}` : `s:${process.env.NEXTAUTH_SECRET ?? ''}`;
  if (cached?.source === source) return cached.key;
  let k: Buffer;
  if (explicit) {
    k = Buffer.from(explicit, 'base64');
    if (k.length !== 32) throw new Error('ADMIN_ENCRYPTION_KEY must be 32 bytes, base64-encoded');
  } else {
    const secret = process.env.NEXTAUTH_SECRET;
    if (!secret) throw new Error('NEXTAUTH_SECRET (or ADMIN_ENCRYPTION_KEY) is required to store secrets');
    k = Buffer.from(crypto.hkdfSync('sha256', secret, 'ufop-admin', 'ufop-admin-secrets-v1', 32));
  }
  cached = { source, key: k };
  return k;
}

/** `v1.<iv>.<tag>.<ciphertext>` (base64url parts). */
export function seal(plain: string): string {
  const iv = crypto.randomBytes(12);
  const c = crypto.createCipheriv('aes-256-gcm', key(), iv);
  const ct = Buffer.concat([c.update(plain, 'utf8'), c.final()]);
  return ['v1', iv.toString('base64url'), c.getAuthTag().toString('base64url'), ct.toString('base64url')].join('.');
}

export function open(sealed: string): string {
  const [v, iv, tag, ct] = sealed.split('.');
  if (v !== 'v1' || !iv || !tag || ct === undefined) throw new Error('Unsupported secret format');
  const d = crypto.createDecipheriv('aes-256-gcm', key(), Buffer.from(iv, 'base64url'));
  d.setAuthTag(Buffer.from(tag, 'base64url'));
  return Buffer.concat([d.update(Buffer.from(ct, 'base64url')), d.final()]).toString('utf8');
}

export function sealOpt(plain: string | null | undefined): string | null {
  return plain ? seal(plain) : null;
}

export function openOpt(sealed: unknown): string | null {
  return typeof sealed === 'string' && sealed ? open(sealed) : null;
}

/** HMAC-SHA256 (hex) with the derived key — signs audit anchors. */
export function sign(data: string): string {
  return crypto.createHmac('sha256', key()).update(data).digest('hex');
}

export function timingSafeEqualHex(a: string, b: string): boolean {
  const x = Buffer.from(a, 'hex');
  const y = Buffer.from(b, 'hex');
  return x.length === y.length && crypto.timingSafeEqual(x, y);
}
