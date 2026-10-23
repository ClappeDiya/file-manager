/**
 * Log retention and diagnostics bundles (Settings → Logging).
 *
 * A bundle is a zip with recent logs, versions and OS details; secrets,
 * e-mail addresses and folder names are scrubbed by the backend. It can be
 * saved to a folder or, on a machine registered with a management console,
 * sent there directly.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { useFleetAgentStore } from "@/stores/fleet-agent-store";
import { DEFAULT_RETENTION, type LogRetention, type SavedBundle } from "@/lib/diagnostics";

const inputCls = "w-24 text-sm px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-transparent";
const btnCls = "text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50";

export function DiagnosticsSection() {
  const [retention, setRetention] = useState<LogRetention>(DEFAULT_RETENTION);
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState<"retention" | "save" | "send" | null>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const registered = useFleetAgentStore((s) => s.status.registered);
  const refreshAgent = useFleetAgentStore((s) => s.refresh);

  useEffect(() => {
    void tauriInvokeSafe<LogRetention>("get_log_retention", undefined, DEFAULT_RETENTION).then(setRetention);
    void refreshAgent();
  }, [refreshAgent]);

  const run = async (kind: typeof busy, fn: () => Promise<string>) => {
    setBusy(kind);
    setError("");
    setMessage("");
    try {
      setMessage(await fn());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const saveRetention = () =>
    run("retention", async () => {
      const removed = await tauriInvoke<number>("set_log_retention", { retention });
      return removed > 0 ? `Retention saved; removed ${removed} old log file${removed === 1 ? "" : "s"}` : "Retention saved";
    });
  const saveBundle = () =>
    run("save", async () => {
      const b = await tauriInvoke<SavedBundle>("diagnostics_create", { outPath: folder.trim() });
      return `Saved ${b.path} (${Math.ceil(b.size / 1024)} KB, ${b.files.length} files${b.truncated ? "; older logs left out" : ""})`;
    });
  const sendBundle = () =>
    run("send", async () => {
      const id = await tauriInvoke<string>("diagnostics_send");
      return `Diagnostics sent to the console (${id})`;
    });

  const setNum = (key: keyof LogRetention) => (e: React.ChangeEvent<HTMLInputElement>) => setRetention({ ...retention, [key]: Math.max(1, Math.floor(Number(e.target.value) || 1)) });

  return (
    <div className="space-y-3 pt-3 border-t border-gray-200 dark:border-gray-700 mt-3" data-testid="diagnostics-section">
      <div>
        <label className="block text-xs font-medium mb-1">Keep log files</label>
        <div className="flex items-center gap-2 flex-wrap text-sm">
          <input aria-label="Days to keep logs" className={inputCls} type="number" min={1} max={3650} value={retention.max_age_days} onChange={setNum("max_age_days")} />
          <span>days, at most</span>
          <input aria-label="Maximum log size in MB" className={inputCls} type="number" min={1} max={100000} value={retention.max_total_mb} onChange={setNum("max_total_mb")} />
          <span>MB</span>
          <button type="button" className={btnCls} disabled={busy !== null} onClick={saveRetention}>
            {busy === "retention" ? "Saving…" : "Save"}
          </button>
        </div>
      </div>

      <div>
        <label className="block text-xs font-medium mb-1">Diagnostics bundle</label>
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">
          Recent logs, app version and OS details. Passwords, tokens, e-mail addresses and folder names are removed first.
        </p>
        <div className="flex items-center gap-2 flex-wrap">
          <input aria-label="Folder to save diagnostics" className="flex-1 min-w-48 text-sm px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-transparent font-mono" placeholder="/Users/me/Desktop" value={folder} onChange={(e) => setFolder(e.target.value)} />
          <button type="button" className={btnCls} disabled={busy !== null || !folder.trim()} onClick={saveBundle}>
            {busy === "save" ? "Creating…" : "Save diagnostics"}
          </button>
          {registered && (
            <button type="button" className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50" disabled={busy !== null} onClick={sendBundle}>
              {busy === "send" ? "Sending…" : "Send to console"}
            </button>
          )}
        </div>
      </div>

      {error && <p className="text-xs text-red-500">{error}</p>}
      {message && <p className="text-xs text-green-600 dark:text-green-400">{message}</p>}
    </div>
  );
}
