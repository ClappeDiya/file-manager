/**
 * Sign in a saved Google Drive / Dropbox / OneDrive connection.
 *
 * - Standard: "Sign in with …" opens the system browser on the provider's
 *   consent page with the app built into this build; UFOP waits for the
 *   browser to come back (loopback redirect, PKCE). A browser on another
 *   device can paste the code it was given instead.
 * - Builds without a built-in app say so plainly and point to Advanced
 *   view, where your own OAuth client ID (and secret) can be entered.
 * - Headless computers paste tokens, or an AuthID from a self-hosted
 *   sign-in broker (`ufop-server --oauth-config`).
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { errorText } from "@/lib/error-text";
import { PasswordInput } from "@/components/password-input";
import {
  awaitSignIn,
  beginSignIn,
  cancelSignIn,
  completeSignIn,
  importGrant,
  providerInfo,
  type OAuthProviderInfo,
  type OAuthSignedIn,
  type OAuthStart,
} from "@/lib/oauth";

const input =
  "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";
const button = "text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50";
const secondary = "text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50";

export function OAuthSignIn({
  connectionId,
  protocol,
  savedClientId,
  onSignedIn,
}: {
  connectionId: string;
  protocol: string;
  /** The connection's own OAuth client ID, if it has one. */
  savedClientId?: string | null;
  onSignedIn: (result: OAuthSignedIn) => void;
}) {
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const setAppMode = useUIStore((s) => s.setAppMode);
  const [info, setInfo] = useState<OAuthProviderInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<OAuthStart | null>(null);
  const [busy, setBusy] = useState(false);
  const [code, setCode] = useState("");
  const [clientId, setClientId] = useState(savedClientId ?? "");
  const [clientSecret, setClientSecret] = useState("");
  const [headless, setHeadless] = useState<"token" | "authid">("token");
  const [token, setToken] = useState("");
  const [brokerUrl, setBrokerUrl] = useState("");
  const [authid, setAuthid] = useState("");
  const stateRef = useRef<string | null>(null);

  useEffect(() => {
    providerInfo(protocol).then(setInfo, (e) => setError(errorText(e)));
  }, [protocol]);

  // Leaving the dialog abandons a sign-in still waiting for the browser.
  useEffect(
    () => () => {
      if (stateRef.current) void cancelSignIn(stateRef.current).catch(() => {});
    },
    [],
  );

  const finish = useCallback(
    (r: OAuthSignedIn) => {
      stateRef.current = null;
      setPending(null);
      onSignedIn(r);
    },
    [onSignedIn],
  );

  const customClient = advanced ? clientId.trim() : (savedClientId ?? "").trim();
  const canSignIn = !!info && (info.has_builtin_client || customClient !== "");

  const start = async () => {
    setError(null);
    setBusy(true);
    let started: string | null = null;
    try {
      const s = await beginSignIn(connectionId, customClient || null, advanced ? clientSecret : null);
      started = s.state;
      stateRef.current = s.state;
      setPending(s);
      setBusy(false);
      const signedIn = await awaitSignIn(s.state);
      // Already finished another way (a pasted code) or cancelled meanwhile.
      if (stateRef.current !== started) return;
      finish(signedIn);
    } catch (e) {
      // Finished another way (a pasted code) or cancelled: not an error here.
      if (started && stateRef.current !== started) return;
      stateRef.current = null;
      setPending(null);
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const submitCode = async () => {
    if (!pending || !code.trim()) return;
    setError(null);
    try {
      finish(await completeSignIn(pending.state, code));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const cancel = async () => {
    if (pending) await cancelSignIn(pending.state).catch(() => {});
    stateRef.current = null;
    setPending(null);
  };

  const submitHeadless = async () => {
    setError(null);
    setBusy(true);
    try {
      finish(await importGrant(connectionId, headless === "token" ? { token } : { brokerUrl, authid }));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const label = info?.label ?? "the service";

  return (
    <div className="space-y-3" data-testid="oauth-sign-in">
      {info && !info.has_builtin_client && !customClient && (
        <div className="text-xs rounded border border-amber-300 bg-amber-50 dark:bg-amber-900/20 dark:border-amber-700 p-2 space-y-1" data-testid="oauth-no-builtin">
          <p>
            This build of UFOP has no built-in {label} app, so it cannot open the {label} sign-in page by itself.
          </p>
          <p>
            Create your own OAuth app ({info.app_console}) and enter its client ID in Advanced view, or sign in with a
            token / AuthID below.
          </p>
          {!advanced && (
            <button type="button" className="text-blue-600 dark:text-blue-400 hover:underline" onClick={() => setAppMode("advanced")}>
              Switch to Advanced view
            </button>
          )}
        </div>
      )}

      {advanced && info && (
        <details className="rounded border border-gray-200 dark:border-gray-700 p-2" open={!info.has_builtin_client || !!savedClientId}>
          <summary className="text-xs font-medium cursor-pointer">Use your own OAuth app</summary>
          <div className="space-y-2 mt-2">
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="oauth-client-id">Client ID</label>
              <input id="oauth-client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} className={`${input} font-mono`} placeholder={info.has_builtin_client ? "(built-in app)" : ""} />
            </div>
            {info.uses_client_secret && (
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="oauth-client-secret">Client secret</label>
                <PasswordInput id="oauth-client-secret" value={clientSecret} onChange={(e) => setClientSecret(e.target.value)} className={`${input} font-mono`} />
              </div>
            )}
            <p className="text-[10px] text-gray-500 dark:text-gray-400">
              Register this redirect URI with your app: <code className="font-mono">{info.redirect_uri}</code>. The secret is
              stored with the tokens in the system keychain.
            </p>
          </div>
        </details>
      )}

      {!pending ? (
        <button type="button" className={button} disabled={!canSignIn || busy} onClick={() => void start()}>
          {busy ? "Opening…" : `Sign in with ${label}`}
        </button>
      ) : (
        <div className="space-y-2 rounded border border-blue-200 dark:border-blue-800 p-2" data-testid="oauth-waiting">
          <p className="text-xs">
            {pending.browser_opened ? "Finish signing in in your browser; UFOP is waiting." : "Open this page in a browser to sign in:"}{" "}
            <a href={pending.auth_url} target="_blank" rel="noreferrer" className="text-blue-600 dark:text-blue-400 underline break-all">
              sign-in page
            </a>
          </p>
          <label className="block text-xs font-medium" htmlFor="oauth-code">
            Browser on another device? Paste the code or the address it was sent to:
          </label>
          <div className="flex gap-2">
            <input id="oauth-code" value={code} onChange={(e) => setCode(e.target.value)} className={`${input} font-mono`} />
            <button type="button" className={secondary} disabled={!code.trim()} onClick={() => void submitCode()}>
              Use code
            </button>
          </div>
          <button type="button" className={secondary} onClick={() => void cancel()}>
            Cancel sign-in
          </button>
        </div>
      )}

      <details className="rounded border border-gray-200 dark:border-gray-700 p-2">
        <summary className="text-xs font-medium cursor-pointer">Sign in on a computer without a browser</summary>
        <div className="space-y-2 mt-2">
          <div className="flex gap-3 text-xs">
            <label className="flex items-center gap-1">
              <input type="radio" checked={headless === "token"} onChange={() => setHeadless("token")} /> Paste tokens
            </label>
            <label className="flex items-center gap-1">
              <input type="radio" checked={headless === "authid"} onChange={() => setHeadless("authid")} /> AuthID from a sign-in broker
            </label>
          </div>
          {headless === "token" ? (
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="oauth-token">Token</label>
              <textarea
                id="oauth-token"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                rows={3}
                className={`${input} font-mono text-xs`}
                placeholder={'{"access_token": "…", "refresh_token": "…", "client_id": "…"}'}
              />
              <p className="text-[10px] text-gray-500 dark:text-gray-400">An access token, or JSON with a refresh token and the client it belongs to.</p>
            </div>
          ) : (
            <div className="space-y-2">
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="oauth-broker">Broker URL</label>
                <input id="oauth-broker" value={brokerUrl} onChange={(e) => setBrokerUrl(e.target.value)} className={input} placeholder="https://ufop.example.com" />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="oauth-authid">AuthID</label>
                <PasswordInput id="oauth-authid" value={authid} onChange={(e) => setAuthid(e.target.value)} className={`${input} font-mono`} />
              </div>
              <p className="text-[10px] text-gray-500 dark:text-gray-400">
                Open <code>/oauth/{"{provider}"}/start</code> on your ufop-server to get an AuthID.
              </p>
            </div>
          )}
          <button type="button" className={secondary} disabled={busy || (headless === "token" ? !token.trim() : !brokerUrl.trim() || !authid.trim())} onClick={() => void submitHeadless()}>
            Check and save
          </button>
        </div>
      </details>

      {error && (
        <p className="text-xs text-red-600 dark:text-red-400" role="alert" data-testid="oauth-error">
          {error}
        </p>
      )}
    </div>
  );
}
