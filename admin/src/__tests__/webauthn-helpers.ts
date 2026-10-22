/** A tiny CBOR encoder and a software WebAuthn authenticator for tests. */
import crypto from "crypto";

function head(major: number, n: number): Buffer {
  if (n < 24) return Buffer.from([(major << 5) | n]);
  if (n < 256) return Buffer.from([(major << 5) | 24, n]);
  const b = Buffer.alloc(3);
  b[0] = (major << 5) | 25;
  b.writeUInt16BE(n, 1);
  return b;
}
export function cbor(v: unknown): Buffer {
  if (typeof v === "number") return v >= 0 ? head(0, v) : head(1, -1 - v);
  if (typeof v === "string")
    return Buffer.concat([head(3, Buffer.byteLength(v)), Buffer.from(v)]);
  if (v instanceof Uint8Array)
    return Buffer.concat([head(2, v.length), Buffer.from(v)]);
  if (v instanceof Map)
    return Buffer.concat([
      head(5, v.size),
      ...[...v].flatMap(([k, x]) => [cbor(k), cbor(x)]),
    ]);
  throw new Error("unsupported");
}

export const ORIGIN = "http://localhost:3001";
const sha = (b: Buffer | string) =>
  crypto.createHash("sha256").update(b).digest();
export const b64 = (b: Buffer) => b.toString("base64url");

export class Authenticator {
  readonly id = crypto.randomBytes(16);
  counter = 0;
  private keys: crypto.KeyPairKeyObjectResult;
  constructor(readonly kind: "es256" | "ed25519" = "es256") {
    this.keys =
      kind === "es256"
        ? crypto.generateKeyPairSync("ec", { namedCurve: "P-256" })
        : crypto.generateKeyPairSync("ed25519");
  }
  cose(): Map<number, unknown> {
    const j = this.keys.publicKey.export({ format: "jwk" });
    return this.kind === "es256"
      ? new Map<number, unknown>([
          [1, 2],
          [3, -7],
          [-1, 1],
          [-2, Buffer.from(j.x!, "base64url")],
          [-3, Buffer.from(j.y!, "base64url")],
        ])
      : new Map<number, unknown>([
          [1, 1],
          [3, -8],
          [-1, 6],
          [-2, Buffer.from(j.x!, "base64url")],
        ]);
  }
  register(
    challenge: string,
    o: { origin?: string; rpId?: string; flags?: number } = {},
  ) {
    const clientData = Buffer.from(
      JSON.stringify({
        type: "webauthn.create",
        challenge,
        origin: o.origin ?? ORIGIN,
      }),
    );
    const credData = Buffer.concat([
      Buffer.alloc(16),
      Buffer.from([0, this.id.length]),
      this.id,
      cbor(this.cose()),
    ]);
    const authData = Buffer.concat([
      sha(o.rpId ?? "localhost"),
      Buffer.from([o.flags ?? 0x45]),
      Buffer.alloc(4),
      credData,
    ]);
    const att = cbor(
      new Map<string, unknown>([
        ["fmt", "none"],
        ["attStmt", new Map()],
        ["authData", authData],
      ]),
    );
    return {
      id: b64(this.id),
      response: {
        clientDataJSON: b64(clientData),
        attestationObject: b64(att),
      },
      transports: ["internal"],
    };
  }
  assert(
    challenge: string,
    o: {
      origin?: string;
      rpId?: string;
      flags?: number;
      counter?: number;
      tamper?: boolean;
    } = {},
  ) {
    this.counter = o.counter ?? this.counter + 1;
    const clientData = Buffer.from(
      JSON.stringify({
        type: "webauthn.get",
        challenge,
        origin: o.origin ?? ORIGIN,
      }),
    );
    const cnt = Buffer.alloc(4);
    cnt.writeUInt32BE(this.counter);
    const authData = Buffer.concat([
      sha(o.rpId ?? "localhost"),
      Buffer.from([o.flags ?? 0x05]),
      cnt,
    ]);
    const data = Buffer.concat([authData, sha(clientData)]);
    let sig =
      this.kind === "es256"
        ? crypto.sign("sha256", data, this.keys.privateKey)
        : crypto.sign(null, data, this.keys.privateKey);
    if (o.tamper)
      sig = Buffer.from(sig.map((x, i) => (i === sig.length - 1 ? x ^ 1 : x)));
    return {
      id: b64(this.id),
      response: {
        clientDataJSON: b64(clientData),
        authenticatorData: b64(authData),
        signature: b64(sig),
      },
    };
  }
}
