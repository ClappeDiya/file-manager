/**
 * Engine options added for Duplicati parity: volume format, PAR2, metadata,
 * change detection, control files, performance, capacity and maintenance
 * cadence. Each field maps 1:1 onto `BackupOptions` in ufop-core.
 */
import { inputCls } from "@/lib/backup-helpers";
import { KDF_PRESETS, parseTags, tagText, type BackupOptions, type LockMode, type SnapshotPolicy, type UsnPolicy, type VolumeFormat, type ZipMethod } from "@/stores/backup-store";
import { Check, Field, Section } from "./backup-ui";
import { RepositoryFormatOptions } from "./backup-repository-format";

const lines = (v: string) => v.split("\n").map((s) => s.trim()).filter(Boolean);
const num = (v: string, min = 0, max = Number.MAX_SAFE_INTEGER) => Math.min(max, Math.max(min, Math.floor(Number(v) || 0)));
const optNum = (v: string) => (v.trim() === "" ? null : Math.max(0, Math.floor(Number(v) || 0)));

export function BackupAdvancedOptions({
  options: o,
  locked,
  onChange,
}: {
  options: BackupOptions;
  /** Repository-fixed settings cannot change after the first version. */
  locked: boolean;
  onChange: (p: Partial<BackupOptions>) => void;
}) {
  const preset = o.kdf ? (Object.entries(KDF_PRESETS).find(([, k]) => k.memory_kib === o.kdf!.memory_kib && k.iterations === o.kdf!.iterations && k.parallelism === o.kdf!.parallelism)?.[0] ?? "custom") : "app";
  return (
    <>
      <Section title="Encryption method" hint="Applies when the backup is created; an existing backup keeps its method and key settings.">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Cipher">
            <select data-testid="opt-encryption-module" className={inputCls} disabled={locked} value={o.encryption_module} onChange={(e) => onChange({ encryption_module: e.target.value as BackupOptions["encryption_module"] })}>
              <option value="aes">AES-256-GCM (built in)</option>
              <option value="gpg">GPG / OpenPGP (needs GnuPG)</option>
            </select>
          </Field>
          {o.encryption_module === "aes" && (
            <Field label="Key derivation (Argon2id)" hint="Stronger settings slow down passphrase guessing and opening the backup.">
              <select
                data-testid="opt-kdf"
                className={inputCls}
                disabled={locked}
                value={preset}
                onChange={(e) => onChange({ kdf: e.target.value === "app" ? null : e.target.value === "custom" ? (o.kdf ?? KDF_PRESETS.default) : KDF_PRESETS[e.target.value] })}
              >
                <option value="app">App default</option>
                <option value="interactive">Interactive (19 MiB, 2 passes)</option>
                <option value="default">Standard (64 MiB, 3 passes)</option>
                <option value="sensitive">Sensitive (256 MiB, 4 passes)</option>
                <option value="custom">Custom…</option>
              </select>
            </Field>
          )}
        </div>
        {o.encryption_module === "aes" && preset === "custom" && o.kdf && (
          <div className="grid grid-cols-3 gap-2">
            <Field label="Memory (KiB, ≥ 8192)">
              <input className={inputCls} type="number" min={8192} value={o.kdf.memory_kib} onChange={(e) => onChange({ kdf: { ...o.kdf!, memory_kib: num(e.target.value, 8192, 4 * 1024 * 1024) } })} />
            </Field>
            <Field label="Passes (1–50)">
              <input className={inputCls} type="number" min={1} max={50} value={o.kdf.iterations} onChange={(e) => onChange({ kdf: { ...o.kdf!, iterations: num(e.target.value, 1, 50) } })} />
            </Field>
            <Field label="Lanes (1–64)">
              <input className={inputCls} type="number" min={1} max={64} value={o.kdf.parallelism} onChange={(e) => onChange({ kdf: { ...o.kdf!, parallelism: num(e.target.value, 1, 64) } })} />
            </Field>
          </div>
        )}
        {o.encryption_module === "gpg" && (
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            <Field label="Recipient keys (one per line)" hint="Empty = symmetric with the backup passphrase. With keys, restores need the secret key.">
              <textarea data-testid="opt-gpg-recipients" className={`${inputCls} font-mono`} rows={2} value={o.gpg.recipients.join("\n")} onChange={(e) => onChange({ gpg: { ...o.gpg, recipients: lines(e.target.value) } })} />
            </Field>
            <Field label="Keyring folder (empty = default)">
              <input className={inputCls} value={o.gpg.homedir} onChange={(e) => onChange({ gpg: { ...o.gpg, homedir: e.target.value } })} />
            </Field>
            <Field label="gpg executable (empty = gpg)">
              <input className={inputCls} value={o.gpg.executable} onChange={(e) => onChange({ gpg: { ...o.gpg, executable: e.target.value } })} />
            </Field>
          </div>
        )}
      </Section>
      <Section title="Volume format" hint="ZIP volumes open with any ZIP tool when unencrypted. Fixed after the first backup.">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Container">
            <select data-testid="opt-volume-format" className={inputCls} disabled={locked} value={o.volume_format} onChange={(e) => onChange({ volume_format: e.target.value as VolumeFormat })}>
              <option value="ufop">UFOP (compact)</option>
              <option value="zip">ZIP / ZIP64</option>
            </select>
          </Field>
          <Field label="ZIP compression method">
            <select className={inputCls} disabled={o.volume_format !== "zip"} value={o.zip_method} onChange={(e) => onChange({ zip_method: e.target.value as ZipMethod })}>
              <option value="deflate">Deflate</option>
              <option value="stored">Stored</option>
              <option value="bzip2">BZip2</option>
              <option value="zstd">Zstandard</option>
            </select>
          </Field>
        </div>
      </Section>

      <RepositoryFormatOptions options={o} locked={locked} onChange={onChange} />
      <Section title="Immutability (object lock)" hint="Locks each new volume so nobody — including ransomware with your credentials — can delete or overwrite it before the lock expires. The destination must support object lock (e.g. an S3 bucket with Object Lock enabled).">
        <Check
          label="Lock new volumes"
          checked={o.object_lock !== null}
          onChange={(v) => onChange({ object_lock: v ? { days: 30, mode: "governance" } : null })}
        />
        {o.object_lock && (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            <Field label="Keep locked for (days)">
              <input data-testid="opt-lock-days" className={inputCls} type="number" min={1} value={o.object_lock.days} onChange={(e) => onChange({ object_lock: { ...o.object_lock!, days: num(e.target.value, 1, 36500) } })} />
            </Field>
            <Field label="Mode" hint="Compliance locks cannot be removed by anyone, not even the account owner.">
              <select className={inputCls} value={o.object_lock.mode} onChange={(e) => onChange({ object_lock: { ...o.object_lock!, mode: e.target.value as LockMode } })}>
                <option value="governance">Governance</option>
                <option value="compliance">Compliance</option>
              </select>
            </Field>
          </div>
        )}
      </Section>

      <Section title="PAR2 recovery data" hint="Uploads parity files next to each volume so damaged volumes repair themselves on restore. Needs the par2 tool.">
        <Check label="Create PAR2 recovery files" checked={o.par2.enabled} onChange={(v) => onChange({ par2: { ...o.par2, enabled: v } })} />
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Redundancy (%)">
            <input className={inputCls} type="number" min={1} max={100} disabled={!o.par2.enabled} value={o.par2.redundancy_percent} onChange={(e) => onChange({ par2: { ...o.par2, redundancy_percent: num(e.target.value, 1, 100) } })} />
          </Field>
          <Field label="par2 program">
            <input className={`${inputCls} font-mono`} disabled={!o.par2.enabled} value={o.par2.binary} onChange={(e) => onChange({ par2: { ...o.par2, binary: e.target.value || "par2" } })} />
          </Field>
        </div>
      </Section>

      <Section title="Snapshots" hint="Read a frozen point-in-time copy so open or changing files are consistent: VSS on Windows, LVM on Linux, APFS on macOS (administrator/root rights needed). Test it from Maintenance.">
        <Field label="Use a snapshot">
          <select data-testid="opt-snapshot" className={inputCls} value={o.snapshot_policy} onChange={(e) => onChange({ snapshot_policy: e.target.value as SnapshotPolicy })}>
            <option value="off">Off (read live files)</option>
            <option value="auto">Auto (silently fall back)</option>
            <option value="on">On (warn on fallback)</option>
            <option value="required">Required (fail without it)</option>
          </select>
        </Field>
        {o.snapshot_policy !== "off" && (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            <Field label="LVM snapshot size" hint="Extents like 10%ORIGIN or a size like 2G.">
              <input className={`${inputCls} font-mono`} value={o.snapshot.lvm_size} onChange={(e) => onChange({ snapshot: { ...o.snapshot, lvm_size: e.target.value.trim() || "10%ORIGIN" } })} />
            </Field>
            <Field label="Mount folder for snapshots">
              <input className={`${inputCls} font-mono`} value={o.snapshot.mount_root ?? ""} placeholder="system temp" onChange={(e) => onChange({ snapshot: { ...o.snapshot, mount_root: e.target.value || null } })} />
            </Field>
            <Field label="VSS provider id (GUID, optional)">
              <input className={`${inputCls} font-mono`} value={o.snapshot.vss_provider ?? ""} onChange={(e) => onChange({ snapshot: { ...o.snapshot, vss_provider: e.target.value.trim() || null } })} />
            </Field>
            <Field label="Excluded VSS writer ids (one per line)" hint="See `vssadmin list writers`.">
              <textarea className={`${inputCls} font-mono h-12`} value={o.snapshot.vss_exclude_writers.join("\n")} onChange={(e) => onChange({ snapshot: { ...o.snapshot, vss_exclude_writers: lines(e.target.value) } })} />
            </Field>
          </div>
        )}
      </Section>

      <Section title="Metadata">
        <Check label="Extended attributes" checked={o.backup_xattrs} onChange={(v) => onChange({ backup_xattrs: v })} />
        <Check label="ACLs, file flags and Windows security descriptors" checked={o.backup_acls} onChange={(v) => onChange({ backup_acls: v })} />
        <Check label="NTFS alternate data streams (Windows)" checked={o.backup_alternate_streams} onChange={(v) => onChange({ backup_alternate_streams: v })} />
        <Check label="Ignore exclusion attributes (Time Machine / xdg.robots)" checked={o.ignore_exclusion_xattrs} onChange={(v) => onChange({ ignore_exclusion_xattrs: v })} />
        <Check label="Skip files locked by another program (advisory locks)" checked={o.respect_advisory_locks} onChange={(v) => onChange({ respect_advisory_locks: v })} />
      </Section>

      <Section title="Change detection" hint="The NTFS change journal rescans only folders that changed since the last backup.">
        <Field label="NTFS change journal (USN)">
          <select data-testid="opt-usn" className={inputCls} value={o.usn_policy} onChange={(e) => onChange({ usn_policy: e.target.value as UsnPolicy })}>
            <option value="off">Off</option>
            <option value="auto">Auto (silently fall back)</option>
            <option value="on">On (warn on fallback)</option>
            <option value="required">Required (fail without it)</option>
          </select>
        </Field>
        <Field label="Changed paths (one per line; only these are rescanned)" hint="Leave empty for a normal scan. Typically supplied by a script or the CLI.">
          <textarea className={`${inputCls} font-mono h-16`} value={(o.changed_paths ?? []).join("\n")} onChange={(e) => onChange({ changed_paths: e.target.value.trim() ? lines(e.target.value) : null })} />
        </Field>
        <Field label="Deleted paths (one per line)">
          <textarea className={`${inputCls} font-mono h-12`} value={o.deleted_paths.join("\n")} onChange={(e) => onChange({ deleted_paths: lines(e.target.value) })} />
        </Field>
        <Check label="Destination ignores name case" checked={o.remote_case_insensitive} onChange={(v) => onChange({ remote_case_insensitive: v })} />
      </Section>

      <Section title="Control files" hint="Small files stored with every version (e.g. a manifest), extractable from Maintenance.">
        <textarea data-testid="opt-control-files" className={`${inputCls} font-mono h-12`} value={o.control_files.join("\n")} onChange={(e) => onChange({ control_files: lines(e.target.value) })} />
      </Section>

      <Section title="Performance">
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <Field label="Hashing threads (0 = auto)">
            <input className={inputCls} type="number" min={0} max={64} value={o.hash_threads} onChange={(e) => onChange({ hash_threads: num(e.target.value, 0, 64) })} />
          </Field>
          <Field label="Parallel uploads">
            <input className={inputCls} type="number" min={1} max={32} value={o.upload_concurrency} onChange={(e) => onChange({ upload_concurrency: num(e.target.value, 1, 32) })} />
          </Field>
          <Field label="CPU intensity (1–10)">
            <input className={inputCls} type="number" min={1} max={10} value={o.cpu_intensity} onChange={(e) => onChange({ cpu_intensity: num(e.target.value, 1, 10) })} />
          </Field>
          <Field label="Read buffer (KB, 0 = default)">
            <input className={inputCls} type="number" min={0} value={Math.round(o.read_buffer_size / 1024)} onChange={(e) => onChange({ read_buffer_size: num(e.target.value) * 1024 })} />
          </Field>
          <Field label="Temporary folder">
            <input className={`${inputCls} font-mono`} value={o.temp_dir ?? ""} placeholder="system default" onChange={(e) => onChange({ temp_dir: e.target.value || null })} />
          </Field>
        </div>
        <Check label="Low-priority (background) disk and CPU use" checked={o.background_io} onChange={(v) => onChange({ background_io: v })} />
      </Section>

      <Section title="Capacity and cadence">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Quota (GB, empty = use destination free space)">
            <input className={inputCls} type="number" min={0} value={o.quota_size == null ? "" : Math.round(o.quota_size / 1073741824)} onChange={(e) => onChange({ quota_size: e.target.value.trim() === "" ? null : num(e.target.value) * 1073741824 })} />
          </Field>
          <Field label="Warn below free (%)">
            <input className={inputCls} type="number" min={0} max={100} value={o.quota_warning_percent} onChange={(e) => onChange({ quota_warning_percent: num(e.target.value, 0, 100) })} />
          </Field>
          <Field label="Compact at most every (days)">
            <input className={inputCls} type="number" min={0} value={o.auto_compact_interval_days ?? ""} placeholder="after every backup" onChange={(e) => onChange({ auto_compact_interval_days: optNum(e.target.value) })} />
          </Field>
          <Field label="Vacuum database every (days)">
            <input className={inputCls} type="number" min={0} value={o.auto_vacuum_interval_days ?? ""} placeholder="only after compaction" onChange={(e) => onChange({ auto_vacuum_interval_days: optNum(e.target.value) })} />
          </Field>
        </div>
        <Check label="Disable capacity warnings" checked={o.quota_disable} onChange={(v) => onChange({ quota_disable: v })} />
      </Section>

      <Section title="Versions and sharing" hint="Stored with every new version; the identity decides which computer a version belongs to in a shared repository.">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Note for new versions">
            <input data-testid="opt-default-note" className={inputCls} maxLength={2000} value={o.default_note ?? ""} onChange={(e) => onChange({ default_note: e.target.value || null })} />
          </Field>
          <Field label="Tags for new versions" hint="key=value or key, separated by commas">
            <input
              data-testid="opt-default-tags"
              className={`${inputCls} font-mono`}
              defaultValue={Object.entries(o.default_tags ?? {}).map(([k, v]) => tagText(k, v)).join(", ")}
              onBlur={(e) => onChange({ default_tags: parseTags(e.target.value) })}
            />
          </Field>
          <Field label="Computer name to record (empty = this computer)" hint="Set it to the old computer's name to continue its history after moving to a new one.">
            <input data-testid="opt-identity-host" className={inputCls} value={o.identity?.hostname ?? ""} onChange={(e) => onChange({ identity: { ...o.identity, hostname: e.target.value.trim() } })} />
          </Field>
          <Field label="User name to record (empty = current user)">
            <input data-testid="opt-identity-user" className={inputCls} value={o.identity?.username ?? ""} onChange={(e) => onChange({ identity: { ...o.identity, username: e.target.value.trim() } })} />
          </Field>
        </div>
        <Check
          label="Share a new repository with other computers and jobs"
          checked={o.shared_repository}
          onChange={(v) => onChange({ shared_repository: v })}
          hint={locked ? "For an existing backup use Maintenance → Sharing." : "Format 2 only. Each job keeps its own versions; identical data is stored once."}
        />
        {o.shared_repository && (
          <Field label="Keep new volumes at least (hours) before compaction may remove them" hint="Protects a computer that is still uploading when another one compacts.">
            <input data-testid="opt-gc-delay" className={inputCls} type="number" min={0} value={o.shared_gc_delay_hours} onChange={(e) => onChange({ shared_gc_delay_hours: num(e.target.value, 0, 24 * 30) })} />
          </Field>
        )}
      </Section>

      <Section title="Repository safety" hint="Locks keep two programs from changing one backup at once; checkpoints keep long backups restorable if they stop half way.">
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          <Field label="Lock lease (minutes)" hint="Renewed every third; an expired lock is taken over.">
            <input data-testid="opt-lock-ttl" className={inputCls} type="number" min={1} value={Math.round((o.lock_ttl_secs || 1800) / 60)} onChange={(e) => onChange({ lock_ttl_secs: num(e.target.value, 1, 24 * 60) * 60 })} />
          </Field>
          <Field label="Clock difference warning (minutes, 0 = off)">
            <input data-testid="opt-skew-warn" className={inputCls} type="number" min={0} value={Math.round(o.clock_skew_warn_secs / 60)} onChange={(e) => onChange({ clock_skew_warn_secs: num(e.target.value) * 60 })} />
          </Field>
          <Field label="Clock difference limit (minutes, 0 = off)" hint="Refuse to write when the destination's clock is this far off.">
            <input data-testid="opt-skew-abort" className={inputCls} type="number" min={0} value={Math.round(o.clock_skew_abort_secs / 60)} onChange={(e) => onChange({ clock_skew_abort_secs: num(e.target.value) * 60 })} />
          </Field>
          <Field label="Checkpoint every (minutes, 0 = off)" hint="Uploads a partial version of the files finished so far.">
            <input data-testid="opt-checkpoint-secs" className={inputCls} type="number" min={0} value={Math.round(o.checkpoint_interval_secs / 60)} onChange={(e) => onChange({ checkpoint_interval_secs: num(e.target.value) * 60 })} />
          </Field>
          <Field label="Checkpoint every (MB uploaded, 0 = off)">
            <input data-testid="opt-checkpoint-bytes" className={inputCls} type="number" min={0} value={Math.round(o.checkpoint_interval_bytes / 1048576)} onChange={(e) => onChange({ checkpoint_interval_bytes: num(e.target.value) * 1048576 })} />
          </Field>
        </div>
      </Section>
    </>
  );
}
