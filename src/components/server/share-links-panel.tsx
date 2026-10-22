/**
 * Share links of ufop-server (`/api/v1/shares`): create links to files and
 * folders under the server's --share-root (download, upload drop box, or
 * both; password, expiry, limits), copy a new link once, see each link's
 * access log, and turn links off.
 */
import { useCallback, useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { serverApi } from "@/lib/server-client";

export type ShareKind = "download" | "upload" | "both";

export interface ServerShare {
  id: string;
  name: string;
  path: string;
  kind: ShareKind;
  token_prefix: string;
  has_password: boolean;
  expires_at: string | null;
  max_downloads: number | null;
  downloads: number;
  max_upload_bytes: number | null;
  created_at: string;
  revoked_at: string | null;
  last_access_at: string | null;
}

interface ShareEvent {
  at: string;
  ip: string;
  action: string;
  detail: string;
}

const KIND_LABEL: Record<ShareKind, string> = {
  download: "Download",
  upload: "Upload (drop box)",
  both: "Download and upload",
};
const input =
  "w-full px-2 py-1 text-sm rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800";
const when = (iso: string | null) =>
  iso ? new Date(iso).toLocaleString() : "—";

function status(s: ServerShare): string {
  if (s.revoked_at) return "Off";
  if (s.expires_at && new Date(s.expires_at) < new Date()) return "Expired";
  if (
    s.max_downloads !== null &&
    s.downloads >= s.max_downloads &&
    s.kind === "download"
  )
    return "Used up";
  return "Active";
}

export function ShareLinksPanel({
  pickFolder,
}: {
  pickFolder?: (title: string) => Promise<string | null>;
}) {
  const [shares, setShares] = useState<ServerShare[]>([]);
  const [roots, setRoots] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [form, setForm] = useState({
    name: "",
    path: "",
    kind: "download" as ShareKind,
    password: "",
    days: "",
    maxDownloads: "",
    maxUploadMb: "",
  });
  const [created, setCreated] = useState<string | null>(null);
  const [log, setLog] = useState<{ id: string; events: ShareEvent[] } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const r = await serverApi<{ shares: ServerShare[]; roots: string[] }>(
        "GET",
        "/shares",
      );
      setShares(r.shares);
      setRoots(r.roots);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const num = (v: string) => (v.trim() === "" ? null : Number(v));
  const create = async () => {
    setBusy(true);
    setError(null);
    setCreated(null);
    try {
      const r = await serverApi<{ url: string }>("POST", "/shares", {
        name:
          form.name ||
          form.path.split(/[\\/]/).filter(Boolean).pop() ||
          "Shared",
        path: form.path,
        kind: form.kind,
        password: form.password || null,
        expires_in_days: num(form.days),
        max_downloads: form.kind === "upload" ? null : num(form.maxDownloads),
        max_upload_mb: form.kind === "download" ? null : num(form.maxUploadMb),
      });
      setCreated(`${window.location.origin}${r.url}`);
      setForm({
        name: "",
        path: "",
        kind: "download",
        password: "",
        days: "",
        maxDownloads: "",
        maxUploadMb: "",
      });
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const revoke = async (id: string) => {
    try {
      await serverApi("DELETE", `/shares/${encodeURIComponent(id)}`);
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const showLog = async (id: string) => {
    try {
      const r = await serverApi<{ events: ShareEvent[] }>(
        "GET",
        `/shares/${encodeURIComponent(id)}/events?limit=100`,
      );
      setLog({ id, events: r.events });
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (roots !== null && roots.length === 0) {
    return (
      <div
        className="p-6 text-sm text-zinc-600 dark:text-zinc-400"
        data-testid="share-links-off"
      >
        Sharing is off on this server. Start <code>ufop-server</code> with{" "}
        <code>--share-root &lt;folder&gt;</code> to share files and folders
        under that folder by link.
      </div>
    );
  }

  return (
    <div
      className="p-4 space-y-4 text-sm text-zinc-900 dark:text-zinc-100 overflow-y-auto h-full"
      data-testid="share-links-panel"
    >
      <section className="space-y-2 rounded-lg border border-zinc-200 dark:border-zinc-700 p-3">
        <h2 className="font-semibold">New link</h2>
        {roots && (
          <p className="text-xs text-zinc-500">
            Files and folders under {roots.join(", ")}
          </p>
        )}
        <div className="grid gap-2 sm:grid-cols-2">
          <label className="text-xs">
            File or folder
            <div className="flex gap-1">
              <input
                aria-label="Path"
                className={input}
                value={form.path}
                onChange={(e) => setForm({ ...form, path: e.target.value })}
                placeholder={roots?.[0] ?? ""}
              />
              {pickFolder && (
                <button
                  type="button"
                  className="px-2 rounded bg-zinc-100 dark:bg-zinc-800 text-xs"
                  onClick={async () => {
                    const p = await pickFolder("Folder to share");
                    if (p) setForm((f) => ({ ...f, path: p }));
                  }}
                >
                  Browse…
                </button>
              )}
            </div>
          </label>
          <label className="text-xs">
            Name
            <input
              aria-label="Link name"
              className={input}
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Shown on the page"
            />
          </label>
          <label className="text-xs">
            Visitors can
            <select
              aria-label="Link kind"
              className={input}
              value={form.kind}
              onChange={(e) =>
                setForm({ ...form, kind: e.target.value as ShareKind })
              }
            >
              {(Object.keys(KIND_LABEL) as ShareKind[]).map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            Password (optional, 8+ characters)
            <input
              aria-label="Link password"
              type="password"
              autoComplete="new-password"
              className={input}
              value={form.password}
              onChange={(e) => setForm({ ...form, password: e.target.value })}
            />
          </label>
          <label className="text-xs">
            Expires after (days)
            <input
              aria-label="Expires after days"
              type="number"
              min={1}
              className={input}
              value={form.days}
              onChange={(e) => setForm({ ...form, days: e.target.value })}
              placeholder="Never"
            />
          </label>
          {form.kind !== "upload" && (
            <label className="text-xs">
              Download limit
              <input
                aria-label="Download limit"
                type="number"
                min={1}
                className={input}
                value={form.maxDownloads}
                onChange={(e) =>
                  setForm({ ...form, maxDownloads: e.target.value })
                }
                placeholder="Unlimited"
              />
            </label>
          )}
          {form.kind !== "download" && (
            <label className="text-xs">
              Largest upload (MB)
              <input
                aria-label="Largest upload"
                type="number"
                min={1}
                className={input}
                value={form.maxUploadMb}
                onChange={(e) =>
                  setForm({ ...form, maxUploadMb: e.target.value })
                }
                placeholder="1024"
              />
            </label>
          )}
        </div>
        <button
          type="button"
          className="px-3 py-1 rounded bg-blue-500 text-white disabled:opacity-50"
          disabled={busy || !form.path.trim()}
          onClick={() => void create()}
        >
          Create link
        </button>
        {created && (
          <div
            className="rounded bg-green-50 dark:bg-green-950 p-2 text-xs space-y-1"
            role="status"
          >
            <p>Link created. Copy it now — it is not shown again.</p>
            <div className="flex gap-2">
              <input
                aria-label="New link"
                readOnly
                className={`${input} font-mono`}
                value={created}
                onFocus={(e) => e.target.select()}
              />
              <button
                type="button"
                className="px-2 rounded bg-zinc-100 dark:bg-zinc-800"
                onClick={() => void navigator.clipboard?.writeText(created)}
              >
                Copy
              </button>
            </div>
          </div>
        )}
      </section>
      {error && (
        <p className="text-red-600 text-xs" role="alert">
          {error}
        </p>
      )}
      <section>
        <h2 className="font-semibold mb-2">Links</h2>
        {shares.length === 0 ? (
          <p className="text-xs text-zinc-500">No links yet.</p>
        ) : (
          <table className="w-full text-xs" aria-label="Share links">
            <thead className="text-left text-zinc-500">
              <tr>
                <th className="py-1">Name</th>
                <th>Path</th>
                <th>Kind</th>
                <th>Status</th>
                <th>Downloads</th>
                <th>Expires</th>
                <th>Last used</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shares.map((s) => (
                <tr
                  key={s.id}
                  className="border-t border-zinc-100 dark:border-zinc-800"
                >
                  <td className="py-1">
                    {s.name}{" "}
                    {s.has_password && (
                      <span title="Password protected">🔒</span>
                    )}{" "}
                    <span className="font-mono text-zinc-400">
                      {s.token_prefix}…
                    </span>
                  </td>
                  <td
                    className="font-mono truncate max-w-[16rem]"
                    title={s.path}
                  >
                    {s.path}
                  </td>
                  <td>{KIND_LABEL[s.kind]}</td>
                  <td>{status(s)}</td>
                  <td>
                    {s.downloads}
                    {s.max_downloads !== null && ` / ${s.max_downloads}`}
                  </td>
                  <td>{when(s.expires_at)}</td>
                  <td>{when(s.last_access_at)}</td>
                  <td className="whitespace-nowrap space-x-2">
                    <button
                      type="button"
                      className="underline"
                      onClick={() => void showLog(s.id)}
                    >
                      Activity
                    </button>
                    {!s.revoked_at && (
                      <button
                        type="button"
                        className="underline text-red-600"
                        onClick={() => void revoke(s.id)}
                        aria-label={`Turn off ${s.name}`}
                      >
                        Turn off
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      {log && (
        <section aria-label="Link activity">
          <h2 className="font-semibold mb-1">
            Activity — {shares.find((s) => s.id === log.id)?.name}
          </h2>
          {log.events.length === 0 ? (
            <p className="text-xs text-zinc-500">Nothing yet.</p>
          ) : (
            <ul className="text-xs space-y-0.5">
              {log.events.map((e, i) => (
                <li key={i}>
                  <span className="text-zinc-500">{when(e.at)}</span> ·{" "}
                  {e.action} {e.detail && `· ${e.detail}`} ·{" "}
                  <span className="font-mono">{e.ip}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}
