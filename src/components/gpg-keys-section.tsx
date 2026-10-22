/**
 * OpenPGP keys (Settings): the GnuPG keyring with expiry warnings,
 * creating a key pair, importing keys (file or pasted) and copying a
 * public key to share.
 */
import { useCallback, useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { EXPIRY_WARNING_DAYS, daysToExpiry, gpg, type GpgKey, type GpgStatus, type NewKey } from "@/stores/gpg-store";

const input = "w-full rounded border border-gray-200 dark:border-gray-700 bg-transparent px-1 py-0.5";

/** Create a key pair (Ed25519 by default) protected by a passphrase. */
function GenerateKey({ onCreated }: { onCreated: (fingerprint: string) => void }) {
  const [key, setKey] = useState<NewKey>({ name: "", email: "", algorithm: "ed25519", expires: "2y" });
  const [pass, setPass] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = (patch: Partial<NewKey>) => setKey((k) => ({ ...k, ...patch }));
  const mismatch = pass !== confirm;
  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const fp = await gpg.generateKey(key, pass || null);
      setPass("");
      setConfirm("");
      onCreated(fp);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <details className="rounded border border-gray-200 dark:border-gray-700 p-2">
      <summary className="cursor-pointer font-medium">Create a key pair</summary>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <label>
          Name
          <input className={input} value={key.name} onChange={(e) => set({ name: e.target.value })} />
        </label>
        <label>
          E-mail
          <input className={input} type="email" value={key.email} onChange={(e) => set({ email: e.target.value })} />
        </label>
        <label>
          Type
          <select className={input} value={key.algorithm} onChange={(e) => set({ algorithm: e.target.value as NewKey["algorithm"] })}>
            <option value="ed25519">Ed25519 (recommended)</option>
            <option value="rsa4096">RSA 4096 (older software)</option>
          </select>
        </label>
        <label>
          Expires
          <select className={input} value={key.expires} onChange={(e) => set({ expires: e.target.value })}>
            <option value="1y">in 1 year</option>
            <option value="2y">in 2 years</option>
            <option value="5y">in 5 years</option>
            <option value="never">never</option>
          </select>
        </label>
        <label>
          Passphrase
          <input className={input} type="password" autoComplete="new-password" value={pass} onChange={(e) => setPass(e.target.value)} />
        </label>
        <label>
          Repeat passphrase
          <input className={input} type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        </label>
      </div>
      {!pass && <p className="mt-1 text-amber-600">Without a passphrase, anyone who can read your keyring can use the key.</p>}
      {mismatch && confirm && <p className="mt-1 text-red-500">The passphrases don&apos;t match.</p>}
      {error && <p className="mt-1 text-red-500" role="alert">{error}</p>}
      <button type="button" disabled={busy || mismatch || !key.name.trim() || !key.email.trim()} className="mt-2 px-2 py-1 rounded border border-gray-200 dark:border-gray-700 disabled:opacity-40" onClick={create}>
        {busy ? "Creating…" : "Create key"}
      </button>
    </details>
  );
}

function expiryText(k: GpgKey): { text: string; tone: string } {
  if (k.revoked) return { text: "revoked", tone: "text-red-500" };
  const d = daysToExpiry(k);
  if (k.expired || (d !== null && d < 0)) return { text: "expired", tone: "text-red-500" };
  if (d === null) return { text: "never expires", tone: "text-gray-500" };
  const date = new Date((k.expires ?? 0) * 1000).toLocaleDateString();
  return d <= EXPIRY_WARNING_DAYS ? { text: `expires in ${d} day${d === 1 ? "" : "s"} (${date})`, tone: "text-amber-600" } : { text: `expires ${date}`, tone: "text-gray-500" };
}

export function GpgKeysSection() {
  const [status, setStatus] = useState<GpgStatus | null>(null);
  const [keys, setKeys] = useState<GpgKey[]>([]);
  const [paste, setPaste] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!isTauriAvailable()) return;
    try {
      const s = await gpg.status();
      setStatus(s);
      setKeys(s.available ? await gpg.keys() : []);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const doImport = async (src: { path?: string; text?: string }) => {
    setError(null);
    setMsg(null);
    try {
      const n = await gpg.importKeys(src);
      setMsg(`Imported ${n} key${n === 1 ? "" : "s"}.`);
      setPaste("");
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (status && !status.available) {
    return <p className="pt-3 text-xs text-gray-500">GnuPG (gpg) was not found. Install it to encrypt, sign and verify files with OpenPGP.</p>;
  }
  return (
    <div className="space-y-3 pt-3 text-xs">
      {status?.version && <p className="text-gray-500">{status.version}</p>}
      {keys.length === 0 ? (
        <p className="text-gray-400">No keys in your keyring.</p>
      ) : (
        <ul className="space-y-1.5" aria-label="OpenPGP keys">
          {keys.map((k) => {
            const e = expiryText(k);
            return (
              <li key={k.fingerprint} className="p-2 rounded border border-gray-200 dark:border-gray-700">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium truncate">{k.user_ids[0] ?? "(no user id)"}</span>
                  <button
                    type="button"
                    className="text-blue-600 hover:underline shrink-0"
                    onClick={async () => {
                      try {
                        await navigator.clipboard.writeText(await gpg.exportPublic(k.fingerprint));
                        setMsg("Public key copied.");
                      } catch (err) {
                        setError(errorText(err));
                      }
                    }}
                  >
                    Copy public key
                  </button>
                </div>
                <div className="font-mono text-gray-500 break-all">{k.fingerprint}</div>
                <div className="flex gap-2 flex-wrap">
                  <span>{k.algorithm}</span>
                  {k.secret && <span className="text-blue-600">your key</span>}
                  <span>{[k.can_encrypt && "encrypt", k.can_sign && "sign"].filter(Boolean).join(" · ") || "no usable capability"}</span>
                  <span className={e.tone} data-testid="key-expiry">{e.text}</span>
                </div>
              </li>
            );
          })}
        </ul>
      )}
      <GenerateKey
        onCreated={async (fp) => {
          setMsg(`Created key ${fp.slice(-16)}.`);
          await load();
        }}
      />
      <div className="space-y-1">
        <label className="block text-gray-500" htmlFor="gpg-paste">Import a key (paste an armored block)</label>
        <textarea id="gpg-paste" rows={3} className="w-full font-mono rounded border border-gray-200 dark:border-gray-700 bg-transparent p-1" value={paste} onChange={(e) => setPaste(e.target.value)} placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----" />
        <div className="flex gap-2">
          <button type="button" disabled={!paste.trim()} className="px-2 py-1 rounded border border-gray-200 dark:border-gray-700 disabled:opacity-40" onClick={() => doImport({ text: paste })}>
            Import pasted key
          </button>
          <button
            type="button"
            className="px-2 py-1 rounded border border-gray-200 dark:border-gray-700"
            onClick={async () => {
              const picked = await tauriInvoke<string[]>("pick_files", { title: "Key file to import" }).catch(() => []);
              for (const path of picked) await doImport({ path });
            }}
          >
            Import key file…
          </button>
        </div>
      </div>
      {msg && <p className="text-green-600">{msg}</p>}
      {error && <p className="text-red-500" role="alert">{error}</p>}
    </div>
  );
}
