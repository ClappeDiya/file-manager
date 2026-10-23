/**
 * Versions & restore: browse a job's versions, search across history,
 * compare versions, pick files and restore them (original or alternate
 * location, conflict policy, permissions, verification).
 *
 * `DirectRestore` restores from any destination with only its passphrase —
 * for a lost installation or another machine's backup.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import {
  DEFAULT_RESTORE_OPTIONS,
  errorText,
  useBackupStore,
  type BackupDestination,
  type BackupFileEntry,
  type BackupJob,
  type Fileset,
  type FoundFile,
  type RestoreOptions,
  type RestoreOverrides,
  type RestoreReport,
  type VersionComparison,
} from "@/stores/backup-store";
import { formatDate, inputCls } from "@/lib/backup-helpers";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";
import { AdvancedOnly } from "./advanced-only";
import { BackupVersionTable } from "./backup-version-list";
import type { SourceInfo } from "@/stores/backup-store";

function RestoreOptionsForm({ value, onChange, versionCount = 0 }: { value: RestoreOptions; onChange: (v: RestoreOptions) => void; versionCount?: number }) {
  const n = (v: string, max: number) => Math.min(max, Math.max(0, Math.floor(Number(v) || 0)));
  return (
    <div className="space-y-2">
      <div className="flex gap-3 text-xs">
        <label className="flex items-center gap-1">
          <input type="radio" checked={value.target === null} onChange={() => onChange({ ...value, target: null })} />
          Original location
        </label>
        <label className="flex items-center gap-1">
          <input type="radio" checked={value.target !== null} onChange={() => onChange({ ...value, target: "" })} />
          Another folder
        </label>
      </div>
      {value.target !== null && (
        <>
          <PathInput value={value.target} onChange={(t) => onChange({ ...value, target: t })} placeholder="/path/to/restore" testId="restore-target" />
          <Check label="Keep the full original folder structure" checked={value.preserve_full_paths} onChange={(v) => onChange({ ...value, preserve_full_paths: v })} />
        </>
      )}
      <Field label="If a file already exists">
        <select className={inputCls} value={value.conflict} onChange={(e) => onChange({ ...value, conflict: e.target.value as RestoreOptions["conflict"] })}>
          <option value="overwrite">Overwrite it</option>
          <option value="rename">Keep it; save the restored copy with a timestamp</option>
          <option value="skip">Skip it</option>
        </select>
      </Field>
      <Check label="Restore file permissions" checked={value.restore_permissions} onChange={(v) => onChange({ ...value, restore_permissions: v })} />
      <Check label="Recreate symbolic links" checked={value.restore_symlinks} onChange={(v) => onChange({ ...value, restore_symlinks: v })} />
      <Check label="Restore extended attributes and Windows attributes" checked={value.restore_metadata} onChange={(v) => onChange({ ...value, restore_metadata: v })} />
      {versionCount > 1 && value.target !== null && (
        <>
          <Check
            label={`Restore all ${versionCount} versions side by side (one folder per version)`}
            checked={value.versions.length > 0}
            onChange={(v) => onChange({ ...value, versions: v ? Array.from({ length: versionCount }, (_, i) => i) : [] })}
          />
          {value.versions.length > 0 && (
            <Check label="Only versions whose content changed" checked={value.unique_versions} onChange={(v) => onChange({ ...value, unique_versions: v })} />
          )}
        </>
      )}
      <Check label="Reuse matching data from files on this computer (download less)" checked={value.use_local_blocks} onChange={(v) => onChange({ ...value, use_local_blocks: v })} />
      <details className="text-xs">
        <summary className="cursor-pointer select-none">Performance and staging</summary>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 mt-2">
          <Field label="Parallel downloads (0 = auto)">
            <input className={inputCls} type="number" min={0} max={16} disabled={value.legacy} value={value.download_threads} onChange={(e) => onChange({ ...value, download_threads: n(e.target.value, 16) })} />
          </Field>
          <Field label="Parallel file writers (0 = auto)">
            <input className={inputCls} type="number" min={0} max={32} disabled={value.legacy} value={value.file_threads} onChange={(e) => onChange({ ...value, file_threads: n(e.target.value, 32) })} />
          </Field>
          <Field label="Staging folder">
            <input className={`${inputCls} font-mono`} value={value.temp_dir ?? ""} placeholder="system temp" onChange={(e) => onChange({ ...value, temp_dir: e.target.value || null })} />
          </Field>
        </div>
        <Check label="Legacy mode (sequential, single-threaded)" checked={value.legacy} onChange={(v) => onChange({ ...value, legacy: v })} />
        <Check label="Skip the free-space check" checked={value.skip_free_space_check} onChange={(v) => onChange({ ...value, skip_free_space_check: v })} />
      </details>
      <Check label="Skip hash verification (recovery only)" checked={value.skip_verify} onChange={(v) => onChange({ ...value, skip_verify: v })} />
      <Check label="Dry run (only list what would be written)" checked={value.dry_run} onChange={(v) => onChange({ ...value, dry_run: v })} />
    </div>
  );
}

function ReportView({ report }: { report: RestoreReport }) {
  return (
    <Banner tone={report.errors.length ? "warning" : "success"}>
      Restored {report.files_restored} file(s), {formatBytes(report.bytes_restored)}; {report.files_skipped_matching} already correct
      {report.files_skipped_conflict > 0 && `, ${report.files_skipped_conflict} skipped (conflict)`}
      {report.files_skipped_duplicate > 0 && `, ${report.files_skipped_duplicate} identical to a newer version`}
      {report.blocks_from_local > 0 && `; ${report.blocks_from_local} block(s) reused from local files`}.
      {report.planned.length > 0 && <div>Would write {report.planned.length} item(s).</div>}
      {report.errors.slice(0, 20).map((e) => (
        <div key={e} className="text-red-600">{e}</div>
      ))}
      {report.warnings.slice(0, 10).map((w) => (
        <div key={w}>{w}</div>
      ))}
    </Banner>
  );
}

/** Shared repositories (K4): pick whose versions to browse (Advanced). */
function SourcePicker({ job, source, onChange }: { job: BackupJob; source: string | null; onChange: (s: string | null) => void }) {
  const { listSources, execute } = useBackupStore();
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    listSources(job.id).then(setSources).catch(() => setSources([]));
  }, [job.id, listSources]);
  if (sources.length < 2) return null;
  return (
    <div className="flex gap-1 items-center text-xs" data-testid="backup-source-picker">
      <span className="text-zinc-500">Versions of</span>
      <select className={`${inputCls} w-auto`} value={source ?? ""} onChange={(e) => onChange(e.target.value || null)} aria-label="Source">
        <option value="">This job</option>
        {sources
          .filter((s) => !s.own)
          .map((s) => (
            <option key={s.id} value={s.id}>
              {s.label} · {s.versions} version(s)
            </option>
          ))}
        <option value="*">All sources</option>
      </select>
      <Btn
        disabled={busy}
        title="Read other computers' latest versions from the destination"
        onClick={async () => {
          setBusy(true);
          try {
            await execute(job.id, { type: "sync_sources" });
            setSources(await listSources(job.id));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Refreshing…" : "Refresh"}
      </Btn>
    </div>
  );
}

export function BackupRestoreView({ job }: { job: BackupJob }) {
  const { execute, listVersions } = useBackupStore();
  const [source, setSource] = useState<string | null>(null);
  const [versions, setVersions] = useState<Fileset[]>([]);
  const [version, setVersion] = useState(0);
  const [files, setFiles] = useState<BackupFileEntry[]>([]);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [opts, setOpts] = useState<RestoreOptions>(DEFAULT_RESTORE_OPTIONS);
  const [overrides, setOverrides] = useState<RestoreOverrides>({ download_bps: null, retries: null, network_timeout_secs: null });
  const [search, setSearch] = useState("");
  const [hits, setHits] = useState<FoundFile[] | null>(null);
  const [cmp, setCmp] = useState<VersionComparison | null>(null);
  const [report, setReport] = useState<RestoreReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Another source's versions (shared repositories); omitted for the job's own.
  const src = useMemo(() => (source ? { source } : {}), [source]);

  const loadVersions = useCallback(async () => {
    try {
      setVersions(await listVersions(job.id, source));
    } catch (e) {
      setError(errorText(e));
    }
  }, [job.id, source, listVersions]);

  useEffect(() => {
    void loadVersions();
  }, [loadVersions]);

  useEffect(() => {
    if (versions.length === 0) return;
    setSelected(new Set());
    tauriInvoke<BackupFileEntry[]>("backup_list_files", { id: job.id, version, prefix: null, ...src }, [])
      .then(setFiles)
      .catch((e) => setError(errorText(e)));
  }, [job.id, version, versions.length, src]);

  const shown = useMemo(() => {
    const f = filter.toLowerCase();
    return (f ? files.filter((x) => x.path.toLowerCase().includes(f)) : files).slice(0, 2000);
  }, [files, filter]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (p: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });

  const picker = (
    <AdvancedOnly>
      <SourcePicker
        job={job}
        source={source}
        onChange={(s) => {
          setSource(s);
          setVersion(0);
          setCmp(null);
        }}
      />
    </AdvancedOnly>
  );

  if (versions.length === 0) {
    return (
      <div className="p-4 text-sm text-zinc-500 space-y-2" data-testid="backup-no-versions">
        {picker}
        {error ? <Banner tone="error">{error}</Banner> : "This job has no versions yet. Run a backup first."}
      </div>
    );
  }
  const current = versions[Math.min(version, versions.length - 1)];

  return (
    <div className="p-3 space-y-3" data-testid="backup-restore-view">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <Section title="Versions">
        {picker}
        <select className={inputCls} value={version} onChange={(e) => setVersion(Number(e.target.value))} data-testid="backup-version-select">
          {versions.map((v, i) => (
            <option key={v.id} value={i}>
              {i === 0 ? "Latest — " : ""}
              {formatDate(v.timestamp)} · {v.file_count} items · {formatBytes(v.total_size)}
              {v.partial ? " · partial (interrupted)" : ""}
            </option>
          ))}
        </select>
        <BackupVersionTable job={job} versions={versions} selected={version} onSelect={setVersion} source={source} onChanged={loadVersions} />
        <div className="flex gap-1 flex-wrap">
          <Btn
            disabled={versions.length < 2 || busy}
            onClick={() => run(async () => setCmp(await tauriInvoke<VersionComparison>("backup_compare", { id: job.id, base: Math.min(version + 1, versions.length - 1), compare: version, ...src })))}
          >
            Compare with previous
          </Btn>
          <Btn
            tone="danger"
            disabled={busy || versions.length < 2 || source !== null}
            title={source !== null ? "Versions of other sources are not deleted from here" : "Delete this version (the last remaining version is protected)"}
            onClick={() =>
              confirm(
                current.pinned
                  ? `The version from ${formatDate(current.timestamp)} is set to Keep forever. Delete it anyway?`
                  : `Delete the version from ${formatDate(current.timestamp)}?`,
              ) &&
              run(async () => {
                await execute(job.id, { type: "delete_versions", versions: [version], allow_full_removal: false, force: current.pinned });
                setVersion(0);
                await loadVersions();
              })
            }
          >
            Delete version
          </Btn>
        </div>
        {cmp && (
          <div className="text-xs space-y-1">
            <div>
              {formatDate(cmp.base)} → {formatDate(cmp.compare)}: <span className="text-green-600">+{cmp.added.length}</span>{" "}
              <span className="text-amber-600">~{cmp.modified.length}</span> <span className="text-red-600">−{cmp.deleted.length}</span>
            </div>
            <ul className="max-h-28 overflow-y-auto font-mono text-[11px]">
              {cmp.added.map((p) => <li key={`a${p}`} className="text-green-600 truncate">+ {p}</li>)}
              {cmp.modified.map((p) => <li key={`m${p}`} className="text-amber-600 truncate">~ {p}</li>)}
              {cmp.deleted.map((p) => <li key={`d${p}`} className="text-red-600 truncate">− {p}</li>)}
            </ul>
          </div>
        )}
      </Section>

      <Section title="Search all versions">
        <div className="flex gap-1">
          <input className={inputCls} value={search} placeholder="name or wildcard, e.g. *.docx" onChange={(e) => setSearch(e.target.value)} />
          <Btn disabled={!search || busy} onClick={() => run(async () => setHits(await tauriInvoke<FoundFile[]>("backup_search", { id: job.id, pattern: search, version: null, ...src })))}>
            Search
          </Btn>
        </div>
        {hits && (
          <ul className="max-h-32 overflow-y-auto text-[11px] font-mono">
            {hits.length === 0 && <li className="text-zinc-500">No matches.</li>}
            {hits.map((h) => (
              <li key={`${h.version}:${h.path}`} className="flex justify-between gap-2">
                <button className="truncate text-left hover:underline" onClick={() => { setVersion(h.version); setSelected(new Set([h.path])); }} title="Select this version of the file">
                  {h.path}
                </button>
                <span className="shrink-0 text-zinc-500">{formatDate(h.timestamp)} · {formatBytes(h.size)}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title={`Files (${files.length}${selected.size ? `, ${selected.size} selected` : ""})`} hint="Select nothing to restore the whole version. Selecting a folder restores everything inside it.">
        <input className={inputCls} value={filter} placeholder="Filter…" onChange={(e) => setFilter(e.target.value)} />
        <ul className="max-h-56 overflow-y-auto text-[11px] font-mono border border-zinc-200 dark:border-zinc-700 rounded" data-testid="backup-file-list">
          {shown.map((f) => (
            <li key={f.path} className="flex items-center gap-2 px-1 hover:bg-zinc-50 dark:hover:bg-zinc-800">
              <input type="checkbox" checked={selected.has(f.path)} onChange={() => toggle(f.path)} />
              <span className="truncate flex-1" title={f.path}>
                {f.kind === "dir" ? "📁 " : f.kind === "symlink" ? "↪ " : ""}
                {f.path}
              </span>
              {f.kind === "file" && <span className="text-zinc-500 shrink-0">{formatBytes(f.size)}</span>}
            </li>
          ))}
        </ul>
      </Section>

      <Section title="Restore">
        <RestoreOptionsForm value={opts} onChange={setOpts} versionCount={versions.length} />
        <details className="text-xs" data-testid="restore-overrides">
          <summary className="cursor-pointer select-none">This restore only (overrides the job)</summary>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 mt-2">
            <Field label="Download limit (KB/s, empty = job setting)">
              <input className={inputCls} type="number" min={0} value={overrides.download_bps == null ? "" : Math.round(overrides.download_bps / 1024)} onChange={(e) => setOverrides({ ...overrides, download_bps: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) * 1024 })} />
            </Field>
            <Field label="Retries">
              <input className={inputCls} type="number" min={0} max={20} value={overrides.retries ?? ""} onChange={(e) => setOverrides({ ...overrides, retries: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) })} />
            </Field>
            <Field label="Network timeout (s)">
              <input className={inputCls} type="number" min={0} value={overrides.network_timeout_secs ?? ""} onChange={(e) => setOverrides({ ...overrides, network_timeout_secs: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) })} />
            </Field>
          </div>
        </details>
        <Btn
          tone="primary"
          disabled={busy || (opts.target !== null && !opts.target.trim())}
          testId="backup-restore-run"
          onClick={() =>
            run(async () => {
              const r = await execute(job.id, { type: "restore", options: { ...opts, version, paths: [...selected], source }, overrides });
              setReport(r.report as unknown as RestoreReport);
            })
          }
        >
          {busy ? "Working…" : opts.dry_run ? "Preview restore" : "Restore"}
        </Btn>
        {report && <ReportView report={report} />}
      </Section>
    </div>
  );
}

/** Restore from any destination using only its location and passphrase. */
export function DirectRestore() {
  const [dest, setDest] = useState<BackupDestination>({ type: "local", path: "", alternate_paths: [], marker_file: null });
  const [passphrase, setPassphrase] = useState("");
  const [prefix, setPrefix] = useState("ufop");
  const [versions, setVersions] = useState<Fileset[] | null>(null);
  const [opts, setOpts] = useState<RestoreOptions>({ ...DEFAULT_RESTORE_OPTIONS, target: "" });
  const [paths, setPaths] = useState("");
  const [config, setConfig] = useState<string | null>(null);
  const [adoptName, setAdoptName] = useState("");
  const [adoptSources, setAdoptSources] = useState("");
  const [remapFrom, setRemapFrom] = useState("");
  const [remapTo, setRemapTo] = useState("");
  const [adoptReadOnly, setAdoptReadOnly] = useState(false);
  const [adopted, setAdopted] = useState<string | null>(null);
  const [report, setReport] = useState<RestoreReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const args = { destination: dest, passphrase: passphrase || null, prefix };

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="p-3 space-y-3" data-testid="backup-direct-restore">
      <Banner tone="info">
        Restore from a backup folder without its original job — e.g. after reinstalling or on another computer. The local index is rebuilt from the destination.
      </Banner>
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <Section title="Backup location">
        <Field label="Folder (local disk, removable drive or mounted share)">
          <PathInput value={dest.type === "local" ? dest.path : ""} onChange={(p) => setDest({ type: "local", path: p, alternate_paths: [], marker_file: null })} />
        </Field>
        <div className="grid grid-cols-2 gap-2">
          <Field label="Passphrase (empty if unencrypted)">
            <input className={inputCls} type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
          </Field>
          <Field label="File-name prefix">
            <input className={inputCls} value={prefix} onChange={(e) => setPrefix(e.target.value)} />
          </Field>
        </div>
        <div className="flex gap-1">
          <Btn tone="primary" disabled={busy} onClick={() => run(async () => setVersions(await tauriInvoke<Fileset[]>("backup_browse_remote", args)))}>
            Connect &amp; list versions
          </Btn>
          <Btn
            disabled={busy}
            onClick={() =>
              run(async () => {
                const c = await tauriInvoke<unknown>("backup_discover_remote_config", args);
                setConfig(c ? JSON.stringify(c, null, 2) : "No configuration is stored in this backup.");
              })
            }
          >
            Read stored job configuration
          </Btn>
        </div>
        {config && (
          <div className="space-y-1">
            <pre className="text-[11px] max-h-32 overflow-auto bg-zinc-50 dark:bg-zinc-800 p-2 rounded">{config}</pre>
            {config.startsWith("{") && (
              <Btn
                onClick={() =>
                  run(async () => {
                    await tauriInvoke("backup_import_config", { json: config, exportPassphrase: null, importStats: false, save: true });
                    await useBackupStore.getState().loadJobs();
                    setConfig("Job recreated from the backup — set its passphrase in the job editor before the next run.");
                  })
                }
              >
                Recreate job from this configuration
              </Btn>
            )}
          </div>
        )}
      </Section>
      <Section title="Continue this backup on this computer" hint="Creates a job for this backup, rebuilds its local index and keeps the version history; unchanged data is not uploaded again.">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Job name (default: the stored one)">
            <input className={inputCls} value={adoptName} onChange={(e) => setAdoptName(e.target.value)} data-testid="adopt-name" />
          </Field>
          <Field label="Source folders (one per line; default: the stored ones)">
            <textarea className={`${inputCls} font-mono`} rows={2} value={adoptSources} onChange={(e) => setAdoptSources(e.target.value)} />
          </Field>
          <Field label="Old folder prefix (optional)" hint="e.g. C:\Users\old">
            <input className={`${inputCls} font-mono`} value={remapFrom} onChange={(e) => setRemapFrom(e.target.value)} />
          </Field>
          <Field label="Replace with">
            <input className={`${inputCls} font-mono`} value={remapTo} onChange={(e) => setRemapTo(e.target.value)} />
          </Field>
        </div>
        <Check
          label="Read-only (restore and browse only)"
          checked={adoptReadOnly}
          onChange={setAdoptReadOnly}
          hint="For someone else's backup or an archive: nothing is ever written to it from this computer."
        />
        <Btn
          tone="primary"
          testId="backup-adopt"
          disabled={busy || (dest.type === "local" && !dest.path.trim())}
          onClick={() =>
            run(async () => {
              const job = await tauriInvoke<BackupJob>("backup_adopt_repository", {
                ...args,
                name: adoptName.trim() || null,
                sources: adoptSources.split("\n").map((s) => s.trim()).filter(Boolean),
                sourceMap: remapFrom.trim() ? [[remapFrom.trim(), remapTo.trim()]] : [],
                readOnly: adoptReadOnly,
              });
              await useBackupStore.getState().loadJobs();
              setAdopted(`Job “${job.name}” created with ${job.stats.version_count} existing version(s); the next backup continues the history.`);
            })
          }
        >
          {busy ? "Working…" : "Continue backup here"}
        </Btn>
        {adopted && <Banner tone="success" onClose={() => setAdopted(null)}>{adopted}</Banner>}
      </Section>
      {versions && (
        <Section title="Restore">
          <select className={inputCls} value={opts.version} onChange={(e) => setOpts({ ...opts, version: Number(e.target.value) })}>
            {versions.map((v, i) => (
              <option key={v.id} value={i}>
                {formatDate(v.timestamp)} · {v.file_count} items · {formatBytes(v.total_size)}
                {v.partial ? " · partial" : ""}
              </option>
            ))}
          </select>
          <Field label="Paths or wildcards (one per line; empty = everything)">
            <textarea className={`${inputCls} font-mono`} rows={2} value={paths} onChange={(e) => setPaths(e.target.value)} />
          </Field>
          <RestoreOptionsForm value={opts} onChange={setOpts} />
          <Btn
            tone="primary"
            disabled={busy || (opts.target !== null && !opts.target.trim())}
            onClick={() =>
              run(async () =>
                setReport(
                  await tauriInvoke<RestoreReport>("backup_direct_restore", {
                    ...args,
                    options: { ...opts, paths: paths.split("\n").map((s) => s.trim()).filter(Boolean) },
                  }),
                ),
              )
            }
          >
            {busy ? "Working…" : "Restore"}
          </Btn>
          {report && <ReportView report={report} />}
        </Section>
      )}
    </div>
  );
}
