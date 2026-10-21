/**
 * Backup job editor: a stepped setup wizard that also serves as the editor
 * for existing jobs. Every field maps 1:1 onto the Rust `BackupJob`.
 */
import { useEffect, useMemo, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import {
  errorText,
  useBackupStore,
  type BackupDestination,
  type BackupJob,
  type FilterRule,
  type PassphraseStrength,
  type RetentionPolicy,
  type SourcePreview,
} from "@/stores/backup-store";
import { inputCls, rulesToText, textToRules } from "@/lib/backup-helpers";
import { pickFolder } from "@/lib/folder-picker";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";

interface ConnectionProfile {
  id: string;
  name: string;
  protocol: string;
  host: string;
  remote_path: string;
}

const STEPS = ["General", "Sources", "Destination", "Schedule", "Retention", "Options", "Hooks"] as const;
type Step = (typeof STEPS)[number];
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];



export function BackupJobEditor({ initial, onDone }: { initial: BackupJob; onDone: (saved: BackupJob | null) => void }) {
  const { saveJob, capabilities, loadCapabilities } = useBackupStore();
  const [job, setJob] = useState<BackupJob>(initial);
  const [step, setStep] = useState<Step>("General");
  const [passphrase, setPassphrase] = useState("");
  const [strength, setStrength] = useState<PassphraseStrength | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [filterText, setFilterText] = useState<string | null>(null);
  const [preview, setPreview] = useState<SourcePreview | null>(null);
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const isNew = job.id === "";

  useEffect(() => {
    void loadCapabilities();
    tauriInvoke<ConnectionProfile[]>("list_connections", undefined, []).then(setConnections).catch(() => setConnections([]));
  }, [loadCapabilities]);

  useEffect(() => {
    if (!passphrase) {
      setStrength(null);
      return;
    }
    const t = setTimeout(() => {
      tauriInvoke<PassphraseStrength>("backup_passphrase_strength", { passphrase }).then(setStrength).catch(() => setStrength(null));
    }, 250);
    return () => clearTimeout(t);
  }, [passphrase]);

  const patch = (p: Partial<BackupJob>) => setJob((j) => ({ ...j, ...p }));
  const patchOpts = (p: Partial<BackupJob["options"]>) => setJob((j) => ({ ...j, options: { ...j.options, ...p } }));
  const patchFilter = (p: Partial<BackupJob["filter"]>) => setJob((j) => ({ ...j, filter: { ...j.filter, ...p } }));
  const backupProtocols = useMemo(() => new Set(capabilities?.connection_protocols ?? []), [capabilities]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      const toSave = filterText !== null ? { ...job, filter: { ...job.filter, rules: textToRules(filterText) } } : job;
      // Empty passphrase = keep the stored one (the backend rejects a new encrypted job without one).
      const saved = await saveJob(toSave, passphrase || null);
      onDone(saved);
    });

  const dest = job.destination;
  const setDest = (d: BackupDestination) => patch({ destination: d });

  const rules = filterText !== null ? textToRules(filterText) : job.filter.rules;
  const setRules = (r: FilterRule[]) => patchFilter({ rules: r });

  return (
    <div className="flex flex-col h-full" data-testid="backup-job-editor">
      <div className="flex items-center gap-1 p-2 border-b border-zinc-200 dark:border-zinc-700 flex-wrap">
        {STEPS.map((s) => (
          <button
            key={s}
            className={`px-2 py-1 text-xs rounded ${s === step ? "bg-blue-500 text-white" : "bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300"}`}
            onClick={() => setStep(s)}
          >
            {s}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
        {info && <Banner tone="success" onClose={() => setInfo(null)}>{info}</Banner>}

        {step === "General" && (
          <Section title="General">
            <Field label="Name">
              <input className={inputCls} value={job.name} onChange={(e) => patch({ name: e.target.value })} data-testid="backup-name" />
            </Field>
            <Field label="Description">
              <textarea className={inputCls} rows={2} value={job.description} onChange={(e) => patch({ description: e.target.value })} />
            </Field>
            <Check
              label="Encrypt backup (AES-256-GCM, key derived with Argon2id)"
              checked={job.encrypted}
              onChange={(v) => patch({ encrypted: v })}
              hint="Without the passphrase the backup cannot be restored. Store it somewhere safe."
            />
            {job.encrypted && (
              <Field label={isNew ? "Passphrase" : "New passphrase (leave empty to keep the current one)"}>
                <div className="flex gap-1">
                  <input className={inputCls} type="text" autoComplete="off" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} data-testid="backup-passphrase" />
                  <Btn onClick={() => tauriInvoke<string>("backup_generate_passphrase", { length: 32 }).then(setPassphrase).catch((e) => setError(errorText(e)))}>Generate</Btn>
                </div>
                {strength && (
                  <span className="block text-[11px]">
                    Strength: <b>{strength.label}</b>
                    {strength.feedback.length > 0 && ` — ${strength.feedback.join(" ")}`}
                  </span>
                )}
                {!isNew && passphrase && (
                  <span className="block text-[11px] text-amber-600">Changing the passphrase of an existing backup makes older volumes unreadable with the new one. Use it only for a fresh destination.</span>
                )}
              </Field>
            )}
          </Section>
        )}

        {step === "Sources" && (
          <>
            <Section title="Source folders and files" hint="One path per line.">
              <textarea
                className={`${inputCls} font-mono`}
                rows={5}
                value={job.sources.join("\n")}
                onChange={(e) => patch({ sources: e.target.value.split("\n") })}
                data-testid="backup-sources"
              />
              <div className="flex gap-1">
                <Btn
                  onClick={async () => {
                    const p = await pickFolder("Add source folder");
                    if (p) patch({ sources: [...job.sources.filter((s) => s.trim()), p] });
                  }}
                >
                  Add folder…
                </Btn>
                <Btn
                  disabled={busy}
                  onClick={() =>
                    run(async () => {
                      const j = filterText !== null ? { ...job, filter: { ...job.filter, rules: textToRules(filterText) } } : job;
                      setPreview(await tauriInvoke<SourcePreview>("backup_preview_sources", { job: j }));
                    })
                  }
                >
                  Preview selection
                </Btn>
              </div>
              {preview && (
                <div className="text-xs text-zinc-600 dark:text-zinc-400">
                  {preview.files} files in {preview.folders} folders, {formatBytes(preview.total_size)}
                  {preview.warnings.map((w) => (
                    <div key={w} className="text-amber-600">{w}</div>
                  ))}
                  <ul className="mt-1 max-h-24 overflow-y-auto font-mono text-[11px]">
                    {preview.sample.map((s) => (
                      <li key={s} className="truncate">{s}</li>
                    ))}
                  </ul>
                </div>
              )}
            </Section>

            <Section title="Filters" hint="Evaluated top to bottom; the first match wins. Excluding a folder skips everything inside it.">
              <div className="flex gap-1">
                <Btn onClick={() => setFilterText(filterText === null ? rulesToText(job.filter.rules) : null)}>
                  {filterText === null ? "Edit as text" : "Edit as list"}
                </Btn>
                {filterText !== null && (
                  <Btn
                    onClick={() => {
                      setRules(textToRules(filterText));
                      setFilterText(null);
                    }}
                  >
                    Apply text
                  </Btn>
                )}
              </div>
              {filterText !== null ? (
                <textarea
                  className={`${inputCls} font-mono`}
                  rows={6}
                  value={filterText}
                  onChange={(e) => setFilterText(e.target.value)}
                  placeholder={"-*.tmp\n-{cache_files}\n-[\\.bak\\d+$]\n+/home/me/Docs/keep.log"}
                />
              ) : (
                <div className="space-y-1">
                  {rules.map((r, i) => (
                    <div key={i} className="flex gap-1 items-center">
                      <select className={`${inputCls} w-24`} value={r.action} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, action: e.target.value as FilterRule["action"] } : x)))}>
                        <option value="exclude">Exclude</option>
                        <option value="include">Include</option>
                      </select>
                      <select className={`${inputCls} w-28`} value={r.kind} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, kind: e.target.value as FilterRule["kind"] } : x)))}>
                        <option value="wildcard">Wildcard</option>
                        <option value="exact">Exact path</option>
                        <option value="regex">Regex</option>
                        <option value="group">Group</option>
                      </select>
                      {r.kind === "group" ? (
                        <select className={inputCls} value={r.pattern} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, pattern: e.target.value } : x)))}>
                          <option value="">Choose group…</option>
                          {capabilities?.filter_groups.map((g) => (
                            <option key={g.name} value={g.name} title={g.patterns.join(" ")}>
                              {g.name} — {g.description}
                            </option>
                          ))}
                        </select>
                      ) : (
                        <input className={`${inputCls} font-mono`} value={r.pattern} onChange={(e) => setRules(rules.map((x, j) => (j === i ? { ...x, pattern: e.target.value } : x)))} />
                      )}
                      <Btn disabled={i === 0} onClick={() => setRules(rules.map((x, j) => (j === i - 1 ? rules[i] : j === i ? rules[i - 1] : x)))} title="Move up">↑</Btn>
                      <Btn onClick={() => setRules(rules.filter((_, j) => j !== i))} title="Remove">✕</Btn>
                    </div>
                  ))}
                  <Btn onClick={() => setRules([...rules, { action: "exclude", kind: "wildcard", pattern: "" }])}>Add rule</Btn>
                </div>
              )}
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 pt-2">
                <Check label="Exclude hidden files" checked={job.filter.exclude_hidden} onChange={(v) => patchFilter({ exclude_hidden: v })} />
                <Check label="Exclude system files (Windows)" checked={job.filter.exclude_system} onChange={(v) => patchFilter({ exclude_system: v })} />
                <Check label="Exclude temporary files (Windows)" checked={job.filter.exclude_temporary} onChange={(v) => patchFilter({ exclude_temporary: v })} />
                <Check label="Skip cloud placeholders (OneDrive/iCloud)" checked={job.filter.exclude_cloud_placeholders} onChange={(v) => patchFilter({ exclude_cloud_placeholders: v })} />
                <Check label="Skip empty folders" checked={job.filter.exclude_empty_folders} onChange={(v) => patchFilter({ exclude_empty_folders: v })} />
              </div>
              <Field label="Skip files larger than (MB, empty = no limit)">
                <input
                  className={inputCls}
                  type="number"
                  min={0}
                  value={job.filter.max_file_size === null ? "" : Math.round(job.filter.max_file_size / 1048576)}
                  onChange={(e) => patchFilter({ max_file_size: e.target.value === "" ? null : Number(e.target.value) * 1048576 })}
                />
              </Field>
              <Field label="Exclude folders containing (comma separated)" hint="e.g. .nobackup, CACHEDIR.TAG (only counted with its standard signature)">
                <input
                  className={inputCls}
                  value={job.filter.exclusion_markers.join(", ")}
                  onChange={(e) => patchFilter({ exclusion_markers: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) })}
                />
              </Field>
            </Section>
          </>
        )}

        {step === "Destination" && (
          <Section title="Destination">
            <div className="flex gap-2 text-xs">
              <label className="flex items-center gap-1">
                <input type="radio" checked={dest.type === "local"} onChange={() => setDest({ type: "local", path: "", alternate_paths: [], marker_file: null })} />
                Folder / drive / mounted share
              </label>
              <label className="flex items-center gap-1">
                <input type="radio" checked={dest.type === "connection"} onChange={() => setDest({ type: "connection", connection_id: "", remote_path: "" })} />
                Saved connection
              </label>
            </div>
            {dest.type === "local" ? (
              <>
                <Field label="Folder">
                  <PathInput value={dest.path} onChange={(v) => setDest({ ...dest, path: v })} placeholder="/Volumes/Backup/ufop or \\\\nas\\backups" testId="backup-dest-path" />
                </Field>
                <Field label="Alternate paths (one per line)" hint="Tried in order when the first is missing. Use * for a drive letter or mount, e.g. *:\\Backups or /media/*/Backups.">
                  <textarea className={`${inputCls} font-mono`} rows={2} value={dest.alternate_paths.join("\n")} onChange={(e) => setDest({ ...dest, alternate_paths: e.target.value.split("\n").filter((s) => s.trim()) })} />
                </Field>
                <Field label="Marker file (optional)" hint="Only use a destination that contains this file — protects against writing to the wrong removable drive.">
                  <input className={inputCls} value={dest.marker_file ?? ""} onChange={(e) => setDest({ ...dest, marker_file: e.target.value || null })} />
                </Field>
              </>
            ) : (
              <>
                <Field label="Connection" hint={`Supported: ${[...backupProtocols].join(", ") || "sftp, ftp, ftps, webdav, s3"}. Manage connections in the Connections panel.`}>
                  <select className={inputCls} value={dest.connection_id} onChange={(e) => setDest({ ...dest, connection_id: e.target.value })}>
                    <option value="">Choose…</option>
                    {connections.map((c) => (
                      <option key={c.id} value={c.id} disabled={backupProtocols.size > 0 && !backupProtocols.has(c.protocol)}>
                        {c.name} ({c.protocol}://{c.host})
                        {backupProtocols.size > 0 && !backupProtocols.has(c.protocol) ? " — not supported for backups yet" : ""}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="Remote folder" hint="For S3 use /bucket/prefix.">
                  <input className={inputCls} value={dest.remote_path} onChange={(e) => setDest({ ...dest, remote_path: e.target.value })} />
                </Field>
              </>
            )}
            <div className="flex gap-1 flex-wrap">
              <Btn
                disabled={busy}
                onClick={() => run(async () => setInfo(`Connected — ${await tauriInvoke<number>("backup_test_destination", { destination: dest, create: false })} file(s) present.`))}
              >
                Test connection
              </Btn>
              <Btn
                disabled={busy}
                onClick={() => run(async () => setInfo(`Folder ready — ${await tauriInvoke<number>("backup_test_destination", { destination: dest, create: true })} file(s) present.`))}
              >
                Test &amp; create folder
              </Btn>
              <Btn
                disabled={busy}
                title="Uploads, lists, downloads and hash-checks random test files, then deletes them"
                onClick={() => run(async () => setInfo((await tauriInvoke<string[]>("backup_test_destination_compat", { destination: dest, files: 3, maxSize: 1048576 })).join("\n")))}
              >
                Compatibility test
              </Btn>
            </div>
            <Field label="File-name prefix" hint="Lets several backups share one folder.">
              <input className={inputCls} value={job.options.prefix} onChange={(e) => patchOpts({ prefix: e.target.value.replace(/[^A-Za-z0-9_.]/g, "") })} />
            </Field>
          </Section>
        )}

        {step === "Schedule" && (
          <Section title="Schedule" hint="Times are local; runs missed while the app was closed start as soon as it is running again.">
            <Check label="Run automatically" checked={job.schedule.enabled} onChange={(v) => patch({ schedule: { ...job.schedule, enabled: v } })} />
            <div className="grid grid-cols-2 gap-2">
              <Field label="Start time">
                <input className={inputCls} type="time" value={job.schedule.start_time} onChange={(e) => patch({ schedule: { ...job.schedule, start_time: e.target.value } })} />
              </Field>
              <Field label="Repeat">
                <select className={inputCls} value={job.schedule.interval_minutes} onChange={(e) => patch({ schedule: { ...job.schedule, interval_minutes: Number(e.target.value) } })}>
                  {[
                    [15, "Every 15 minutes"],
                    [60, "Hourly"],
                    [360, "Every 6 hours"],
                    [720, "Every 12 hours"],
                    [1440, "Daily"],
                    [10080, "Weekly"],
                    [43200, "Every 30 days"],
                  ].map(([v, l]) => (
                    <option key={v} value={v}>{l}</option>
                  ))}
                </select>
              </Field>
            </div>
            <div className="flex gap-2 flex-wrap text-xs">
              {WEEKDAYS.map((d, i) => (
                <label key={d} className="flex items-center gap-1">
                  <input
                    type="checkbox"
                    checked={job.schedule.weekdays.length === 0 || job.schedule.weekdays.includes(i)}
                    onChange={(e) => {
                      const all = job.schedule.weekdays.length === 0 ? [0, 1, 2, 3, 4, 5, 6] : job.schedule.weekdays;
                      const next = e.target.checked ? [...new Set([...all, i])].sort() : all.filter((x) => x !== i);
                      patch({ schedule: { ...job.schedule, weekdays: next.length === 7 ? [] : next } });
                    }}
                  />
                  {d}
                </label>
              ))}
            </div>
          </Section>
        )}

        {step === "Retention" && (
          <Section title="Version retention" hint="Applied after each successful backup. The newest version is never deleted.">
            <select
              className={inputCls}
              value={job.retention.type}
              onChange={(e) => {
                const t = e.target.value as RetentionPolicy["type"];
                const r: RetentionPolicy =
                  t === "keep_days" ? { type: t, days: 30 } : t === "keep_count" ? { type: t, count: 10 } : t === "custom" ? { type: t, rules: "1W:1D,4W:1W,12M:1M" } : { type: t };
                patch({ retention: r });
              }}
              data-testid="backup-retention"
            >
              <option value="keep_all">Keep all versions</option>
              <option value="keep_days">Delete versions older than…</option>
              <option value="keep_count">Keep a number of versions</option>
              <option value="smart">Smart (one per day for a week, per week for a month, per month for a year)</option>
              <option value="custom">Custom thinning rules</option>
            </select>
            {job.retention.type === "keep_days" && (
              <Field label="Days">
                <input className={inputCls} type="number" min={1} value={job.retention.days} onChange={(e) => patch({ retention: { type: "keep_days", days: Math.max(1, Number(e.target.value)) } })} />
              </Field>
            )}
            {job.retention.type === "keep_count" && (
              <Field label="Versions">
                <input className={inputCls} type="number" min={1} value={job.retention.count} onChange={(e) => patch({ retention: { type: "keep_count", count: Math.max(1, Number(e.target.value)) } })} />
              </Field>
            )}
            {job.retention.type === "custom" && (
              <Field label="Rules (timeframe:interval, …)" hint="Units s, m, h, D, W, M, Y; interval U keeps all. Example: 7D:U,4W:1D,12M:1W">
                <input className={`${inputCls} font-mono`} value={job.retention.rules} onChange={(e) => patch({ retention: { type: "custom", rules: e.target.value } })} />
              </Field>
            )}
          </Section>
        )}

        {step === "Options" && (
          <>
            <Section title="Storage">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                <Field label="Block size (KB)" hint="Fixed after the first backup.">
                  <input className={inputCls} type="number" min={1} disabled={!isNew && job.stats.version_count > 0} value={Math.round(job.options.block_size / 1024)} onChange={(e) => patchOpts({ block_size: Math.max(1, Number(e.target.value)) * 1024 })} />
                </Field>
                <Field label="Remote volume size (MB)">
                  <input className={inputCls} type="number" min={1} value={Math.round(job.options.volume_size / 1048576)} onChange={(e) => patchOpts({ volume_size: Math.max(1, Number(e.target.value)) * 1048576 })} />
                </Field>
                <Field label="Compression level (0 = store)">
                  <input className={inputCls} type="number" min={0} max={9} value={job.options.compression_level} onChange={(e) => patchOpts({ compression_level: Math.min(9, Math.max(0, Number(e.target.value))) })} />
                </Field>
                <Field label="Hash algorithm">
                  <select className={inputCls} disabled={!isNew && job.stats.version_count > 0} value={job.options.block_hash} onChange={(e) => patchOpts({ block_hash: e.target.value as "sha256", file_hash: e.target.value as "sha256" })}>
                    <option value="sha256">SHA-256</option>
                    <option value="sha512">SHA-512</option>
                  </select>
                </Field>
              </div>
              <Field label="Store without compression (extensions)">
                <input className={inputCls} value={job.options.no_compress_extensions.join(", ")} onChange={(e) => patchOpts({ no_compress_extensions: e.target.value.split(",").map((s) => s.trim().replace(/^\./, "").toLowerCase()).filter(Boolean) })} />
              </Field>
            </Section>
            <Section title="Source handling">
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
                <Field label="Symbolic links">
                  <select className={inputCls} value={job.options.symlink_policy} onChange={(e) => patchOpts({ symlink_policy: e.target.value as "store" })}>
                    <option value="store">Store the link</option>
                    <option value="follow">Follow (back up target)</option>
                    <option value="ignore">Ignore</option>
                  </select>
                </Field>
                <Field label="Hard links (Linux/macOS)">
                  <select className={inputCls} value={job.options.hardlink_policy} onChange={(e) => patchOpts({ hardlink_policy: e.target.value as "all" })}>
                    <option value="all">Every path</option>
                    <option value="first">First path only</option>
                    <option value="none">Skip multiply-linked</option>
                  </select>
                </Field>
                <Field label="Missing source">
                  <select className={inputCls} value={job.options.missing_source} onChange={(e) => patchOpts({ missing_source: e.target.value as "fail" })}>
                    <option value="fail">Fail the backup</option>
                    <option value="warn">Warn and continue</option>
                  </select>
                </Field>
              </div>
              <Check label="Allow a version with no files" checked={job.options.allow_empty_source} onChange={(v) => patchOpts({ allow_empty_source: v })} />
              <Check label="Always re-read files (ignore timestamps)" checked={job.options.disable_timestamp_check} onChange={(v) => patchOpts({ disable_timestamp_check: v })} />
              <Check label="Trust timestamps only (ignore size changes)" checked={job.options.timestamps_only} onChange={(v) => patchOpts({ timestamps_only: v })} />
              <Check label="Do not record timestamps and permissions" checked={job.options.skip_metadata} onChange={(v) => patchOpts({ skip_metadata: v })} />
              <Check label="Create a version even when nothing changed" checked={job.options.upload_unchanged_backups} onChange={(v) => patchOpts({ upload_unchanged_backups: v })} />
            </Section>
            <Section title="Integrity and maintenance">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                <Field label="Volumes to verify after each backup">
                  <input className={inputCls} type="number" min={0} value={job.options.verify_samples} onChange={(e) => patchOpts({ verify_samples: Math.max(0, Number(e.target.value)) })} />
                </Field>
                <Field label="Compact when wasted space ≥ (%)">
                  <input className={inputCls} type="number" min={1} max={100} value={job.options.compact_threshold_percent} onChange={(e) => patchOpts({ compact_threshold_percent: Math.min(100, Math.max(1, Number(e.target.value))) })} />
                </Field>
              </div>
              <Check label="Compact automatically" checked={job.options.auto_compact} onChange={(v) => patchOpts({ auto_compact: v })} />
              <Check label="Upload an unencrypted verification manifest (sizes + SHA-256)" checked={job.options.upload_verification_file} onChange={(v) => patchOpts({ upload_verification_file: v })} />
              <Check label="Store this job's configuration in the backup (without secrets)" checked={job.store_config_remotely} onChange={(v) => patch({ store_config_remotely: v })} hint="Lets you recreate the job from the destination after losing this computer." />
              <Field label="Soft-delete prefix (rename instead of delete)">
                <input className={inputCls} value={job.soft_delete_prefix ?? ""} placeholder="e.g. deleted-" onChange={(e) => patch({ soft_delete_prefix: e.target.value || null })} />
              </Field>
            </Section>
            <Section title="Network">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                <Field label="Upload limit (KB/s, 0 = unlimited)">
                  <input className={inputCls} type="number" min={0} value={Math.round(job.throttle.upload_bps / 1024)} onChange={(e) => patch({ throttle: { ...job.throttle, upload_bps: Math.max(0, Number(e.target.value)) * 1024 } })} />
                </Field>
                <Field label="Download limit (KB/s, 0 = unlimited)">
                  <input className={inputCls} type="number" min={0} value={Math.round(job.throttle.download_bps / 1024)} onChange={(e) => patch({ throttle: { ...job.throttle, download_bps: Math.max(0, Number(e.target.value)) * 1024 } })} />
                </Field>
                <Field label="Retries">
                  <input className={inputCls} type="number" min={0} max={20} value={job.retry.retries} onChange={(e) => patch({ retry: { ...job.retry, retries: Math.max(0, Number(e.target.value)) } })} />
                </Field>
                <Field label="Retry delay (s)">
                  <input className={inputCls} type="number" min={0} value={Math.round(job.retry.delay_ms / 1000)} onChange={(e) => patch({ retry: { ...job.retry, delay_ms: Math.max(0, Number(e.target.value)) * 1000 } })} />
                </Field>
              </div>
              <Check label="Exponential backoff" checked={job.retry.exponential} onChange={(v) => patch({ retry: { ...job.retry, exponential: v } })} />
            </Section>
          </>
        )}

        {step === "Hooks" && (
          <>
            <Section title="Scripts" hint="Receive UFOP_* environment variables. Lines printed as LOG:INFO / LOG:WARN / LOG:ERROR appear in the run log.">
              <Field label="Before backup">
                <input className={`${inputCls} font-mono`} value={job.scripts.run_before ?? ""} onChange={(e) => patch({ scripts: { ...job.scripts, run_before: e.target.value || null } })} />
              </Field>
              <Check label="Required — abort the backup if this script fails" checked={job.scripts.run_before_required} onChange={(v) => patch({ scripts: { ...job.scripts, run_before_required: v } })} />
              <Field label="After backup" hint="UFOP_PARSED_RESULT holds success/warning/error; UFOP_RESULTFILE points at the result.">
                <input className={`${inputCls} font-mono`} value={job.scripts.run_after ?? ""} onChange={(e) => patch({ scripts: { ...job.scripts, run_after: e.target.value || null } })} />
              </Field>
              <div className="grid grid-cols-2 gap-2">
                <Field label="Timeout (s)">
                  <input className={inputCls} type="number" min={1} value={job.scripts.timeout_secs} onChange={(e) => patch({ scripts: { ...job.scripts, timeout_secs: Math.max(1, Number(e.target.value)) } })} />
                </Field>
                <Field label="Result file format">
                  <select className={inputCls} value={job.scripts.result_format} onChange={(e) => patch({ scripts: { ...job.scripts, result_format: e.target.value } })}>
                    <option value="json">JSON</option>
                    <option value="text">Text</option>
                  </select>
                </Field>
              </div>
            </Section>
            <Section title="HTTP reports" hint="Sent to every URL when a run ends with a selected result.">
              <Field label="URLs (one per line)">
                <textarea className={`${inputCls} font-mono`} rows={2} value={job.reports.http_urls.join("\n")} onChange={(e) => patch({ reports: { ...job.reports, http_urls: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean) } })} />
              </Field>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
                <Field label="Method">
                  <select className={inputCls} value={job.reports.http_method} onChange={(e) => patch({ reports: { ...job.reports, http_method: e.target.value } })}>
                    <option>POST</option>
                    <option>PUT</option>
                    <option>GET</option>
                  </select>
                </Field>
                <Field label="Format">
                  <select className={inputCls} value={job.reports.format} onChange={(e) => patch({ reports: { ...job.reports, format: e.target.value as "json" } })}>
                    <option value="json">JSON</option>
                    <option value="form">Form</option>
                  </select>
                </Field>
                <Field label="Retries">
                  <input className={inputCls} type="number" min={0} max={10} value={job.reports.retries} onChange={(e) => patch({ reports: { ...job.reports, retries: Math.max(0, Number(e.target.value)) } })} />
                </Field>
              </div>
              <div className="flex gap-3 text-xs">
                {["success", "warning", "error"].map((r) => (
                  <Check
                    key={r}
                    label={`On ${r}`}
                    checked={job.reports.send_on.includes(r)}
                    onChange={(v) => patch({ reports: { ...job.reports, send_on: v ? [...job.reports.send_on, r] : job.reports.send_on.filter((x) => x !== r) } })}
                  />
                ))}
              </div>
              <Field label="Message template" hint="{{name}} {{operation}} {{result}} {{machine}} {{duration}} {{files}} {{bytes}} {{errors}}">
                <input className={inputCls} value={job.reports.message_template} onChange={(e) => patch({ reports: { ...job.reports, message_template: e.target.value } })} />
              </Field>
              <div className="grid grid-cols-2 gap-2">
                <Field label="Machine name override">
                  <input className={inputCls} value={job.reports.machine_name ?? ""} onChange={(e) => patch({ reports: { ...job.reports, machine_name: e.target.value || null } })} />
                </Field>
                <Field label="Backup id override">
                  <input className={inputCls} value={job.reports.backup_id ?? ""} onChange={(e) => patch({ reports: { ...job.reports, backup_id: e.target.value || null } })} />
                </Field>
              </div>
              <Check label="Accept invalid TLS certificates on report URLs" checked={job.reports.accept_invalid_certs} onChange={(v) => patch({ reports: { ...job.reports, accept_invalid_certs: v } })} />
            </Section>
          </>
        )}
      </div>

      <div className="flex justify-between items-center p-2 border-t border-zinc-200 dark:border-zinc-700">
        <Btn onClick={() => onDone(null)}>Cancel</Btn>
        <div className="flex gap-1">
          <Btn disabled={STEPS.indexOf(step) === 0} onClick={() => setStep(STEPS[STEPS.indexOf(step) - 1])}>Back</Btn>
          {STEPS.indexOf(step) < STEPS.length - 1 && <Btn onClick={() => setStep(STEPS[STEPS.indexOf(step) + 1])}>Next</Btn>}
          <Btn tone="success" disabled={busy} onClick={save} testId="backup-save">
            {isNew ? "Create backup" : "Save"}
          </Btn>
        </div>
      </div>
    </div>
  );
}
