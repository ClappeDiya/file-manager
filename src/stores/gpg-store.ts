/**
 * OpenPGP (GnuPG) actions. Types mirror `ufop_core::gpg` and
 * `gpg_commands.rs`; the dialog's open state lives here.
 */
import { create } from "zustand";
import { tauriInvoke } from "@/hooks/use-tauri";

export interface GpgKey {
  fingerprint: string;
  key_id: string;
  user_ids: string[];
  created: number | null;
  expires: number | null;
  expired: boolean;
  revoked: boolean;
  can_encrypt: boolean;
  can_sign: boolean;
  secret: boolean;
  algorithm: string;
}

export interface SignatureCheck {
  valid: boolean;
  key_id: string | null;
  fingerprint: string | null;
  user_id: string | null;
  signed_at: number | null;
  key_expired: boolean;
  key_revoked: boolean;
  missing_key: boolean;
  message: string;
}

export interface GpgOutcome {
  input: string;
  output: string | null;
  signature: SignatureCheck | null;
  error: string | null;
  /** Decrypted without integrity protection (a legacy file, allowed explicitly). */
  unprotected: boolean;
  /** Refused for lacking integrity protection; can be decrypted anyway. */
  needs_unprotected: boolean;
}

export type KeyAlgorithm = "ed25519" | "rsa4096";

export interface NewKey {
  name: string;
  email: string;
  algorithm: KeyAlgorithm;
  /** `2y`, `90d`, `2030-12-31` or `never`. */
  expires: string;
}

export interface GpgStatus {
  available: boolean;
  version: string | null;
  error: string | null;
}

export interface EncryptOptions {
  recipients: string[];
  armor: boolean;
  sign_with: string | null;
}

export type GpgMode = "encrypt" | "decrypt" | "recrypt" | "sign" | "verify";

export const gpg = {
  status: () => tauriInvoke<GpgStatus>("gpg_status"),
  keys: () => tauriInvoke<GpgKey[]>("gpg_list_keys"),
  importKeys: (src: { path?: string; text?: string }) => tauriInvoke<number>("gpg_import_keys", { path: src.path ?? null, text: src.text ?? null }),
  exportPublic: (key: string) => tauriInvoke<string>("gpg_export_public", { key }),
  encrypt: (paths: string[], options: EncryptOptions, passphrase: string | null) => tauriInvoke<GpgOutcome[]>("gpg_encrypt_files", { paths, options, passphrase }),
  decrypt: (paths: string[], passphrase: string | null, allowUnprotected = false) => tauriInvoke<GpgOutcome[]>("gpg_decrypt_files", { paths, passphrase, allowUnprotected }),
  recrypt: (paths: string[], options: EncryptOptions, oldPassphrase: string | null, newPassphrase: string | null, allowUnprotected = false) =>
    tauriInvoke<GpgOutcome[]>("gpg_recrypt_files", { paths, options, oldPassphrase, newPassphrase, allowUnprotected }),
  generateKey: (key: NewKey, passphrase: string | null) => tauriInvoke<string>("gpg_generate_key", { key, passphrase }),
  sign: (paths: string[], key: string | null, detached: boolean, armor: boolean, passphrase: string | null) =>
    tauriInvoke<GpgOutcome[]>("gpg_sign_files", { paths, key, detached, armor, passphrase }),
  verify: (path: string) => tauriInvoke<SignatureCheck>("gpg_verify", { path }),
};

const ENCRYPTED = /\.(gpg|pgp|asc)$/i;
const SIGNATURE = /\.(sig|asc)$/i;
export const looksEncrypted = (name: string) => ENCRYPTED.test(name);
export const looksLikeSignature = (name: string) => SIGNATURE.test(name);

/** Days until a key expires (negative = expired), null = never. */
export function daysToExpiry(k: GpgKey, now = Date.now()): number | null {
  return k.expires === null ? null : Math.floor((k.expires * 1000 - now) / 86_400_000);
}

export const EXPIRY_WARNING_DAYS = 30;

export function keyLabel(k: GpgKey): string {
  return `${k.user_ids[0] ?? "(no user id)"} · ${k.key_id.slice(-8)}`;
}

interface GpgDialogState {
  mode: GpgMode | null;
  paths: string[];
  open: (mode: GpgMode, paths: string[]) => void;
  close: () => void;
}

export const useGpgDialog = create<GpgDialogState>((set) => ({
  mode: null,
  paths: [],
  open: (mode, paths) => set({ mode, paths }),
  close: () => set({ mode: null, paths: [] }),
}));
