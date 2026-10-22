/**
 * "Managed by console" (Settings): connect this machine to a UFOP
 * management console with a registration link token or a claim code,
 * and see heartbeats, managed policies and queued reports.
 */
import { useEffect, useRef, useState } from "react";
import { useFleetAgentStore } from "@/stores/fleet-agent-store";

const input =
  "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";
const button = "text-sm px-3 py-1.5 rounded disabled:opacity-50";

function when(ts: string | null): string {
  if (!ts) return "never";
  const d = new Date(ts);
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
}

export function FleetAgentSection() {
  const { status, busy, error, refresh, register, startClaim, pollClaim, cancelClaim, unregister, setEnabled, syncNow } =
    useFleetAgentStore();
  const [mode, setMode] = useState<"link" | "claim">("link");
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [tags, setTags] = useState("");
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Poll while a claim code is shown.
  const claiming = !status.registered && status.claim !== null;
  useEffect(() => {
    if (!claiming) return;
    timer.current = setInterval(() => {
      void pollClaim();
    }, 5000);
    return () => {
      if (timer.current) clearInterval(timer.current);
    };
  }, [claiming, pollClaim]);

  if (status.registered) {
    const state = status.machine_status || "active";
    return (
      <div className="pt-3 space-y-3 text-sm" data-testid="fleet-agent-registered">
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-gray-500">Console</dt>
          <dd className="break-all">{status.console_url}</dd>
          <dt className="text-gray-500">Machine</dt>
          <dd>
            {status.machine_id}
            {status.organization ? ` · ${status.organization}` : ""}
          </dd>
          <dt className="text-gray-500">State</dt>
          <dd className={state === "active" ? "" : "text-amber-600"}>
            {state}
            {status.enabled ? "" : " (agent turned off)"}
          </dd>
          <dt className="text-gray-500">Last heartbeat</dt>
          <dd>
            {when(status.last_heartbeat_at)} · every {status.heartbeat_seconds}s
          </dd>
          <dt className="text-gray-500">Queued reports</dt>
          <dd>
            {status.pending_reports}
            {status.dropped_reports > 0 ? ` (${status.dropped_reports} dropped while offline)` : ""}
          </dd>
        </dl>
        {status.last_error && <p className="text-xs text-amber-600">{status.last_error}</p>}
        {status.managed_policies.length > 0 && (
          <div>
            <p className="text-xs font-medium mb-1">Managed backup jobs</p>
            <ul className="text-xs space-y-0.5">
              {status.managed_policies.map((p) => (
                <li key={p.policy_id}>
                  {p.job_id} <span className="text-gray-500">v{p.version}</span>
                  {p.error && <span className="text-red-600"> — {p.error}</span>}
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="flex flex-wrap gap-2">
          <button type="button" disabled={busy || !status.enabled} onClick={() => void syncNow()} className={`${button} bg-blue-500 text-white hover:bg-blue-600`}>
            Sync now
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void setEnabled(!status.enabled)}
            className={`${button} border border-gray-300 dark:border-gray-600`}
          >
            {status.enabled ? "Turn off" : "Turn on"}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              if (window.confirm("Leave the management console? Managed backup jobs are removed (their backups stay).")) {
                void unregister(state === "revoked");
              }
            }}
            className={`${button} border border-red-300 text-red-600`}
          >
            Unregister
          </button>
        </div>
        {error && <p className="text-xs text-red-600">{error}</p>}
      </div>
    );
  }

  return (
    <div className="pt-3 space-y-3 text-sm" data-testid="fleet-agent-unregistered">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Let an administrator manage this machine's backups centrally: policies, remote runs and reports.
      </p>
      {status.claim ? (
        <div className="space-y-2">
          <p className="text-xs">Enter this code in the console under Machines → Claim:</p>
          <p className="font-mono text-2xl tracking-widest" data-testid="claim-code">
            {status.claim.code}
          </p>
          <p className="text-xs text-gray-500">Waiting for {status.claim.console_url} · valid until {when(status.claim.expires_at)}</p>
          <button type="button" onClick={() => void cancelClaim()} className={`${button} border border-gray-300 dark:border-gray-600`}>
            Cancel
          </button>
        </div>
      ) : (
        <form
          className="space-y-2"
          onSubmit={(e) => {
            e.preventDefault();
            void (mode === "link" ? register(url, token, tags) : startClaim(url));
          }}
        >
          <div className="flex gap-3 text-xs">
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "link"} onChange={() => setMode("link")} /> Registration link
            </label>
            <label className="flex items-center gap-1">
              <input type="radio" checked={mode === "claim"} onChange={() => setMode("claim")} /> Claim code
            </label>
          </div>
          <input aria-label="Console URL" className={input} placeholder="https://console.example.com" value={url} onChange={(e) => setUrl(e.target.value)} />
          {mode === "link" && (
            <>
              <input aria-label="Registration token" className={input} type="password" placeholder="ufop_reg_…" value={token} onChange={(e) => setToken(e.target.value)} />
              <input aria-label="Tags" className={input} placeholder="Tags (optional, comma separated)" value={tags} onChange={(e) => setTags(e.target.value)} />
            </>
          )}
          <button type="submit" disabled={busy || !url.trim() || (mode === "link" && !token.trim())} className={`${button} bg-blue-500 text-white hover:bg-blue-600`}>
            {mode === "link" ? "Register" : "Get claim code"}
          </button>
        </form>
      )}
      {error && <p className="text-xs text-red-600">{error}</p>}
    </div>
  );
}
