/** Browser side of passkeys: JSON (base64url) ⇄ the WebAuthn API's ArrayBuffers. */

const toBuf = (s: string): ArrayBuffer => {
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  const out = new Uint8Array(b.length);
  for (let i = 0; i < b.length; i++) out[i] = b.charCodeAt(i);
  return out.buffer;
};

const fromBuf = (b: ArrayBuffer): string => {
  let s = '';
  for (const x of new Uint8Array(b)) s += String.fromCharCode(x);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
};

export const passkeysSupported = () => typeof window !== 'undefined' && !!window.PublicKeyCredential && !!navigator.credentials;

interface CreationJson {
  rp: { id: string; name: string };
  user: { id: string; name: string; displayName: string };
  challenge: string;
  pubKeyCredParams: { type: 'public-key'; alg: number }[];
  excludeCredentials: { type: 'public-key'; id: string; transports?: string[] }[];
  authenticatorSelection: AuthenticatorSelectionCriteria;
  attestation: AttestationConveyancePreference;
  timeout: number;
}

export async function createPasskey(o: CreationJson) {
  const cred = (await navigator.credentials.create({
    publicKey: {
      ...o,
      challenge: toBuf(o.challenge),
      user: { ...o.user, id: toBuf(o.user.id) },
      excludeCredentials: o.excludeCredentials.map((c) => ({ type: c.type, id: toBuf(c.id), transports: c.transports as AuthenticatorTransport[] })),
    },
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error('No passkey was created');
  const r = cred.response as AuthenticatorAttestationResponse;
  return { id: cred.id, response: { clientDataJSON: fromBuf(r.clientDataJSON), attestationObject: fromBuf(r.attestationObject) }, transports: r.getTransports?.() ?? [] };
}

export async function getPasskey(o: { challenge: string; rpId: string; userVerification: UserVerificationRequirement; timeout: number }) {
  const cred = (await navigator.credentials.get({ publicKey: { ...o, challenge: toBuf(o.challenge) } })) as PublicKeyCredential | null;
  if (!cred) throw new Error('No passkey was chosen');
  const r = cred.response as AuthenticatorAssertionResponse;
  return { id: cred.id, response: { clientDataJSON: fromBuf(r.clientDataJSON), authenticatorData: fromBuf(r.authenticatorData), signature: fromBuf(r.signature) } };
}
