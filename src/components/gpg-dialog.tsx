/**
 * OpenPGP dialog for the selection: encrypt (to keys or with a
 * passphrase), decrypt, re-encrypt in place (new keys or passphrase,
 * without a plaintext copy), sign (detached or inline) and verify. Results
 * are listed per file; outputs land next to the inputs under free names.
 * Legacy files without integrity protection are refused until the user
 * chooses to decrypt them anyway.
 */
import { useEffect, useMemo, useState } from "react";
import { ShieldCheck, ShieldAlert, X } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { errorText } from "@/lib/error-text";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { EXPIRY_WARNING_DAYS, daysToExpiry, gpg, keyLabel, useGpgDialog, type GpgKey, type GpgMode, type GpgOutcome, type SignatureCheck } from "@/stores/gpg-store";

const TITLES: Record<GpgMode, string> = { encrypt: "Encrypt", decrypt: "Decrypt", recrypt: "Re-encrypt", sign: "Sign", verify: "Verify signature" };
const field = "w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";

export function SignatureLine({ s }: { s: SignatureCheck }) {
  const ok = s.valid && !s.key_expired && !s.key_revoked;
  const Icon = ok ? ShieldCheck : ShieldAlert;
  return (
    <p className={`flex items-center gap-1 ${ok ? "text-green-600" : s.valid ? "text-amber-600" : "text-red-500"}`}>
      <Icon className="h-3.5 w-3.5 shrink-0" />
      <span>
        {s.message || (s.valid ? "Good signature" : "Signature not valid")}
        {s.signed_at ? ` · signed ${new Date(s.signed_at * 1000).toLocaleString()}` : ""}
      </span>
    </p>
  );
}

export function GpgDialog() {
  const mode = useGpgDialog((s) => s.mode);
  const paths = useGpgDialog((s) => s.paths);
  const close = useGpgDialog((s) => s.close);
  const [keys, setKeys] = useState<GpgKey[]>([]);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [recipients, setRecipients] = useState<string[]>([]);
  const [usePassphrase, setUsePassphrase] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  /** Re-encrypt: the passphrase that opens the files now. */
  const [oldPassphrase, setOldPassphrase] = useState("");
  const [armor, setArmor] = useState(false);
  const [detached, setDetached] = useState(true);
  const [signer, setSigner] = useState("");
  const [signToo, setSignToo] = useState(false);
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<GpgOutcome[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const containerRef = useFocusTrap<HTMLDivElement>({ active: !!mode, onEscape: close });

  useEffect(() => {
    if (!mode) return;
    setResults(null);
    setError(null);
    setPassphrase("");
    setOldPassphrase("");
    setRecipients([]);
    gpg
      .status()
      .then((s) => {
        setUnavailable(s.available ? null : s.error ?? "GnuPG was not found.");
        return s.available ? gpg.keys() : [];
      })
      .then((k) => {
        setKeys(k);
        setSigner(k.find((x) => x.secret && x.can_sign && !x.expired && !x.revoked)?.fingerprint ?? "");
      })
      .catch((e) => setUnavailable(errorText(e)));
  }, [mode]);

  const encryptTo = useMemo(() => keys.filter((k) => k.can_encrypt && !k.expired && !k.revoked), [keys]);
  const signers = useMemo(() => keys.filter((k) => k.secret && k.can_sign && !k.expired && !k.revoked), [keys]);
  if (!mode) return null;

  const pass = passphrase ? passphrase : null;
  const sealing = mode === "encrypt" || mode === "recrypt";
  const options = () => ({ recipients: usePassphrase ? [] : recipients, armor, sign_with: signToo && signer ? signer : null });
  const run = async (allowUnprotected = false, only: string[] = paths) => {
    setBusy(true);
    setError(null);
    try {
      let out: GpgOutcome[];
      if (mode === "encrypt") {
        out = await gpg.encrypt(only, options(), pass);
      } else if (mode === "recrypt") {
        out = await gpg.recrypt(only, options(), oldPassphrase || null, pass, allowUnprotected);
      } else if (mode === "decrypt") {
        out = await gpg.decrypt(only, pass, allowUnprotected);
      } else if (mode === "sign") {
        out = await gpg.sign(paths, signer || null, detached, armor, pass);
      } else {
        out = await Promise.all(only.map(async (p) => ({ input: p, output: null, signature: await gpg.verify(p), error: null, unprotected: false, needs_unprotected: false })));
      }
      // A retry replaces only the files it covered.
      setResults((prev) => (prev && only !== paths ? prev.map((r) => out.find((o) => o.input === r.input) ?? r) : out));
      if (!out.some((o) => o.needs_unprotected)) {
        setPassphrase("");
        setOldPassphrase("");
      }
      dispatchRefresh(out.flatMap((o) => (o.output ? [o.output] : [])));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const canRun =
    !unavailable &&
    !busy &&
    (!sealing || (usePassphrase ? passphrase.length > 0 : recipients.length > 0)) &&
    (mode !== "sign" || signers.length > 0);
  const n = paths.length;

  return (
    <div ref={containerRef} className="fixed inset-0 z-[60] flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-labelledby="gpg-title">
      <div className="absolute inset-0 bg-black/50" aria-hidden="true" onClick={close} />
      <div className="relative z-10 w-full max-w-md rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl text-xs">
        <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 id="gpg-title" className="text-sm font-semibold">
            {TITLES[mode]} {n} file{n === 1 ? "" : "s"} (OpenPGP)
          </h2>
          <Button variant="ghost" size="icon" onClick={close} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="space-y-3 px-4 py-3 max-h-[60vh] overflow-y-auto">
          {unavailable && (
            <p className="text-red-500" role="alert">
              {unavailable} Install GnuPG (gpg) to use these actions.
            </p>
          )}

          {mode === "recrypt" && !unavailable && (
            <>
              <p className="text-[color:var(--color-text-tertiary)]">The files are decrypted in memory and encrypted again in place, for the keys or passphrase you choose below. No unencrypted copy is written.</p>
              <label className="block">
                <span className="font-medium">Current passphrase (of the file, or of your key)</span>
                <input type="password" autoComplete="off" className={field} value={oldPassphrase} onChange={(e) => setOldPassphrase(e.target.value)} aria-label="Current passphrase" />
              </label>
            </>
          )}

          {sealing && !unavailable && (
            <>
              <div className="flex gap-3">
                <label className="flex items-center gap-1">
                  <input type="radio" checked={!usePassphrase} onChange={() => setUsePassphrase(false)} /> To keys
                </label>
                <label className="flex items-center gap-1">
                  <input type="radio" checked={usePassphrase} onChange={() => setUsePassphrase(true)} /> With a passphrase
                </label>
              </div>
              {!usePassphrase && (
                <fieldset className="space-y-1">
                  <legend className="font-medium mb-1">Recipients</legend>
                  {encryptTo.length === 0 && <p className="text-[color:var(--color-text-tertiary)]">No usable keys. Import the recipient's public key in Settings → OpenPGP Keys.</p>}
                  {encryptTo.map((k) => {
                    const days = daysToExpiry(k);
                    return (
                      <label key={k.fingerprint} className="flex items-center gap-2">
                        <input
                          type="checkbox"
                          checked={recipients.includes(k.fingerprint)}
                          onChange={(e) => setRecipients((r) => (e.target.checked ? [...r, k.fingerprint] : r.filter((x) => x !== k.fingerprint)))}
                        />
                        <span className="truncate">{keyLabel(k)}</span>
                        {days !== null && days <= EXPIRY_WARNING_DAYS && <span className="text-amber-600 shrink-0">expires in {days}d</span>}
                      </label>
                    );
                  })}
                </fieldset>
              )}
              {signers.length > 0 && (
                <label className="flex items-center gap-1">
                  <input type="checkbox" checked={signToo} onChange={(e) => setSignToo(e.target.checked)} /> Also sign it
                </label>
              )}
            </>
          )}

          {(mode === "sign" || (sealing && signToo)) && !unavailable && (
            <label className="block">
              <span className="font-medium">Sign as</span>
              {signers.length === 0 ? (
                <p className="text-[color:var(--color-text-tertiary)]">No secret key that can sign. Create or import one with GnuPG.</p>
              ) : (
                <select className={field} value={signer} onChange={(e) => setSigner(e.target.value)} aria-label="Signing key">
                  {signers.map((k) => (
                    <option key={k.fingerprint} value={k.fingerprint}>{keyLabel(k)}</option>
                  ))}
                </select>
              )}
            </label>
          )}

          {mode === "sign" && !unavailable && (
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={detached} onChange={(e) => setDetached(e.target.checked)} /> Separate signature file (keep the original untouched)
            </label>
          )}

          {(sealing || mode === "sign") && !unavailable && (
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={armor} onChange={(e) => setArmor(e.target.checked)} /> Text output (ASCII armor, .asc)
            </label>
          )}

          {mode !== "verify" && !unavailable && (usePassphrase || !sealing || signToo) && (
            <label className="block">
              <span className="font-medium">
                {sealing && usePassphrase ? (mode === "recrypt" ? "New passphrase" : "Passphrase") : "Passphrase (if the key or file needs one)"}
              </span>
              <input type="password" autoComplete="off" className={field} value={passphrase} onChange={(e) => setPassphrase(e.target.value)} aria-label="Passphrase" />
            </label>
          )}

          {results && (
            <ul className="space-y-1.5" aria-label="Results">
              {results.map((r) => (
                <li key={r.input} className="rounded border border-[var(--color-border)] p-1.5">
                  <div className="font-mono truncate" title={r.input}>{r.input.split(/[\\/]/).pop()}</div>
                  {r.error ? <p className="text-red-500">{r.error}</p> : r.output && <p className="text-green-600 truncate" title={r.output}>{mode === "recrypt" ? "re-encrypted" : `→ ${r.output.split(/[\\/]/).pop()}`}</p>}
                  {r.unprotected && <p className="text-amber-600">Decrypted without integrity protection: changes to this file could not be detected.</p>}
                  {r.needs_unprotected && (
                    <Button variant="outline" size="sm" className="mt-1" disabled={busy} onClick={() => void run(true, [r.input])}>
                      Decrypt anyway
                    </Button>
                  )}
                  {r.signature && <SignatureLine s={r.signature} />}
                </li>
              ))}
            </ul>
          )}
          {error && <p className="text-red-500" role="alert">{error}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <Button variant="ghost" size="sm" onClick={close}>{results ? "Done" : "Cancel"}</Button>
          {!results && (
            <Button size="sm" disabled={!canRun} onClick={() => void run()}>
              {busy ? "Working…" : TITLES[mode]}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
