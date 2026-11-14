/**
 * Security audit log section (Settings): integrity verification, signed
 * checkpoints and a JSON-lines export an external party can verify.
 *
 * The log is append-only and hash-chained on the Rust side (#357); this
 * view never edits it.
 */
import { useCallback, useEffect, useState } from "react";
import {
  createAuditAnchor,
  describeIpcError,
  exportAuditLog,
  getAuditLogStatus,
  KEY_SOURCE_LABELS,
  verifyAuditLog,
  type AuditLogStatus,
  type AuditVerification,
} from "@/lib/audit-log";

function shortHash(hash: string | null): string {
  return hash ? `${hash.slice(0, 12)}…` : "—";
}

export function SecurityAuditSection() {
  const [status, setStatus] = useState<AuditLogStatus | null>(null);
  const [result, setResult] = useState<AuditVerification | null>(null);
  const [busy, setBusy] = useState<"verify" | "export" | "anchor" | null>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  const refresh = useCallback(async () => {
    setStatus(await getAuditLogStatus());
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (kind: "verify" | "export" | "anchor", fn: () => Promise<void>) => {
    setBusy(kind);
    setError("");
    setMessage("");
    try {
      await fn();
    } catch (err) {
      setError(describeIpcError(err));
    } finally {
      setBusy(null);
    }
  };

  const handleVerify = () =>
    run("verify", async () => {
      setResult(null);
      setResult(await verifyAuditLog());
      await refresh();
    });

  const handleAnchor = () =>
    run("anchor", async () => {
      const a = await createAuditAnchor();
      setMessage(`Checkpoint #${a.anchor_seq} signed over ${a.count} entries`);
      await refresh();
    });

  const handleExport = () =>
    run("export", async () => {
      const text = await exportAuditLog();
      const lines = text.split("\n").filter((l) => l.trim()).length;
      const blob = new Blob([text], { type: "application/x-ndjson" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `ufop-security-audit-${new Date().toISOString().slice(0, 10)}.jsonl`;
      a.click();
      URL.revokeObjectURL(url);
      setMessage(`Exported ${Math.max(lines - 1, 0)} records (entries and checkpoints)`);
    });

  return (
    <div className="space-y-3 pt-3" data-testid="security-audit-section">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Security events are kept in an append-only, hash-chained log with signed checkpoints.
        Entries cannot be edited or deleted, and any tampering is detected on verification.
      </p>

      {status && (
        <dl className="text-xs grid grid-cols-[auto_1fr] gap-x-3 gap-y-1" data-testid="audit-status">
          <dt className="text-gray-500 dark:text-gray-400">Entries</dt>
          <dd>{status.entries}</dd>
          <dt className="text-gray-500 dark:text-gray-400">Checkpoints</dt>
          <dd>
            {status.anchors}
            {status.unanchored_entries > 0 && ` (${status.unanchored_entries} newer entries not yet checkpointed)`}
          </dd>
          <dt className="text-gray-500 dark:text-gray-400">Signing key</dt>
          <dd className="font-mono">
            {status.key_id ?? "unavailable"}
            {status.key_source && (
              <span className="font-sans text-gray-500 dark:text-gray-400"> · {KEY_SOURCE_LABELS[status.key_source]}</span>
            )}
          </dd>
          {!status.append_only_enforced && (
            <>
              <dt className="text-red-500">Protection</dt>
              <dd className="text-red-500">Append-only triggers are missing</dd>
            </>
          )}
        </dl>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={handleVerify}
          disabled={busy !== null}
          data-testid="audit-verify"
          className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {busy === "verify" ? "Verifying..." : "Verify integrity"}
        </button>
        <button
          type="button"
          onClick={handleExport}
          disabled={busy !== null}
          data-testid="audit-export"
          className="text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
        >
          {busy === "export" ? "Exporting..." : "Export (JSON lines)"}
        </button>
        <button
          type="button"
          onClick={handleAnchor}
          disabled={busy !== null || (status !== null && status.entries === 0)}
          data-testid="audit-anchor"
          className="text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
        >
          {busy === "anchor" ? "Signing..." : "Sign checkpoint now"}
        </button>
      </div>

      {result && (
        <div
          role="status"
          data-testid="audit-verify-result"
          data-ok={result.ok ? "true" : "false"}
          className={`text-xs rounded border px-3 py-2 space-y-1 ${
            result.ok
              ? "border-green-300 bg-green-50 text-green-800 dark:border-green-800 dark:bg-green-900/20 dark:text-green-300"
              : "border-red-300 bg-red-50 text-red-800 dark:border-red-800 dark:bg-red-900/20 dark:text-red-300"
          }`}
        >
          <p className="font-medium">
            {result.ok ? "Audit log intact" : "Audit log integrity check FAILED"}
          </p>
          <p>
            {result.entries_checked} entries verified · {result.anchors_verified} of {result.anchors_total} checkpoints
            verified
            {result.ok && result.last_hash && <> · head {shortHash(result.last_hash)}</>}
          </p>
          {result.first_bad_sequence !== null && <p>First bad entry: #{result.first_bad_sequence}</p>}
          {result.first_bad_anchor !== null && <p>First bad checkpoint: #{result.first_bad_anchor}</p>}
          {result.reason && <p className="break-words">{result.reason}</p>}
        </div>
      )}

      {error && <p className="text-xs text-red-500" data-testid="audit-error">{error}</p>}
      {message && <p className="text-xs text-green-600 dark:text-green-400">{message}</p>}
    </div>
  );
}

export default SecurityAuditSection;
