/**
 * Settings → Managed by Console: what this machine lets the console do.
 * Set only here; the console can neither see nor change these limits.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { describeAgentError } from "@/stores/fleet-agent-store";

/** Mirrors `ufop_core::agent::AgentLimits`. */
export interface AgentLimits {
  roots: string[];
  read_only: boolean;
  symlinks: "stay_inside" | "follow";
  allow_scripts: boolean;
}

const DEFAULT_LIMITS: AgentLimits = { roots: [], read_only: false, symlinks: "stay_inside", allow_scripts: true };
const input = "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";

export function FleetAgentLimits() {
  const [limits, setLimits] = useState<AgentLimits>(DEFAULT_LIMITS);
  const [roots, setRoots] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    void tauriInvokeSafe<AgentLimits>("fleet_agent_get_limits", undefined, DEFAULT_LIMITS).then((l) => {
      setLimits(l);
      setRoots(l.roots.join("\n"));
    });
  }, []);

  const save = async () => {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const next = { ...limits, roots: roots.split("\n").map((r) => r.trim()).filter(Boolean) };
      const saved = await tauriInvoke<AgentLimits>("fleet_agent_set_limits", { limits: next });
      setLimits(saved);
      setRoots(saved.roots.join("\n"));
      setMessage("Saved");
    } catch (err) {
      setError(describeAgentError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <details className="mt-3 text-sm" data-testid="fleet-agent-limits">
      <summary className="cursor-pointer text-xs font-medium">What the console may do on this machine</summary>
      <div className="space-y-2 pt-2">
        <label className="block text-xs">
          Allowed folders (one per line; empty = any). Sources, local destinations and restore targets from the console must be inside.
          <textarea aria-label="Allowed folders" className={`${input} font-mono min-h-16 mt-1`} value={roots} onChange={(e) => setRoots(e.target.value)} placeholder="/Users/me/Documents" />
        </label>
        <label className="flex items-center gap-2 text-xs">
          Symbolic links
          <select aria-label="Symbolic links" className={`${input} w-auto`} value={limits.symlinks} onChange={(e) => setLimits({ ...limits, symlinks: e.target.value as AgentLimits["symlinks"] })}>
            <option value="stay_inside">Must point inside the allowed folders</option>
            <option value="follow">Judge the path as written</option>
          </select>
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={limits.read_only} onChange={(e) => setLimits({ ...limits, read_only: e.target.checked })} />
          Read-only: the console can view and run backups, but not change jobs, restore files or delete backup data
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={limits.allow_scripts} onChange={(e) => setLimits({ ...limits, allow_scripts: e.target.checked })} />
          Allow jobs from the console to run pre/post scripts on this machine
        </label>
        <button type="button" disabled={busy} onClick={() => void save()} className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50">
          {busy ? "Saving…" : "Save limits"}
        </button>
        {error && <p className="text-xs text-red-600">{error}</p>}
        {message && <p className="text-xs text-green-600 dark:text-green-400">{message}</p>}
      </div>
    </details>
  );
}
