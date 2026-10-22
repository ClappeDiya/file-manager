/**
 * Proxy (Settings): System / No proxy / Manual / PAC, with a bypass list,
 * optional credentials (kept in the credential store) and a connection
 * test. Applies to every outbound connection the app makes.
 */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

export type ProxyMode = "system" | "none" | "manual" | "pac";

/** Mirrors `net::ProxySettings`. */
export interface ProxySettings {
  mode: ProxyMode;
  url: string;
  pac_url: string;
  no_proxy: string[];
  username: string;
  managed: boolean;
}

interface ProxyView {
  settings: ProxySettings;
  has_password: boolean;
  os: { http: string | null; https: string | null; bypass: string[]; pac_url: string | null; env: string | null };
}

interface ProxyTest {
  ok: boolean;
  status: number | null;
  via: string | null;
  error: string | null;
  millis: number;
}

const MODES: { mode: ProxyMode; label: string }[] = [
  { mode: "system", label: "Use system settings" },
  { mode: "none", label: "No proxy" },
  { mode: "manual", label: "Manual" },
  { mode: "pac", label: "Automatic configuration (PAC)" },
];

const input = "w-full px-2 py-1 text-xs rounded border border-gray-200 dark:border-gray-700 bg-transparent";

export function ProxySection() {
  const [view, setView] = useState<ProxyView | null>(null);
  const [s, setS] = useState<ProxySettings | null>(null);
  const [password, setPassword] = useState<string | undefined>(undefined);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [test, setTest] = useState<ProxyTest | null>(null);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    tauriInvoke<ProxyView>("proxy_get").then(
      (v) => {
        setView(v);
        setS(v.settings);
      },
      (e) => setError(errorText(e)),
    );
  }, []);

  if (!s || !view) return <p className="pt-3 text-xs text-gray-400">Loading…</p>;
  const set = (patch: Partial<ProxySettings>) => {
    setS({ ...s, ...patch });
    setSaved(false);
  };
  const save = async () => {
    setError(null);
    try {
      const args: Record<string, unknown> = { settings: s };
      if (password !== undefined) args.password = password || null;
      setS(await tauriInvoke<ProxySettings>("proxy_set", args));
      setPassword(undefined);
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const runTest = async () => {
    setTesting(true);
    setTest(null);
    try {
      setTest(await tauriInvoke<ProxyTest>("proxy_test", { url: null }));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setTesting(false);
    }
  };
  const locked = s.managed;

  return (
    <div className="space-y-3 pt-3 text-xs">
      {locked && <p className="text-amber-600">Managed by your organization's console.</p>}
      <fieldset className="space-y-1" disabled={locked}>
        {MODES.map((m) => (
          <label key={m.mode} className="flex items-center gap-2">
            <input type="radio" name="proxy-mode" checked={s.mode === m.mode} onChange={() => set({ mode: m.mode })} /> {m.label}
          </label>
        ))}
      </fieldset>
      {s.mode === "system" && (
        <p className="text-gray-500" data-testid="proxy-system">
          {view.os.env
            ? `Environment: ${view.os.env}`
            : view.os.pac_url
              ? `PAC: ${view.os.pac_url}`
              : view.os.https || view.os.http
                ? `System proxy: ${view.os.https ?? view.os.http}`
                : "No system proxy configured — connections go direct."}
        </p>
      )}
      {s.mode === "manual" && (
        <label className="block">
          Proxy URL
          <input className={`${input} font-mono`} disabled={locked} placeholder="http://proxy.example.com:3128 or socks5://host:1080" value={s.url} onChange={(e) => set({ url: e.target.value })} />
        </label>
      )}
      {s.mode === "pac" && (
        <label className="block">
          PAC file URL or path
          <input className={`${input} font-mono`} disabled={locked} placeholder="http://wpad.example.com/wpad.dat" value={s.pac_url} onChange={(e) => set({ pac_url: e.target.value })} />
        </label>
      )}
      {s.mode === "manual" && (
        <label className="block">
          Don't use the proxy for (comma-separated)
          <input className={input} disabled={locked} placeholder="localhost, .corp.example, 10.*" value={s.no_proxy.join(", ")} onChange={(e) => set({ no_proxy: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })} />
        </label>
      )}
      {(s.mode === "manual" || s.mode === "pac") && (
        <div className="grid grid-cols-2 gap-2">
          <label className="block">
            Username (optional)
            <input className={input} disabled={locked} value={s.username} onChange={(e) => set({ username: e.target.value })} autoComplete="off" />
          </label>
          <label className="block">
            Password
            <input
              type="password"
              className={input}
              disabled={locked}
              autoComplete="new-password"
              placeholder={view.has_password && password === undefined ? "•••••• (saved)" : ""}
              value={password ?? ""}
              onChange={(e) => {
                setPassword(e.target.value);
                setSaved(false);
              }}
            />
          </label>
          {view.has_password && password === undefined && (
            <button type="button" disabled={locked} className="col-span-2 justify-self-start text-red-500 hover:underline" onClick={() => setPassword("")}>
              Remove saved password
            </button>
          )}
          {password === "" && <p className="col-span-2 text-amber-600">The saved password will be removed when you save.</p>}
        </div>
      )}
      <div className="flex gap-2 items-center">
        <button type="button" disabled={locked} className="px-3 py-1 rounded bg-blue-500 text-white disabled:opacity-40" onClick={save}>
          Save
        </button>
        <button type="button" disabled={testing} className="px-2 py-1 rounded border border-gray-200 dark:border-gray-700" onClick={runTest}>
          {testing ? "Testing…" : "Test connection"}
        </button>
        {saved && <span className="text-green-600">Saved</span>}
      </div>
      {test && (
        <p className={test.ok ? "text-green-600" : "text-red-500"} data-testid="proxy-test">
          {test.ok ? `Connected in ${test.millis} ms` : `Failed: ${test.error ?? `HTTP ${test.status}`}`} · {test.via ? `via ${test.via}` : "direct"}
        </p>
      )}
      {error && <p className="text-red-500" role="alert">{error}</p>}
    </div>
  );
}
