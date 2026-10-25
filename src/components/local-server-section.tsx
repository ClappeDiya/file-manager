/**
 * Local web server and saved servers (Settings → Advanced).
 *
 * The first part starts and stops the app's own web server (the same code as
 * `ufop-server`): loopback only unless "other computers" is ticked, which also
 * turns on HTTPS, and never without a password. The second part is the list of
 * remote servers shared with the `ufop` command line (`ufop server ...`),
 * with a test that asks each server's health address; no tokens are shown.
 * Mirrors Rust `embedded_server::{EmbeddedSettings, EmbeddedStatus}` and
 * `commands::embedded_server_commands::ProbeResult`.
 */
import { useCallback, useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

export interface EmbeddedSettings {
  port: number;
  allow_network: boolean;
  https: boolean;
  web_root: string | null;
}

export interface EmbeddedStatus {
  running: boolean;
  url: string | null;
  settings: EmbeddedSettings;
  has_password: boolean;
  web_interface: boolean;
  started_at: string | null;
  error: string | null;
}

export interface ServerProfile {
  name: string;
  server_url: string | null;
  signed_in: boolean;
  current: boolean;
}

export interface ProbeResult {
  ok: boolean;
  message: string;
  version: string | null;
  uptime_secs: number | null;
  latency_ms: number;
  untrusted_certificate: boolean;
}

export const MIN_PASSWORD_CHARS = 8;

const field = "rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 px-2 py-1 text-sm";
const btn = "text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50";

function describeProbe(r: ProbeResult): string {
  if (!r.ok) return r.message;
  const bits = [r.version ? `ufop-server ${r.version}` : "ufop-server", `${r.latency_ms} ms`];
  if (r.uptime_secs != null) bits.push(`up ${Math.max(1, Math.round(r.uptime_secs / 60))} min`);
  return `Reachable: ${bits.join(", ")}`;
}

function LocalServer() {
  const [status, setStatus] = useState<EmbeddedStatus | null>(null);
  const [settings, setSettings] = useState<EmbeddedSettings | null>(null);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const show = useCallback((s: EmbeddedStatus) => {
    setStatus(s);
    setSettings(s.settings);
  }, []);

  useEffect(() => {
    tauriInvoke<EmbeddedStatus>("embedded_server_status")
      .then(show)
      .catch((e) => setError(errorText(e)));
  }, [show]);

  // It can stop by itself (for example when its port is taken away).
  useEffect(() => {
    if (!status?.running) return;
    const t = setInterval(() => {
      tauriInvoke<EmbeddedStatus>("embedded_server_status")
        .then((s) => setStatus((old) => (old ? { ...old, running: s.running, url: s.url, error: s.error } : s)))
        .catch(() => {});
    }, 5000);
    return () => clearInterval(t);
  }, [status?.running]);

  if (!status || !settings) return error ? <p className="pt-3 text-xs text-red-500">{error}</p> : null;

  const act = async (fn: () => Promise<EmbeddedStatus | void>, done?: string) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const s = await fn();
      if (s) show(s);
      if (done) setNotice(done);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const running = status.running;
  const changed = JSON.stringify(settings) !== JSON.stringify(status.settings);
  const passwordProblem =
    password && password.length < MIN_PASSWORD_CHARS
      ? `Use at least ${MIN_PASSWORD_CHARS} characters`
      : confirm && confirm !== password
        ? "The two passwords are different"
        : null;
  const patch = (p: Partial<EmbeddedSettings>) => setSettings({ ...settings, ...p });

  const savePassword = () =>
    act(async () => {
      const s = await tauriInvoke<EmbeddedStatus>("embedded_server_set_password", { password });
      setPassword("");
      setConfirm("");
      return s;
    }, "Password saved");
  const start = () =>
    act(async () => {
      if (changed) await tauriInvoke<EmbeddedStatus>("embedded_server_save_settings", { settings });
      return tauriInvoke<EmbeddedStatus>("embedded_server_start");
    });

  return (
    <div className="space-y-3 pt-3" data-testid="local-server">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Run the web interface and API of <code>ufop-server</code> inside this app, sharing its backup jobs and connections. It is off until you start it, it listens on this computer only unless you
        allow other computers, and it never starts without a password.
      </p>

      <p className="text-sm" data-testid="local-server-status" role="status">
        {running ? (
          <>
            <span className="text-green-600 dark:text-green-400 font-medium">Running</span> at <code>{status.url}</code>
            {!status.web_interface && <span className="block text-xs text-amber-600">The web interface files were not found, so only the API answers. Choose its folder below.</span>}
          </>
        ) : (
          <span className="text-gray-600 dark:text-gray-300">Stopped</span>
        )}
      </p>
      {status.error && !running && <p className="text-xs text-red-600" data-testid="local-server-error">{status.error}</p>}

      <div className="flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-2 text-sm" htmlFor="local-server-port">
          Port
          <input
            id="local-server-port"
            type="number"
            min={1024}
            max={65535}
            className={`${field} w-24`}
            value={settings.port}
            disabled={running}
            onChange={(e) => patch({ port: Number(e.target.value) })}
          />
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={settings.allow_network} disabled={running} onChange={(e) => patch({ allow_network: e.target.checked })} />
          Allow other computers (uses HTTPS)
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={settings.allow_network || settings.https} disabled={running || settings.allow_network} onChange={(e) => patch({ https: e.target.checked })} />
          HTTPS
        </label>
      </div>
      <div className="flex items-center gap-2">
        <label className="text-sm" htmlFor="local-server-webroot">Web interface folder</label>
        <input
          id="local-server-webroot"
          className={`${field} flex-1`}
          placeholder="Optional; found next to the app when empty"
          value={settings.web_root ?? ""}
          disabled={running}
          onChange={(e) => patch({ web_root: e.target.value || null })}
        />
      </div>

      <fieldset className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-3">
        <legend className="px-1 text-xs font-semibold">Password {status.has_password ? "(set)" : "(required)"}</legend>
        <div className="flex flex-wrap gap-2">
          <input type="password" autoComplete="new-password" aria-label="New password" className={field} placeholder="New password" value={password} onChange={(e) => setPassword(e.target.value)} />
          <input type="password" autoComplete="new-password" aria-label="Repeat password" className={field} placeholder="Repeat password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
          <button type="button" className={btn} disabled={busy || !password || !!passwordProblem || confirm !== password} onClick={savePassword}>
            {status.has_password ? "Change password" : "Set password"}
          </button>
        </div>
        {passwordProblem && <p className="text-xs text-red-600">{passwordProblem}</p>}
        <p className="text-xs text-gray-500">This is the password you sign in with in the browser. It is stored hashed and cannot be shown again.</p>
      </fieldset>

      <div className="flex flex-wrap gap-2">
        {running ? (
          <>
            <button type="button" className={btn} disabled={busy} onClick={() => act(() => tauriInvoke<EmbeddedStatus>("embedded_server_stop"))}>Stop server</button>
            <button type="button" className={btn} disabled={busy || !status.url} onClick={() => act(async () => { await tauriInvoke("open_server_in_browser", { url: status.url }); })}>Open in browser</button>
          </>
        ) : (
          <button type="button" className={btn} disabled={busy || !status.has_password} title={status.has_password ? undefined : "Set a password first"} onClick={start}>Start server</button>
        )}
      </div>
      {!running && !status.has_password && <p className="text-xs text-gray-500">Set a password to be able to start the server.</p>}
      {error && <p className="text-xs text-red-600" role="alert">{error}</p>}
      {notice && <p className="text-xs text-green-600 dark:text-green-400">{notice}</p>}
    </div>
  );
}

function SavedServers() {
  const [profiles, setProfiles] = useState<ServerProfile[]>([]);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ProbeResult>>({});
  const [testing, setTesting] = useState<string | null>(null);
  const [trust, setTrust] = useState<Record<string, boolean>>({});

  useEffect(() => {
    tauriInvoke<ServerProfile[]>("server_profiles_list")
      .then(setProfiles)
      .catch((e) => setError(errorText(e)));
  }, []);

  const change = async (cmd: string, args: Record<string, unknown>) => {
    setError(null);
    try {
      setProfiles(await tauriInvoke<ServerProfile[]>(cmd, args));
      return true;
    } catch (e) {
      setError(errorText(e));
      return false;
    }
  };
  const add = async () => {
    if (await change("server_profile_save", { name, serverUrl: url })) {
      setName("");
      setUrl("");
    }
  };
  const test = async (key: string, address: string) => {
    setTesting(key);
    try {
      const r = await tauriInvoke<ProbeResult>("server_profile_test", { serverUrl: address, acceptUntrustedCertificate: trust[key] === true });
      setResults((old) => ({ ...old, [key]: r }));
    } catch (e) {
      setResults((old) => ({ ...old, [key]: { ok: false, message: errorText(e), version: null, uptime_secs: null, latency_ms: 0, untrusted_certificate: false } }));
    } finally {
      setTesting(null);
    }
  };

  return (
    <div className="space-y-2 pt-3" data-testid="saved-servers">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Remote <code>ufop-server</code> addresses, the same list the <code>ufop</code> command line uses. Sign in to one from a terminal with <code>ufop server login</code>; no tokens are shown or changed here.
      </p>
      {profiles.length === 0 && <p className="text-xs text-gray-500">No saved servers.</p>}
      <ul className="space-y-2">
        {profiles.map((p) => {
          const r = results[p.name];
          return (
            <li key={p.name} className="rounded border border-gray-200 dark:border-gray-700 p-2 text-sm" data-testid={`server-${p.name}`}>
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{p.name}</span>
                {p.current && <span className="text-xs text-gray-500">(default)</span>}
                <code className="text-xs">{p.server_url ?? "no address"}</code>
                {p.signed_in && <span className="text-xs text-gray-500">signed in from the command line</span>}
                <span className="flex-1" />
                <button type="button" className={btn} disabled={!p.server_url || testing === p.name} onClick={() => test(p.name, p.server_url ?? "")}>
                  {testing === p.name ? "Testing…" : "Test connection"}
                </button>
                <button type="button" className={btn} onClick={() => change("server_profile_remove", { name: p.name })}>Remove</button>
              </div>
              {r && (
                <p className={`mt-1 text-xs ${r.ok ? "text-green-600 dark:text-green-400" : "text-red-600"}`} role="status">
                  {describeProbe(r)}
                </p>
              )}
              {r?.untrusted_certificate && (
                <label className="mt-1 flex items-center gap-2 text-xs">
                  <input type="checkbox" checked={trust[p.name] === true} onChange={(e) => setTrust({ ...trust, [p.name]: e.target.checked })} />
                  Accept this server's untrusted certificate for the next test (only for a server you set up yourself)
                </label>
              )}
            </li>
          );
        })}
      </ul>
      <div className="flex flex-wrap gap-2">
        <input aria-label="Server name" className={`${field} w-32`} placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} />
        <input aria-label="Server address" className={`${field} flex-1 min-w-48`} placeholder="https://backup.example.com:8200" value={url} onChange={(e) => setUrl(e.target.value)} />
        <button type="button" className={btn} disabled={!name.trim() || !url.trim()} onClick={add}>Save server</button>
      </div>
      {error && <p className="text-xs text-red-600" role="alert">{error}</p>}
    </div>
  );
}

export function LocalServerSection() {
  if (!isTauriAvailable()) {
    return <p className="pt-3 text-xs text-gray-500">The local web server is available in the desktop app.</p>;
  }
  return (
    <>
      <LocalServer />
      <h3 className="pt-3 text-xs font-semibold uppercase tracking-wide text-gray-500">Saved servers</h3>
      <SavedServers />
    </>
  );
}
