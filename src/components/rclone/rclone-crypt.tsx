/**
 * rclone crypt remotes (Advanced): the banner shown while browsing one,
 * with "Open underlying remote", and a small dialog that encrypts or
 * decrypts file names offline with the remote's keys
 * (`rclone cryptdecode [--reverse]`).
 */
import { useState } from "react";
import { errorText } from "@/lib/error-text";
import { rcloneCryptDecode, type RcloneCryptInfo, type RcloneCryptName } from "@/lib/rclone";

const btn = "text-xs px-2 py-0.5 rounded border border-amber-300 dark:border-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40";

export function CryptBanner({
  info,
  onOpenUnderlying,
  onNames,
}: {
  info: RcloneCryptInfo;
  onOpenUnderlying?: (remote: string) => void;
  onNames: () => void;
}) {
  if (!info.is_crypt) return null;
  return (
    <div
      className="px-3 py-1.5 text-xs bg-amber-50 dark:bg-amber-900/20 border-b border-amber-200 dark:border-amber-800 flex flex-wrap items-center gap-2"
      role="note"
      data-testid="crypt-banner"
    >
      <span>
        Encrypted remote: names and contents are decrypted locally; underlying remote:{" "}
        <span className="font-mono">{info.underlying ?? "unknown"}</span>
        {info.filename_encryption === "off" && " (file names are not encrypted)"}
      </span>
      <span className="ml-auto flex gap-1">
        {info.underlying_remote && onOpenUnderlying && (
          <button type="button" className={btn} onClick={() => onOpenUnderlying(info.underlying_remote!)}>
            Open underlying remote
          </button>
        )}
        <button type="button" className={btn} onClick={onNames}>
          Encrypt / decrypt names…
        </button>
      </span>
    </div>
  );
}

export function CryptNamesDialog({ remote, onClose }: { remote: string; onClose: () => void }) {
  const [text, setText] = useState("");
  const [reverse, setReverse] = useState(false);
  const [results, setResults] = useState<RcloneCryptName[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      setResults(await rcloneCryptDecode(remote, text.split("\n").filter((l) => l.trim()), reverse));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40" role="dialog" aria-modal="true" aria-label="Encrypt or decrypt names" onClick={onClose}>
      <div className="w-[min(560px,92vw)] rounded-lg bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-700 p-4 space-y-3" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-sm font-semibold">
          File names of <span className="font-mono">{remote}</span>
        </h3>
        <p className="text-xs text-gray-500">Translated offline with this remote's keys; nothing on the storage is read or changed.</p>
        <div className="flex gap-3 text-xs" role="radiogroup" aria-label="Direction">
          <label className="flex items-center gap-1">
            <input type="radio" checked={!reverse} onChange={() => setReverse(false)} /> Decrypt (encrypted → real names)
          </label>
          <label className="flex items-center gap-1">
            <input type="radio" checked={reverse} onChange={() => setReverse(true)} /> Encrypt (real → encrypted names)
          </label>
        </div>
        <textarea
          aria-label="Names, one per line"
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={5}
          placeholder={reverse ? "Documents/report.pdf" : "1tbpm0eqlu3hsb7bfd5fcn16c8/eq7kknqitracs7oj959q9alb5c"}
          className="w-full text-xs font-mono px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-950"
        />
        {error && (
          <p className="text-xs text-red-600" role="alert">
            {error}
          </p>
        )}
        {results && (
          <table className="w-full text-xs font-mono" data-testid="crypt-results">
            <tbody>
              {results.map((r) => (
                <tr key={r.input} className="border-t border-gray-100 dark:border-gray-800">
                  <td className="py-0.5 pr-2 break-all">{r.input}</td>
                  <td className={`py-0.5 break-all ${r.error ? "text-red-600" : ""}`}>{r.output ?? r.error}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <div className="flex justify-end gap-2">
          <button type="button" className="text-xs px-3 py-1 rounded border border-gray-300 dark:border-gray-600" onClick={onClose}>
            Close
          </button>
          <button type="button" className="text-xs px-3 py-1 rounded bg-blue-500 text-white disabled:opacity-50" disabled={busy || !text.trim()} onClick={() => void go()}>
            {busy ? "Working…" : reverse ? "Encrypt names" : "Decrypt names"}
          </button>
        </div>
      </div>
    </div>
  );
}
