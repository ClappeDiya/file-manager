/**
 * Tamper-evident security audit log (#357) — IPC types and calls.
 *
 * Types mirror the Rust serde output of `security::audit_chain` and
 * `security::audit` exactly (snake_case fields, `Option<T>` → `T | null`).
 */
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";

/** `AuditVerification` (security/audit_chain.rs). */
export interface AuditVerification {
  ok: boolean;
  entries_checked: number;
  first_bad_sequence: number | null;
  first_bad_anchor: number | null;
  reason: string | null;
  anchors_verified: number;
  anchors_total: number;
  last_sequence: number | null;
  last_hash: string | null;
  append_only_enforced: boolean | null;
  key_id: string | null;
  verified_at: string;
}

/** `AuditAnchor` (security/audit_chain.rs). */
export interface AuditAnchor {
  anchor_seq: number;
  last_sequence: number;
  last_hash: string;
  count: number;
  created_at: string;
  key_id: string;
  algorithm: string;
  signature: string;
}

/** `KeySource` (security/audit.rs). */
export type AuditKeySource =
  | "environment"
  | "os_keychain"
  | "settings_store_encrypted"
  | "settings_store_plain"
  | "ephemeral";

/** `AuditLogStatus` (security/audit.rs). */
export interface AuditLogStatus {
  entries: number;
  last_sequence: number | null;
  last_hash: string | null;
  anchors: number;
  last_anchor: AuditAnchor | null;
  unanchored_entries: number;
  append_only_enforced: boolean;
  key_id: string | null;
  public_key: string | null;
  key_source: AuditKeySource | null;
}

export const KEY_SOURCE_LABELS: Record<AuditKeySource, string> = {
  environment: "environment variable",
  os_keychain: "OS keychain",
  settings_store_encrypted: "encrypted settings store",
  settings_store_plain: "settings store (unencrypted)",
  ephemeral: "this session only",
};

/** AppError arrives as `{ message, advice }`; anything else is stringified. */
export function describeIpcError(err: unknown): string {
  if (err && typeof err === "object" && "message" in err) {
    const { message, advice } = err as { message?: unknown; advice?: unknown };
    return [message, advice].filter((p) => typeof p === "string" && p).join(" ");
  }
  return String(err);
}

export function verifyAuditLog(): Promise<AuditVerification> {
  return tauriInvoke<AuditVerification>("verify_audit_log");
}

/** JSON lines: header (with public key), entries with hashes, anchors. */
export function exportAuditLog(): Promise<string> {
  return tauriInvoke<string>("export_audit_log");
}

export function createAuditAnchor(): Promise<AuditAnchor> {
  return tauriInvoke<AuditAnchor>("create_audit_anchor");
}

/** `null` outside Tauri or when the status cannot be read. */
export function getAuditLogStatus(): Promise<AuditLogStatus | null> {
  return tauriInvokeSafe<AuditLogStatus | null>("get_audit_log_status", undefined, null);
}
