/**
 * Trusted SSH hosts (Settings): host keys the app accepted on first use.
 *
 * Keys are persisted on the Rust side (connectors/known_hosts.rs), so a
 * server whose key changes after a restart is refused as a possible MITM.
 * "Forget" is the way out after a legitimate change (server rebuilt or
 * re-keyed): the next connection accepts and stores the new key. Keys in
 * ~/.ssh/known_hosts are honoured but managed with ssh-keygen, not here.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";

export interface KnownHostEntry {
  hostname: string;
  port: number;
  key_type: string;
  fingerprint: string;
  added_at: string;
}

export function TrustedSshHostsSection() {
  const [hosts, setHosts] = useState<KnownHostEntry[]>([]);
  const [error, setError] = useState("");
  const [confirming, setConfirming] = useState<string | null>(null);

  const load = useCallback(async () => {
    setHosts(await tauriInvokeSafe<KnownHostEntry[]>("list_known_hosts", undefined, []));
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const forget = async (host: string, port: number) => {
    setError("");
    try {
      await tauriInvoke<number>("forget_known_host", { host, port });
      setConfirming(null);
      await load();
    } catch (err) {
      setError(String(err));
    }
  };

  // One row per host:port; a host can have several key types.
  const grouped = new Map<string, KnownHostEntry[]>();
  for (const h of hosts) {
    const key = `${h.hostname}:${h.port}`;
    grouped.set(key, [...(grouped.get(key) ?? []), h]);
  }

  return (
    <div className="space-y-3 pt-3">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Server keys accepted on first connection. If a server's key changes, connecting is
        refused until you confirm the change here.
      </p>
      {grouped.size === 0 ? (
        <p className="text-xs text-gray-400">No hosts trusted yet. Keys are added the first time you connect.</p>
      ) : (
        <ul className="space-y-1.5" aria-label="Trusted SSH hosts">
          {[...grouped.entries()].map(([key, entries]) => {
            const { hostname, port } = entries[0];
            return (
              <li key={key} className="text-xs p-2 rounded border border-gray-200 dark:border-gray-700">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{port === 22 ? hostname : `${hostname}:${port}`}</span>
                  {confirming === key ? (
                    <span className="flex gap-2">
                      <button
                        type="button"
                        onClick={() => forget(hostname, port)}
                        className="text-red-600 hover:text-red-700 font-medium"
                      >
                        Forget keys
                      </button>
                      <button type="button" onClick={() => setConfirming(null)} className="text-gray-500">
                        Cancel
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => setConfirming(key)}
                      className="text-red-500 hover:text-red-700"
                    >
                      Forget
                    </button>
                  )}
                </div>
                {entries.map((e) => (
                  <div key={e.key_type} className="font-mono text-gray-500 break-all mt-1">
                    {e.key_type} {e.fingerprint}
                  </div>
                ))}
                {confirming === key && (
                  <p className="text-amber-600 mt-1">
                    Only do this if you know the server was rebuilt or re-keyed. The next connection
                    will trust whatever key the server presents.
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {error && <p className="text-xs text-red-500">{error}</p>}
    </div>
  );
}
