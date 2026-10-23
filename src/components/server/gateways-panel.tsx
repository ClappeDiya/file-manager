/**
 * File gateways connected to ufop-server (`/api/v1/gateways`): on-premises
 * agents (`ufop gateway serve`) and their nodes, with a browser for the
 * folders they serve — download, and upload through writable gateways.
 * Data streams through the server and is not stored there.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { serverApi, serverRaw } from "@/lib/server-client";

export interface GatewayNode {
  node: string;
  host: string;
  ip: string;
  roots: string[];
  writable: boolean;
  version: string;
  last_seen: string;
  online: boolean;
  jobs: number;
}

export interface GatewayInfo {
  name: string;
  online: boolean;
  online_nodes: number;
  writable: boolean;
  queued: number;
  nodes: GatewayNode[];
}

interface GatewayEntry {
  name: string;
  is_dir: boolean;
  size: number;
  modified: string | null;
}

const btn =
  "px-2 py-0.5 rounded border border-zinc-300 dark:border-zinc-600 text-xs disabled:opacity-50";
const enc = encodeURIComponent;
const join = (dir: string, name: string) =>
  `${dir.replace(/\/+$/, "")}/${name}`;
const parent = (p: string) => p.replace(/\/[^/]+\/?$/, "") || "/";

export function GatewaysPanel() {
  const [gateways, setGateways] = useState<GatewayInfo[] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [path, setPath] = useState("/");
  const [entries, setEntries] = useState<GatewayEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const load = useCallback(
    () =>
      run(async () => {
        const v = await serverApi<{ gateways: GatewayInfo[] }>(
          "GET",
          "/gateways",
        );
        setGateways(v.gateways);
      }),
    [],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const browse = (gw: string, p: string) =>
    run(async () => {
      const v = await serverApi<{ entries: GatewayEntry[] }>(
        "GET",
        `/gateways/${enc(gw)}/files?path=${enc(p)}`,
      );
      setOpen(gw);
      setPath(p);
      setEntries(v.entries);
    });

  const download = (gw: string, p: string, name: string) =>
    run(async () => {
      const r = await serverRaw(
        "GET",
        `/gateways/${enc(gw)}/download?path=${enc(p)}`,
      );
      const url = URL.createObjectURL(await r.blob());
      const a = document.createElement("a");
      a.href = url;
      a.download = name;
      a.click();
      URL.revokeObjectURL(url);
    });

  const upload = (gw: string, files: FileList | null) =>
    run(async () => {
      for (const f of Array.from(files ?? [])) {
        await serverRaw(
          "PUT",
          `/gateways/${enc(gw)}/upload?path=${enc(join(path, f.name))}`,
          f,
          "application/octet-stream",
        );
      }
      setNote(`Uploaded ${files?.length ?? 0} file(s).`);
      const v = await serverApi<{ entries: GatewayEntry[] }>(
        "GET",
        `/gateways/${enc(gw)}/files?path=${enc(path)}`,
      );
      setEntries(v.entries);
    });

  const current = gateways?.find((g) => g.name === open);
  return (
    <div className="p-3 space-y-3 text-sm" data-testid="gateways-panel">
      <div className="flex items-center gap-2">
        <h2 className="font-semibold">File gateways</h2>
        <button
          type="button"
          className={btn}
          disabled={busy}
          onClick={() => void load()}
        >
          Refresh
        </button>
      </div>
      <p className="text-xs text-zinc-500">
        Folders on other machines, served by{" "}
        <code>ufop gateway serve --server … --name … --root …</code>. The agent
        connects out to this server (no inbound port), and agents with the same
        name share the work.
      </p>
      {gateways && gateways.length === 0 && (
        <p className="text-xs text-zinc-500">No gateway has connected yet.</p>
      )}
      {gateways?.map((g) => (
        <section
          key={g.name}
          className="rounded-lg border border-zinc-200 dark:border-zinc-700 p-2 space-y-1"
          data-testid="gateway"
        >
          <div className="flex items-center gap-2">
            <span
              className={g.online ? "text-green-600" : "text-zinc-400"}
              aria-label={g.online ? "online" : "offline"}
            >
              ●
            </span>
            <b>{g.name}</b>
            <span className="text-xs text-zinc-500">
              {g.online_nodes}/{g.nodes.length} node(s) online
              {g.writable ? " · uploads allowed" : " · read-only"}
            </span>
            <button
              type="button"
              className={`${btn} ml-auto`}
              disabled={busy || !g.online}
              onClick={() => void browse(g.name, "/")}
            >
              Browse
            </button>
          </div>
          <ul className="text-xs text-zinc-500">
            {g.nodes.map((n) => (
              <li key={n.node}>
                {n.host || n.node} ({n.ip}) · {n.roots.join(", ")} ·{" "}
                {n.online ? "online" : "offline"} · {n.jobs} job(s)
              </li>
            ))}
          </ul>
        </section>
      ))}
      {open && entries && (
        <section
          className="rounded-lg border border-zinc-200 dark:border-zinc-700 p-2 space-y-1"
          data-testid="gateway-browser"
        >
          <div className="flex items-center gap-2 text-xs">
            <b>{open}</b>
            <span className="font-mono">{path}</span>
            {path !== "/" && (
              <button
                type="button"
                className={btn}
                onClick={() => void browse(open, parent(path))}
              >
                Up
              </button>
            )}
            {current?.writable && path !== "/" && (
              <>
                <button
                  type="button"
                  className={`${btn} ml-auto`}
                  disabled={busy}
                  onClick={() => fileInput.current?.click()}
                >
                  Upload…
                </button>
                <input
                  ref={fileInput}
                  type="file"
                  multiple
                  hidden
                  aria-label="Upload files"
                  onChange={(e) => void upload(open, e.target.files)}
                />
              </>
            )}
          </div>
          <ul className="text-xs">
            {entries.map((e) => (
              <li key={e.name} className="flex gap-2">
                {e.is_dir ? (
                  <button
                    type="button"
                    className="text-blue-600 hover:underline"
                    onClick={() => void browse(open, join(path, e.name))}
                  >
                    {e.name}/
                  </button>
                ) : (
                  <button
                    type="button"
                    className="hover:underline"
                    title="Download"
                    onClick={() =>
                      void download(open, join(path, e.name), e.name)
                    }
                  >
                    {e.name}
                  </button>
                )}
                {!e.is_dir && (
                  <span className="ml-auto text-zinc-500">
                    {formatBytes(e.size)}
                  </span>
                )}
              </li>
            ))}
            {entries.length === 0 && (
              <li className="text-zinc-500">Empty folder</li>
            )}
          </ul>
        </section>
      )}
      {note && (
        <p className="text-xs text-green-600" role="status">
          {note}
        </p>
      )}
      {error && (
        <p className="text-xs text-red-600" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
