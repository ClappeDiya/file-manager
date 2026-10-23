/**
 * Client addresses and CIDR allowlists (no Node APIs: also used by the
 * middleware).
 *
 * The console sits behind a reverse proxy, so the client address comes from
 * X-Forwarded-For. Only the entries added by trusted proxies can be believed:
 * with TRUSTED_PROXY_HOPS=n (default 1) the address is the n-th entry from
 * the right, which a client cannot forge by sending its own header. The
 * console must not be reachable except through those proxies.
 */

/** IPv4 or IPv6 text → bytes (4 or 16), or null. IPv4-mapped IPv6 becomes IPv4. */
export function parseIp(text: string): Uint8Array | null {
  const s = text
    .trim()
    .replace(/^\[|\]$/g, '')
    .replace(/%.*$/, '');
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(s)) {
    const parts = s.split('.').map(Number);
    return parts.every((p) => p <= 255) ? new Uint8Array(parts) : null;
  }
  if (!s.includes(':') || !/^[0-9a-fA-F:.]+$/.test(s)) return null;
  let tail: number[] = [];
  let body = s;
  const v4 = /(\d{1,3}(\.\d{1,3}){3})$/.exec(s);
  if (v4) {
    const b = parseIp(v4[1]);
    if (!b) return null;
    tail = [(b[0] << 8) | b[1], (b[2] << 8) | b[3]];
    body = s.slice(0, -v4[1].length);
    if (body.endsWith(':') && !body.endsWith('::')) body = body.slice(0, -1);
  }
  const halves = body.split('::');
  if (halves.length > 2) return null;
  const group = (h: string) => (h ? h.split(':') : []);
  const head = group(halves[0]);
  const rest = halves.length === 2 ? group(halves[1]) : [];
  if ([...head, ...rest].some((g) => !/^[0-9a-fA-F]{1,4}$/.test(g))) return null;
  const total = head.length + rest.length + tail.length;
  if (halves.length === 1 ? total !== 8 : total > 7) return null;
  const words = [...head.map((g) => parseInt(g, 16)), ...Array(8 - total).fill(0), ...rest.map((g) => parseInt(g, 16)), ...tail];
  const out = new Uint8Array(16);
  words.forEach((w, i) => {
    out[i * 2] = w >> 8;
    out[i * 2 + 1] = w & 0xff;
  });
  const mapped = out.slice(0, 10).every((b) => b === 0) && out[10] === 0xff && out[11] === 0xff;
  return mapped ? out.slice(12) : out;
}

export interface Cidr {
  bytes: Uint8Array;
  prefix: number;
  text: string;
}

/** "10.0.0.0/8", "2001:db8::/32" or a single address. Throws on bad input. */
export function parseCidr(text: string): Cidr {
  const t = text.trim();
  const [addr, len, extra] = t.split('/');
  const bytes = parseIp(addr ?? '');
  if (!bytes || extra !== undefined) throw new Error(`"${t}" is not an IP address or CIDR range`);
  const max = bytes.length * 8;
  const prefix = len === undefined ? max : /^\d{1,3}$/.test(len) ? Number(len) : NaN;
  if (!(prefix >= 0 && prefix <= max)) throw new Error(`"${t}": the prefix length is 0 to ${max}`);
  return { bytes, prefix, text: t };
}

export function cidrContains(c: Cidr, ip: Uint8Array): boolean {
  if (c.bytes.length !== ip.length) return false;
  for (let bit = 0; bit < c.prefix; bit += 8) {
    const n = Math.min(8, c.prefix - bit);
    const mask = (0xff << (8 - n)) & 0xff;
    if ((c.bytes[bit / 8] & mask) !== (ip[bit / 8] & mask)) return false;
  }
  return true;
}

/** Validate a list of ranges; returns them trimmed and without blanks or duplicates. */
export function normalizeAllowlist(list: unknown): string[] {
  if (!Array.isArray(list) || list.some((x) => typeof x !== 'string')) throw new Error('The allowlist is a list of IP addresses or CIDR ranges');
  const out = [...new Set((list as string[]).map((x) => x.trim()).filter(Boolean))];
  if (out.length > 200) throw new Error('At most 200 ranges');
  out.forEach(parseCidr);
  return out;
}

/** Whether an address is allowed: an empty list allows everything; an unknown address nothing. */
export function ipAllowed(list: string[], ip: string): boolean {
  if (!list.length) return true;
  const b = parseIp(ip);
  return !!b && list.some((c) => cidrContains(parseCidr(c), b));
}

function trustedHops(): number {
  const n = Number(process.env.TRUSTED_PROXY_HOPS ?? 1);
  return Number.isInteger(n) && n >= 1 ? n : 1;
}

/** The client address the trusted proxies saw ('unknown' without one). */
export function clientIp(req: { headers: Headers }): string {
  const xff = req.headers.get('x-forwarded-for');
  if (xff) {
    const parts = xff
      .split(',')
      .map((p) => p.trim())
      .filter(Boolean);
    const ip = parts[Math.max(0, parts.length - trustedHops())];
    if (ip) return ip;
  }
  return req.headers.get('x-real-ip')?.trim() || 'unknown';
}
