/**
 * Prometheus metrics (Settings → Advanced, K108 extra): the desktop app can
 * serve `/metrics` on 127.0.0.1 for a local Prometheus or agent to scrape.
 * Off by default; the port and an optional bearer token are configurable.
 * Mirrors Rust `backup_engine::metrics_exporter::{MetricsStatus,
 * MetricsSettingsUpdate}` (`desktop_metrics_status`, `desktop_metrics_configure`).
 */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

export interface MetricsStatus {
  enabled: boolean;
  port: number;
  token_set: boolean;
  listening: boolean;
  url: string | null;
  error: string | null;
}

export interface MetricsSettingsUpdate {
  enabled: boolean;
  port: number | null;
  /** null keeps the current token, "" removes it. */
  token: string | null;
}

export const DEFAULT_METRICS_PORT = 9464;

/** Port validation shared with the backend (1024–65535). */
export function portProblem(raw: string): string | null {
  const n = Number(raw);
  if (!/^\d+$/.test(raw.trim()) || n < 1024 || n > 65535) return "Use a port from 1024 to 65535.";
  return null;
}

const field = "w-full rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 px-2 py-1 text-sm";

export function MetricsExporterSection() {
  const [status, setStatus] = useState<MetricsStatus | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [port, setPort] = useState(String(DEFAULT_METRICS_PORT));
  const [token, setToken] = useState("");
  const [removeToken, setRemoveToken] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const show = (s: MetricsStatus) => {
    setStatus(s);
    setEnabled(s.enabled);
    setPort(String(s.port));
  };

  useEffect(() => {
    if (!isTauriAvailable()) return;
    tauriInvoke<MetricsStatus>("desktop_metrics_status")
      .then(show)
      .catch((e) => setError(errorText(e)));
  }, []);

  if (!isTauriAvailable()) {
    return <p className="pt-3 text-xs text-gray-500">The metrics exporter is available in the desktop app.</p>;
  }

  const problem = portProblem(port) ?? (token && token.trim().length < 8 ? "Use a token of at least 8 characters." : null);

  const save = async () => {
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      const settings: MetricsSettingsUpdate = { enabled, port: Number(port), token: removeToken ? "" : token.trim() ? token.trim() : null };
      show(await tauriInvoke<MetricsStatus>("desktop_metrics_configure", { settings }));
      setToken("");
      setRemoveToken(false);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pt-3 space-y-3 text-sm" data-testid="metrics-exporter">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Serve backup metrics in the Prometheus format at <code>http://127.0.0.1:&lt;port&gt;/metrics</code> for a monitoring agent on this computer. Only
        this computer can connect.
      </p>
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
        Serve Prometheus metrics
      </label>
      <div className="grid grid-cols-2 gap-3">
        <label className="space-y-1">
          <span className="text-xs text-gray-500">Port</span>
          <input aria-label="Metrics port" className={field} inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value)} />
        </label>
        <label className="space-y-1">
          <span className="text-xs text-gray-500">Bearer token (optional)</span>
          <input
            aria-label="Metrics token"
            type="password"
            autoComplete="new-password"
            className={field}
            value={token}
            disabled={removeToken}
            placeholder={status?.token_set ? "Saved — leave empty to keep" : "No token"}
            onChange={(e) => setToken(e.target.value)}
          />
        </label>
      </div>
      {status?.token_set && (
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={removeToken} onChange={(e) => setRemoveToken(e.target.checked)} />
          Remove the saved token
        </label>
      )}
      <div className="flex items-center gap-3">
        <button type="button" onClick={save} disabled={busy} className="text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50">
          Save
        </button>
        <span data-testid="metrics-state" className="text-xs text-gray-500 dark:text-gray-400">
          {status?.listening && status.url ? `Serving ${status.url}${status.token_set ? " (token required)" : ""}` : status?.enabled ? "Not serving" : "Off"}
        </span>
      </div>
      {status?.error && <p className="text-xs text-red-600 dark:text-red-400">{status.error}</p>}
      {error && (
        <p role="alert" className="text-xs text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Prometheus: <code>{`scrape_configs: [{ job_name: ufop, static_configs: [{ targets: ["127.0.0.1:${port}"] }]${status?.token_set ? ", authorization: { credentials: <token> }" : ""} }]`}</code>
      </p>
    </div>
  );
}
