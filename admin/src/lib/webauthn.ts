/**
 * WebAuthn (passkeys) relying-party checks on Node's crypto.
 *
 * Registration accepts any attestation format without trusting it
 * (attestation "none" is requested): what matters is the RP ID hash, the
 * user-presence flag, the challenge and origin, and the credential public
 * key (COSE ES256, RS256 or EdDSA). Authentication verifies the signature
 * over authenticatorData ‖ SHA-256(clientDataJSON) and the signature
 * counter (a counter that goes backwards suggests a cloned key).
 */
import crypto from 'crypto';
import { decodeCbor, type CborValue } from './cbor';

export const ALGS = { ES256: -7, EdDSA: -8, RS256: -257 } as const;

export class WebAuthnError extends Error {}

export const b64url = (b: Uint8Array | Buffer) => Buffer.from(b).toString('base64url');
export const fromB64url = (s: string) => new Uint8Array(Buffer.from(s, 'base64url'));

export interface ClientData {
  type: string;
  challenge: string;
  origin: string;
  crossOrigin?: boolean;
}

export function parseClientData(json: Uint8Array, expect: { type: 'webauthn.create' | 'webauthn.get'; challenge: string; origin: string }): ClientData {
  let c: ClientData;
  try {
    c = JSON.parse(Buffer.from(json).toString('utf8')) as ClientData;
  } catch {
    throw new WebAuthnError('clientDataJSON is not valid JSON');
  }
  if (c.type !== expect.type) throw new WebAuthnError(`Unexpected ceremony type ${c.type}`);
  if (c.challenge !== expect.challenge) throw new WebAuthnError('The challenge does not match');
  if (c.origin !== expect.origin) throw new WebAuthnError(`Unexpected origin ${c.origin}`);
  if (c.crossOrigin) throw new WebAuthnError('Cross-origin ceremonies are not allowed');
  return c;
}

export interface AuthData {
  rpIdHash: Uint8Array;
  userPresent: boolean;
  userVerified: boolean;
  signCount: number;
  credential?: { aaguid: string; id: Uint8Array; publicKey: Map<CborValue, CborValue> };
}

export function parseAuthData(d: Uint8Array): AuthData {
  if (d.length < 37) throw new WebAuthnError('authenticatorData is too short');
  const flags = d[32];
  const signCount = Buffer.from(d.subarray(33, 37)).readUInt32BE(0);
  const out: AuthData = { rpIdHash: d.subarray(0, 32), userPresent: !!(flags & 0x01), userVerified: !!(flags & 0x04), signCount };
  if (flags & 0x40) {
    if (d.length < 55) throw new WebAuthnError('Attested credential data is truncated');
    const aaguid = Buffer.from(d.subarray(37, 53)).toString('hex');
    const len = (d[53] << 8) | d[54];
    const id = d.subarray(55, 55 + len);
    if (id.length !== len) throw new WebAuthnError('Credential id is truncated');
    const { value } = decodeCbor(d.subarray(55 + len), { allowTrailing: true });
    if (!(value instanceof Map)) throw new WebAuthnError('The credential public key is not a COSE map');
    out.credential = { aaguid, id, publicKey: value };
  }
  return out;
}

const bytes = (m: Map<CborValue, CborValue>, k: number) => {
  const v = m.get(k);
  if (!(v instanceof Uint8Array)) throw new WebAuthnError(`COSE key parameter ${k} is missing`);
  return Buffer.from(v).toString('base64url');
};

/** COSE public key → (SPKI PEM, alg). */
export function coseToPem(cose: Map<CborValue, CborValue>): { pem: string; alg: number } {
  const kty = cose.get(1);
  const alg = Number(cose.get(3));
  let jwk: crypto.JsonWebKey;
  if (kty === 2 && alg === ALGS.ES256) {
    if (cose.get(-1) !== 1) throw new WebAuthnError('Only P-256 EC keys are supported');
    jwk = { kty: 'EC', crv: 'P-256', x: bytes(cose, -2), y: bytes(cose, -3) };
  } else if (kty === 3 && alg === ALGS.RS256) {
    jwk = { kty: 'RSA', n: bytes(cose, -1), e: bytes(cose, -2) };
  } else if (kty === 1 && alg === ALGS.EdDSA) {
    if (cose.get(-1) !== 6) throw new WebAuthnError('Only Ed25519 OKP keys are supported');
    jwk = { kty: 'OKP', crv: 'Ed25519', x: bytes(cose, -2) };
  } else {
    throw new WebAuthnError(`Unsupported key type ${String(kty)} / algorithm ${alg}`);
  }
  const key = crypto.createPublicKey({ key: jwk, format: 'jwk' });
  return { pem: key.export({ type: 'spki', format: 'pem' }).toString(), alg };
}

export function verifySignature(pem: string, alg: number, data: Buffer, sig: Uint8Array): boolean {
  const key = crypto.createPublicKey(pem);
  if (alg === ALGS.ES256) return crypto.verify('sha256', data, { key, dsaEncoding: 'der' }, sig);
  if (alg === ALGS.RS256) return crypto.verify('sha256', data, key, sig);
  if (alg === ALGS.EdDSA) return crypto.verify(null, data, key, sig);
  return false;
}

const sha256 = (b: Uint8Array | string) => crypto.createHash('sha256').update(b).digest();

export interface RegistrationInput {
  clientDataJSON: string;
  attestationObject: string;
}

/** Check a registration response; returns the credential to store. */
export function verifyRegistration(r: RegistrationInput, expect: { challenge: string; origin: string; rpId: string; requireUserVerification?: boolean }) {
  const clientData = fromB64url(r.clientDataJSON);
  parseClientData(clientData, { type: 'webauthn.create', ...expect });
  const { value } = decodeCbor(fromB64url(r.attestationObject));
  if (!(value instanceof Map)) throw new WebAuthnError('attestationObject is not a map');
  const authData = value.get('authData');
  if (!(authData instanceof Uint8Array)) throw new WebAuthnError('authData is missing');
  const a = parseAuthData(authData);
  if (!sha256(expect.rpId).equals(Buffer.from(a.rpIdHash))) throw new WebAuthnError('The credential is for another site');
  if (!a.userPresent) throw new WebAuthnError('User presence was not confirmed');
  if (expect.requireUserVerification && !a.userVerified) throw new WebAuthnError('User verification was not performed');
  if (!a.credential) throw new WebAuthnError('No credential in the response');
  const { pem, alg } = coseToPem(a.credential.publicKey);
  return { credentialId: b64url(a.credential.id), publicKey: pem, alg, signCount: a.signCount, aaguid: a.credential.aaguid, userVerified: a.userVerified };
}

export interface AssertionInput {
  clientDataJSON: string;
  authenticatorData: string;
  signature: string;
}

/** Check an authentication response against a stored credential. */
export function verifyAssertion(r: AssertionInput, cred: { publicKey: string; alg: number; signCount: number }, expect: { challenge: string; origin: string; rpId: string }) {
  const clientData = fromB64url(r.clientDataJSON);
  parseClientData(clientData, { type: 'webauthn.get', ...expect });
  const authData = fromB64url(r.authenticatorData);
  const a = parseAuthData(authData);
  if (!sha256(expect.rpId).equals(Buffer.from(a.rpIdHash))) throw new WebAuthnError('The assertion is for another site');
  if (!a.userPresent) throw new WebAuthnError('User presence was not confirmed');
  const signed = Buffer.concat([Buffer.from(authData), sha256(clientData)]);
  if (!verifySignature(cred.publicKey, cred.alg, signed, fromB64url(r.signature))) throw new WebAuthnError('The signature is not valid');
  // Authenticators that count must count upwards.
  if ((a.signCount > 0 || cred.signCount > 0) && a.signCount <= cred.signCount) throw new WebAuthnError('The signature counter went backwards; the passkey may have been copied');
  return { signCount: a.signCount, userVerified: a.userVerified };
}

export function newChallenge(): string {
  return b64url(crypto.randomBytes(32));
}
