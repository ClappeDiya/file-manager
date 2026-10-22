/**
 * Minimal CBOR (RFC 8949) decoder for WebAuthn: unsigned/negative
 * integers, byte and text strings, arrays, maps, booleans and null.
 * Indefinite lengths, tags and floats are not needed and are rejected.
 */
export type CborValue = number | bigint | Uint8Array | string | boolean | null | CborValue[] | Map<CborValue, CborValue>;

export class CborError extends Error {}

export function decodeCbor(buf: Uint8Array, opts: { allowTrailing?: boolean } = {}): { value: CborValue; length: number } {
  let pos = 0;
  const need = (n: number) => {
    if (pos + n > buf.length) throw new CborError('Truncated CBOR');
  };
  const readLength = (info: number): number => {
    if (info < 24) return info;
    const sizes: Record<number, number> = { 24: 1, 25: 2, 26: 4, 27: 8 };
    const n = sizes[info];
    if (!n) throw new CborError(`Unsupported CBOR length encoding ${info}`);
    need(n);
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 256 + buf[pos + i];
    pos += n;
    if (!Number.isSafeInteger(v)) throw new CborError('CBOR integer too large');
    return v;
  };
  const item = (depth: number): CborValue => {
    if (depth > 32) throw new CborError('CBOR nested too deeply');
    need(1);
    const b = buf[pos++];
    const major = b >> 5;
    const info = b & 31;
    switch (major) {
      case 0:
        return readLength(info);
      case 1:
        return -1 - readLength(info);
      case 2: {
        const n = readLength(info);
        need(n);
        const v = buf.slice(pos, pos + n);
        pos += n;
        return v;
      }
      case 3: {
        const n = readLength(info);
        need(n);
        const v = new TextDecoder('utf-8', { fatal: true }).decode(buf.subarray(pos, pos + n));
        pos += n;
        return v;
      }
      case 4: {
        const n = readLength(info);
        const a: CborValue[] = [];
        for (let i = 0; i < n; i++) a.push(item(depth + 1));
        return a;
      }
      case 5: {
        const n = readLength(info);
        const m = new Map<CborValue, CborValue>();
        for (let i = 0; i < n; i++) {
          const k = item(depth + 1);
          m.set(k, item(depth + 1));
        }
        return m;
      }
      case 7:
        if (info === 20) return false;
        if (info === 21) return true;
        if (info === 22) return null;
        throw new CborError(`Unsupported CBOR simple value ${info}`);
      default:
        throw new CborError(`Unsupported CBOR major type ${major}`);
    }
  };
  const value = item(0);
  if (!opts.allowTrailing && pos !== buf.length) throw new CborError('Trailing bytes after CBOR');
  return { value, length: pos };
}
