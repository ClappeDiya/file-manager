/**
 * Recovery tool: salvage a damaged backup without its job database.
 * Steps mirror `ufop backup recovery …`: download (decrypted, resumable),
 * index, list versions, restore (gaps reported), convert to a new backup.
 */
import { useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import { formatDate, inputCls } from "@/lib/backup-helpers";
import {
  errorText,
  type RecoveryConvertReport,
  type RecoveryDownloadReport,
  type RecoveryIndexReport,
  type RecoveryRestoreReport,
  type RecoveryVersion,
  type VolumeFormat,
} from "@/stores/backup-store";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";
import { BackupFileCrypt } from "./backup-file-crypt";
import { AdvancedOnly } from "./advanced-only";

export function BackupRecoveryView() {
  const [source, setSource] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [prefix, setPrefix] = useState("ufop");
  const [folder, setFolder] = useState("");
  const [threads, setThreads] = useState(4);
  const [resume, setResume] = useState(true);
  const [kbps, setKbps] = useState(0);
  const [versions, setVersions] = useState<RecoveryVersion[] | null>(null);
  const [version, setVersion] = useState(0);
  const [paths, setPaths] = useState("");
  const [target, setTarget] = useState("");
  const [convOut, setConvOut] = useState("");
  const [convFormat, setConvFormat] = useState<VolumeFormat>("ufop");
  const [convPass, setConvPass] = useState("");
  const [msg, setMsg] = useState<{ tone: "success" | "warning" | "error"; lines: string[] } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const step = async (label: string, fn: () => Promise<{ tone: "success" | "warning"; lines: string[] }>) => {
    setBusy(label);
    setMsg(null);
    try {
      setMsg(await fn());
    } catch (e) {
      setMsg({ tone: "error", lines: [errorText(e)] });
    } finally {
      setBusy(null);
    }
  };

  const ready = folder.trim().length > 0;
  return (
    <div className="p-3 space-y-3" data-testid="backup-recovery">
      <Banner tone="info">
        For badly damaged backups: copies every volume to a local folder (decrypted — keep it private), then rebuilds files from whatever survived. Files with missing data are still written, with the gaps reported.
      </Banner>
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.lines.slice(0, 30).map((l) => (
            <div key={l}>{l}</div>
          ))}
        </Banner>
      )}

      <Section title="1. Download">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Backup folder (destination)">
            <PathInput value={source} onChange={setSource} testId="recovery-source" />
          </Field>
          <Field label="Local recovery folder">
            <PathInput value={folder} onChange={setFolder} testId="recovery-folder" />
          </Field>
          <Field label="Passphrase (empty if unencrypted)">
            <input className={inputCls} type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
          </Field>
          <Field label="File-name prefix">
            <input className={inputCls} value={prefix} onChange={(e) => setPrefix(e.target.value)} />
          </Field>
          <Field label="Parallel downloads">
            <input className={inputCls} type="number" min={1} max={32} value={threads} onChange={(e) => setThreads(Math.min(32, Math.max(1, Number(e.target.value) || 1)))} />
          </Field>
          <Field label="Download limit (KB/s, 0 = unlimited)">
            <input className={inputCls} type="number" min={0} value={kbps} onChange={(e) => setKbps(Math.max(0, Number(e.target.value) || 0))} />
          </Field>
        </div>
        <Check label="Continue an interrupted download (skip volumes already copied)" checked={resume} onChange={setResume} />
        <Btn
          tone="primary"
          testId="recovery-download"
          disabled={busy !== null || !source.trim() || !ready}
          onClick={() =>
            void step("download", async () => {
              const r = await tauriInvoke<RecoveryDownloadReport>("backup_recovery_download", {
                destination: { type: "local", path: source, alternate_paths: [], marker_file: null },
                passphrase: passphrase || null,
                prefix,
                folder,
                options: { threads, resume, max_bytes_per_sec: kbps * 1024 },
              });
              return {
                tone: r.failed.length ? "warning" : "success",
                lines: [`Downloaded ${r.downloaded} volume(s) (${formatBytes(r.bytes)}), ${r.skipped_existing} already present.`, ...r.failed.map((f) => `Failed: ${f}`)],
              };
            })
          }
        >
          {busy === "download" ? "Downloading…" : "Download"}
        </Btn>
      </Section>

      <Section title="2. Index and list versions">
        <Btn
          testId="recovery-index"
          disabled={busy !== null || !ready}
          onClick={() =>
            void step("index", async () => {
              const ix = await tauriInvoke<RecoveryIndexReport>("backup_recovery_index", { folder });
              const v = await tauriInvoke<RecoveryVersion[]>("backup_recovery_list", { folder });
              setVersions(v);
              return {
                tone: ix.damaged.length ? "warning" : "success",
                lines: [`Indexed ${ix.blocks} block(s) in ${ix.block_volumes} volume(s); ${v.length} version(s) found.`, ...ix.damaged.map((d) => `Damaged: ${d}`)],
              };
            })
          }
        >
          {busy === "index" ? "Indexing…" : "Index"}
        </Btn>
      </Section>

      {versions && (
        <Section title="3. Restore">
          <select className={inputCls} value={version} onChange={(e) => setVersion(Number(e.target.value))} data-testid="recovery-version">
            {versions.map((v) => (
              <option key={v.volume} value={v.version}>
                {formatDate(v.timestamp)} · {v.files} items{v.partial ? " · partial" : ""}
              </option>
            ))}
          </select>
          <Field label="Paths or wildcards (one per line; empty = everything)">
            <textarea className={`${inputCls} font-mono`} rows={2} value={paths} onChange={(e) => setPaths(e.target.value)} />
          </Field>
          <Field label="Restore into">
            <PathInput value={target} onChange={setTarget} testId="recovery-target" />
          </Field>
          <Btn
            tone="primary"
            testId="recovery-restore"
            disabled={busy !== null || !target.trim()}
            onClick={() =>
              void step("restore", async () => {
                const r = await tauriInvoke<RecoveryRestoreReport>("backup_recovery_restore", {
                  folder,
                  version,
                  paths: paths.split("\n").map((s) => s.trim()).filter(Boolean),
                  target,
                });
                return {
                  tone: r.files_partial.length || r.errors.length ? "warning" : "success",
                  lines: [
                    `Restored ${r.files_restored} complete file(s) (${formatBytes(r.bytes_written)}); ${r.files_partial.length} with missing data.`,
                    ...r.files_partial.map((f) => `Partial: ${f}`),
                    ...r.errors,
                  ],
                };
              })
            }
          >
            {busy === "restore" ? "Restoring…" : "Restore"}
          </Btn>
        </Section>
      )}

      <Section title="4. Convert to a new backup (optional)" hint="Writes the recovered data as a fresh repository, e.g. to change the container format or passphrase.">
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <Field label="New backup folder">
            <PathInput value={convOut} onChange={setConvOut} testId="recovery-convert-out" />
          </Field>
          <Field label="Container">
            <select className={inputCls} value={convFormat} onChange={(e) => setConvFormat(e.target.value as VolumeFormat)}>
              <option value="ufop">UFOP</option>
              <option value="zip">ZIP</option>
            </select>
          </Field>
          <Field label="New passphrase (empty = unencrypted)">
            <input className={inputCls} type="password" value={convPass} onChange={(e) => setConvPass(e.target.value)} />
          </Field>
        </div>
        <Btn
          disabled={busy !== null || !ready || !convOut.trim()}
          onClick={() =>
            void step("convert", async () => {
              const r = await tauriInvoke<RecoveryConvertReport>("backup_recovery_convert", {
                folder,
                destination: { type: "local", path: convOut, alternate_paths: [], marker_file: null },
                options: { prefix, volume_format: convFormat, zip_method: "deflate", compression_level: 6, passphrase: convPass || null },
              });
              return {
                tone: r.skipped.length ? "warning" : "success",
                lines: [`Wrote ${r.block_volumes} data volume(s) and ${r.file_lists} version(s) (${formatBytes(r.bytes_uploaded)}).`, ...r.skipped.map((x) => `Skipped: ${x}`)],
              };
            })
          }
        >
          {busy === "convert" ? "Converting…" : "Convert"}
        </Btn>
      </Section>
      <AdvancedOnly hint>
        <BackupFileCrypt />
      </AdvancedOnly>
    </div>
  );
}
