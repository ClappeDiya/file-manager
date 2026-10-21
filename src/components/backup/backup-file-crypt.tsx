/**
 * Standalone encryption with the backup cipher (`.ufopenc` files) and manual
 * decryption of a single backup volume with its repository header.
 */
import { useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import { inputCls } from "@/lib/backup-helpers";
import { errorText, type CryptReport } from "@/stores/backup-store";
import { Banner, Btn, Field, Section } from "./backup-ui";

export function BackupFileCrypt() {
  const [mode, setMode] = useState<"encrypt" | "decrypt" | "volume">("encrypt");
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [header, setHeader] = useState("");
  const [pass, setPass] = useState("");
  const [preset, setPreset] = useState("default");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ tone: "success" | "error"; text: string } | null>(null);
  const run = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r =
        mode === "volume"
          ? await tauriInvoke<CryptReport>("backup_decrypt_volume", { volume: input, header, output, passphrase: pass || null })
          : await tauriInvoke<CryptReport>("backup_crypt_file", { decrypt: mode === "decrypt", input, output, passphrase: pass, preset: mode === "encrypt" ? preset : null });
      setMsg({ tone: "success", text: `Wrote ${output} (${formatBytes(r.output_bytes)} from ${formatBytes(r.input_bytes)}).` });
      setPass("");
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  const ready = input.trim() && output.trim() && (mode === "volume" ? header.trim() : pass);
  return (
    <Section title="Encrypt or decrypt files" hint="Encrypt any file with the backup cipher (AES-256-GCM, Argon2id key, tamper-evident chunks), or decrypt one backup volume by hand using the destination's repository header.">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <Field label="Action">
          <select className={inputCls} value={mode} onChange={(e) => setMode(e.target.value as typeof mode)} data-testid="crypt-mode">
            <option value="encrypt">Encrypt a file</option>
            <option value="decrypt">Decrypt a .ufopenc file</option>
            <option value="volume">Decrypt a backup volume</option>
          </select>
        </Field>
        {mode === "encrypt" && (
          <Field label="Key derivation">
            <select className={inputCls} value={preset} onChange={(e) => setPreset(e.target.value)}>
              <option value="interactive">Interactive</option>
              <option value="default">Standard</option>
              <option value="sensitive">Sensitive</option>
            </select>
          </Field>
        )}
        <Field label={mode === "volume" ? "Volume file (.dblock / .dindex / .dlist)" : "Input file"}>
          <input className={`${inputCls} font-mono`} value={input} onChange={(e) => setInput(e.target.value)} data-testid="crypt-input" />
        </Field>
        <Field label="Output file">
          <input className={`${inputCls} font-mono`} value={output} onChange={(e) => setOutput(e.target.value)} data-testid="crypt-output" />
        </Field>
        {mode === "volume" && (
          <Field label="Repository header (<prefix>-repo.json)">
            <input className={`${inputCls} font-mono`} value={header} onChange={(e) => setHeader(e.target.value)} data-testid="crypt-header" />
          </Field>
        )}
        <Field label={mode === "volume" ? "Passphrase (if encrypted)" : "Passphrase"}>
          <input className={inputCls} type="password" autoComplete="off" value={pass} onChange={(e) => setPass(e.target.value)} data-testid="crypt-pass" />
        </Field>
      </div>
      <Btn tone="primary" disabled={busy || !ready} onClick={() => void run()} testId="crypt-run">
        {busy ? "Working…" : mode === "encrypt" ? "Encrypt" : "Decrypt"}
      </Btn>
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.text}
        </Banner>
      )}
    </Section>
  );
}
