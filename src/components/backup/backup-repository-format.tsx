/**
 * Repository format (format 1 / format 2) UI:
 *
 * - `RepositoryFormatOptions`: creation-time choices for a new repository
 *   (format version, chunker, block codec, cipher, layout). Lives in the
 *   job editor's advanced options; locked once the first version exists.
 * - `BackupRepositoryFormat`: maintenance tools for an existing repository
 *   (show format, upgrade to format 2, roll back, reshard). Advanced only.
 * - `BackupChangePassphrase`: change an encrypted job's passphrase in place
 *   (format 2 rewrites only the repository header). Standard mode.
 */
import { useState } from "react";
import { inputCls } from "@/lib/backup-helpers";
import { formatBytes } from "@/lib/format-bytes";
import {
  describeChunker,
  describeLayout,
  errorText,
  useBackupStore,
  type BackupJob,
  type BackupOptions,
  type BlockCodec,
  type ChunkerKind,
  type CipherKind,
  type KdfPreset,
  type OperationResult,
  type RepoLayout,
  type RepositoryFormatInfo,
  type ReshardReport,
} from "@/stores/backup-store";
import { Banner, Btn, Field, Section } from "./backup-ui";

/** Creation-time format options; format 1 disables the format 2 choices. */
export function RepositoryFormatOptions({ options: o, locked, onChange }: { options: BackupOptions; locked: boolean; onChange: (p: Partial<BackupOptions>) => void }) {
  const v1 = o.format_version === 1;
  const aes = o.encryption_module === "aes";
  const sharded = o.layout.type === "sharded";
  return (
    <Section
      title="Repository format"
      hint="Applies when the backup is created. Format 2 adds content-defined chunking (better deduplication after edits), zstd compression, a password that can change without rewriting data, ChaCha20-Poly1305 and sharded folders. Format 1 matches older app versions."
    >
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
        <Field label="Format">
          <select
            data-testid="opt-format-version"
            className={inputCls}
            disabled={locked}
            value={v1 ? "1" : "2"}
            onChange={(e) =>
              onChange(
                e.target.value === "1"
                  ? { format_version: 1, chunker: null, block_codec: null, metadata_compression: null, cipher: "aes256_gcm", layout: { type: "flat" } }
                  : { format_version: null },
              )
            }
          >
            <option value="2">Format 2 (recommended)</option>
            <option value="1">Format 1 (compatible with older versions)</option>
          </select>
        </Field>
        <Field label="Chunking">
          <select data-testid="opt-chunker" className={inputCls} disabled={locked || v1} value={v1 ? "fixed" : (o.chunker ?? "fastcdc")} onChange={(e) => onChange({ chunker: e.target.value as ChunkerKind })}>
            <option value="fastcdc">Content-defined (FastCDC)</option>
            <option value="fixed">Fixed-size blocks</option>
          </select>
        </Field>
        <Field label="Block compression">
          <select data-testid="opt-block-codec" className={inputCls} disabled={locked || v1} value={v1 ? "deflate" : (o.block_codec ?? "zstd")} onChange={(e) => onChange({ block_codec: e.target.value as BlockCodec })}>
            <option value="zstd">Zstandard</option>
            <option value="deflate">Deflate</option>
          </select>
        </Field>
        <Field label="Cipher" hint={aes ? undefined : "Only for the built-in encryption method."}>
          <select data-testid="opt-cipher" className={inputCls} disabled={locked || v1 || !aes} value={o.cipher} onChange={(e) => onChange({ cipher: e.target.value as CipherKind })}>
            <option value="aes256_gcm">AES-256-GCM</option>
            <option value="chacha20_poly1305">ChaCha20-Poly1305</option>
          </select>
        </Field>
        <Field label="Folder layout" hint="Sharded keeps folders small; needs a local, network or folder-capable destination.">
          <select
            data-testid="opt-layout"
            className={inputCls}
            disabled={locked || v1}
            value={o.layout.type}
            onChange={(e) => onChange({ layout: e.target.value === "sharded" ? { type: "sharded", prefix_len: 2 } : { type: "flat" } })}
          >
            <option value="flat">Flat</option>
            <option value="sharded">Sharded subfolders</option>
          </select>
        </Field>
        {sharded && (
          <Field label="Shard name length">
            <select
              className={inputCls}
              disabled={locked || v1}
              value={o.layout.type === "sharded" ? o.layout.prefix_len : 2}
              onChange={(e) => onChange({ layout: { type: "sharded", prefix_len: Number(e.target.value) } })}
            >
              <option value={1}>1 hex digit (16 folders)</option>
              <option value={2}>2 hex digits (256 folders)</option>
              <option value={3}>3 hex digits (4096 folders)</option>
            </select>
          </Field>
        )}
      </div>
    </Section>
  );
}

/** Format info, upgrade / rollback and reshard for an existing repository. */
export function BackupRepositoryFormat({ job }: { job: BackupJob }) {
  const execute = useBackupStore((s) => s.execute);
  const [info, setInfo] = useState<RepositoryFormatInfo | null>(null);
  const [result, setResult] = useState<OperationResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<"upgrade" | "rollback" | "reshard" | null>(null);
  const [target, setTarget] = useState<RepoLayout>({ type: "sharded", prefix_len: 2 });
  const [preview, setPreview] = useState<ReshardReport | null>(null);

  const run = async (op: Parameters<typeof execute>[1]): Promise<OperationResult | null> => {
    setBusy(true);
    setError(null);
    try {
      const r = await execute(job.id, op);
      setResult(r);
      return r;
    } catch (e) {
      setError(errorText(e));
      return null;
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };
  const refresh = async () => {
    const r = await run({ type: "format_info" });
    if (r) setInfo(r.report as unknown as RepositoryFormatInfo);
  };
  const previewReshard = async () => {
    const r = await run({ type: "reshard", layout: target, dry_run: true });
    if (r) setPreview(r.report as unknown as ReshardReport);
  };

  return (
    <Section title="Repository format" hint="Format 2 reads all format 1 data. Upgrading rewrites only the repository header; until the next backup or maintenance write it can be rolled back.">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {result && !error && (
        <Banner tone={result.status === "success" ? "success" : "warning"} onClose={() => setResult(null)}>
          {typeof result.report.message === "string" ? result.report.message : result.summary}
        </Banner>
      )}
      <div className="flex gap-1 flex-wrap">
        <Btn disabled={busy} onClick={refresh} testId="backup-format-info">
          Show format
        </Btn>
        {info && info.format_version < 2 && (
          <Btn tone="primary" disabled={busy} onClick={() => setConfirm("upgrade")} testId="backup-format-upgrade">
            Upgrade to format 2…
          </Btn>
        )}
        {info?.rollback_available && (
          <Btn disabled={busy} onClick={() => setConfirm("rollback")} testId="backup-format-rollback">
            Roll back to format 1…
          </Btn>
        )}
      </div>
      {info && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs" data-testid="backup-format-details">
          <dt className="text-zinc-500">Format</dt>
          <dd>{info.format_version}</dd>
          <dt className="text-zinc-500">Chunking</dt>
          <dd>{describeChunker(info.chunker)}</dd>
          <dt className="text-zinc-500">Compression</dt>
          <dd>
            {info.block_codec === "zstd" ? "Zstandard" : "Deflate"} blocks, {info.metadata_compression === "zstd" ? "compressed" : "uncompressed"} file lists
          </dd>
          <dt className="text-zinc-500">Encryption</dt>
          <dd>{info.cipher ? `${info.cipher === "chacha20_poly1305" ? "ChaCha20-Poly1305" : "AES-256-GCM"}${info.envelope_key ? ", changeable password" : ""}` : "None or GPG"}</dd>
          <dt className="text-zinc-500">Layout</dt>
          <dd>{describeLayout(info.layout)}</dd>
        </dl>
      )}
      {confirm === "upgrade" && (
        <Banner tone="warning">
          <p>Upgrade this backup to format 2? Older app versions will no longer open it once new data is written. Existing data stays as it is.</p>
          <div className="flex gap-1 mt-1">
            <Btn tone="primary" disabled={busy} onClick={() => run({ type: "upgrade_format", rollback: false }).then(refresh)} testId="backup-format-upgrade-confirm">
              Upgrade
            </Btn>
            <Btn onClick={() => setConfirm(null)}>Cancel</Btn>
          </div>
        </Banner>
      )}
      {confirm === "rollback" && (
        <Banner tone="warning">
          <p>Restore the saved format 1 header? Nothing has been written in format 2 since the upgrade.</p>
          <div className="flex gap-1 mt-1">
            <Btn tone="danger" disabled={busy} onClick={() => run({ type: "upgrade_format", rollback: true }).then(refresh)} testId="backup-format-rollback-confirm">
              Roll back
            </Btn>
            <Btn onClick={() => setConfirm(null)}>Cancel</Btn>
          </div>
        </Banner>
      )}
      {info && info.format_version >= 2 && (
        <div className="flex gap-2 items-end flex-wrap">
          <Field label="Move volumes to">
            <select
              data-testid="backup-reshard-target"
              className={inputCls}
              value={target.type === "flat" ? "flat" : String(target.prefix_len)}
              onChange={(e) => {
                setPreview(null);
                setTarget(e.target.value === "flat" ? { type: "flat" } : { type: "sharded", prefix_len: Number(e.target.value) });
              }}
            >
              <option value="flat">Flat (one folder)</option>
              <option value="1">Sharded, 1-digit subfolders</option>
              <option value="2">Sharded, 2-digit subfolders</option>
              <option value="3">Sharded, 3-digit subfolders</option>
            </select>
          </Field>
          <Btn disabled={busy} onClick={previewReshard} testId="backup-reshard-preview">
            Preview
          </Btn>
          <Btn disabled={busy || !preview || preview.moved === 0} onClick={() => setConfirm("reshard")} testId="backup-reshard">
            Reshard…
          </Btn>
        </div>
      )}
      {preview && (
        <p className="text-xs text-zinc-600 dark:text-zinc-400" data-testid="backup-reshard-summary">
          {preview.moved} volume(s) ({formatBytes(preview.bytes_moved)}) would move; {preview.already_in_place} already in place.
        </p>
      )}
      {confirm === "reshard" && preview && (
        <Banner tone="warning">
          <p>Move {preview.moved} volume(s) to {describeLayout(target)}? Each is copied, checked and then removed from its old place; run it again to finish if interrupted.</p>
          <div className="flex gap-1 mt-1">
            <Btn
              tone="primary"
              disabled={busy}
              onClick={() =>
                run({ type: "reshard", layout: target, dry_run: false }).then((r) => {
                  setPreview(null);
                  if (r) void refresh();
                })
              }
              testId="backup-reshard-confirm"
            >
              Move volumes
            </Btn>
            <Btn onClick={() => setConfirm(null)}>Cancel</Btn>
          </div>
        </Banner>
      )}
    </Section>
  );
}

/** Change an encrypted job's passphrase (format 2: header only). */
export function BackupChangePassphrase({ job }: { job: BackupJob }) {
  const changePassphrase = useBackupStore((s) => s.changePassphrase);
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [preset, setPreset] = useState<KdfPreset | "">("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  if (!job.encrypted || job.options.encryption_module !== "aes") return null;
  const mismatch = repeat !== "" && next !== repeat;
  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await changePassphrase(job.id, current || null, next, preset || null);
      setDone(r.message);
      setOpen(false);
      setCurrent("");
      setNext("");
      setRepeat("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="space-y-2" data-testid="backup-change-passphrase">
      {done && <Banner tone="success" onClose={() => setDone(null)}>{done}</Banner>}
      {!open ? (
        <Btn onClick={() => setOpen(true)} testId="backup-change-passphrase-open" title="Format 2 backups change only the repository header; format 1 backups need an upgrade first (Maintenance)">
          Change password…
        </Btn>
      ) : (
        <div className="space-y-2 border border-zinc-200 dark:border-zinc-700 rounded p-2">
          {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
          <Field label="Current passphrase" hint="Leave empty to use the one stored for this job.">
            <input className={inputCls} type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} data-testid="backup-current-passphrase" />
          </Field>
          <Field label="New passphrase">
            <input className={inputCls} type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} data-testid="backup-new-passphrase" />
          </Field>
          <Field label="Repeat new passphrase" hint={mismatch ? "The passphrases do not match." : undefined}>
            <input className={inputCls} type="password" autoComplete="new-password" value={repeat} onChange={(e) => setRepeat(e.target.value)} data-testid="backup-repeat-passphrase" />
          </Field>
          <Field label="Key strength">
            <select className={inputCls} value={preset} onChange={(e) => setPreset(e.target.value as KdfPreset | "")}>
              <option value="">Keep current</option>
              <option value="interactive">Interactive (faster)</option>
              <option value="default">Default</option>
              <option value="sensitive">Sensitive (slower, stronger)</option>
            </select>
          </Field>
          <p className="text-[11px] text-amber-600">Anyone restoring this backup will need the new passphrase. Store it somewhere safe.</p>
          <div className="flex gap-1">
            <Btn tone="primary" disabled={busy || !next || next !== repeat} onClick={submit} testId="backup-change-passphrase-confirm">
              {busy ? "Changing…" : "Change password"}
            </Btn>
            <Btn onClick={() => setOpen(false)}>Cancel</Btn>
          </div>
        </div>
      )}
    </div>
  );
}
