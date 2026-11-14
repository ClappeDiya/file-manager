/**
 * Version table (K7, K22, K113, K69, K76): every version with its time,
 * size, the storage it added, a "Keep forever" pin and an editable note
 * (Standard), plus tag chips and filters, the computer that wrote it and
 * "Copy to another backup…" (Advanced).
 *
 * Version numbers are positions in the unfiltered list (0 = newest of the
 * selected source), as the backend expects. Columns sort by a header click
 * and the table pages (K122); the view is remembered as "backup-versions".
 */
import { useMemo, useState } from "react";
import { formatBytes } from "@/lib/format-bytes";
import { formatDate, inputCls } from "@/lib/backup-helpers";
import {
  errorText,
  matchesTags,
  parseTags,
  tagText,
  useBackupStore,
  type BackupJob,
  type CopyReport,
  type CopyTarget,
  type Fileset,
} from "@/stores/backup-store";
import { AdvancedOnly } from "./advanced-only";
import { useIsAdvanced } from "./use-is-advanced";
import { Banner, Btn, Field, PathInput } from "./backup-ui";
import { DataGrid } from "../data-grid";
import type { ColumnDef } from "@tanstack/react-table";

/** A version and its position in the unfiltered list (the backend's version number). */
type Row = { v: Fileset; i: number };

function NoteCell({ value, onSave, disabled }: { value: string | null; onSave: (note: string) => Promise<void>; disabled: boolean }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(value ?? "");
  if (!editing) {
    return (
      <button
        type="button"
        className="text-left truncate max-w-[16rem] hover:underline disabled:no-underline"
        disabled={disabled}
        title={value ? `${value} (click to edit)` : "Add a note"}
        onClick={() => {
          setText(value ?? "");
          setEditing(true);
        }}
      >
        {value || <span className="text-zinc-400">Add note…</span>}
      </button>
    );
  }
  const save = async () => {
    setEditing(false);
    if (text.trim() !== (value ?? "")) await onSave(text.trim());
  };
  return (
    <input
      autoFocus
      aria-label="Version note"
      className={`${inputCls} py-0`}
      value={text}
      maxLength={2000}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => void save()}
      onKeyDown={(e) => {
        if (e.key === "Enter") void save();
        if (e.key === "Escape") setEditing(false);
      }}
    />
  );
}

function TagsCell({ tags, onSave, disabled }: { tags: Record<string, string>; onSave: (tags: Record<string, string>) => Promise<void>; disabled: boolean }) {
  const advanced = useIsAdvanced();
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const entries = Object.entries(tags);
  if (editing) {
    return (
      <input
        autoFocus
        aria-label="Version tags"
        className={`${inputCls} py-0 font-mono`}
        placeholder="key=value, key2"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          setEditing(false);
          void onSave(parseTags(text));
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setEditing(false);
        }}
      />
    );
  }
  return (
    <span className="flex flex-wrap gap-1 items-center">
      {entries.map(([k, v]) => (
        <span key={k} className="px-1 rounded bg-zinc-100 dark:bg-zinc-800 text-[10px] font-mono">
          {tagText(k, v)}
        </span>
      ))}
      {advanced && (
        <button
          type="button"
          className="text-[10px] text-blue-600 dark:text-blue-400 hover:underline"
          disabled={disabled}
          onClick={() => {
            setText(entries.map(([k, v]) => tagText(k, v)).join(", "));
            setEditing(true);
          }}
        >
          {entries.length ? "edit" : "+ tag"}
        </button>
      )}
    </span>
  );
}

/** Copy chosen versions to another job's backup or a folder (Advanced). */
function CopyVersionsDialog({ job, versions, source, onClose }: { job: BackupJob; versions: number[]; source: string | null; onClose: () => void }) {
  const { jobs, execute } = useBackupStore();
  const others = jobs.filter((j) => j.id !== job.id && !j.read_only);
  const [mode, setMode] = useState<"job" | "folder">(others.length ? "job" : "folder");
  const [targetJob, setTargetJob] = useState(others[0]?.id ?? "");
  const [folder, setFolder] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [report, setReport] = useState<CopyReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const target = (): CopyTarget =>
    mode === "job"
      ? { type: "job", job_id: targetJob }
      : { type: "destination", destination: { type: "local", path: folder, alternate_paths: [], marker_file: null }, passphrase: passphrase || null, prefix: null, options: null };
  const go = async (dry_run: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const r = await execute(job.id, { type: "copy_versions", versions, source, target: target(), dry_run });
      setReport(r.report as unknown as CopyReport);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const ready = mode === "job" ? !!targetJob : !!folder.trim();
  return (
    <div className="border border-zinc-200 dark:border-zinc-700 rounded p-2 space-y-2 text-xs" data-testid="copy-versions-dialog">
      <div className="font-medium">Copy {versions.length} version(s) to another backup</div>
      <div className="flex gap-3">
        <label className="flex items-center gap-1">
          <input type="radio" checked={mode === "job"} disabled={!others.length} onChange={() => setMode("job")} /> Another job
        </label>
        <label className="flex items-center gap-1">
          <input type="radio" checked={mode === "folder"} onChange={() => setMode("folder")} /> A folder
        </label>
      </div>
      {mode === "job" ? (
        <Field label="Backup job">
          <select className={inputCls} value={targetJob} onChange={(e) => setTargetJob(e.target.value)} data-testid="copy-target-job">
            {others.map((j) => (
              <option key={j.id} value={j.id}>
                {j.name}
              </option>
            ))}
          </select>
        </Field>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Folder (created when empty)">
            <PathInput value={folder} onChange={setFolder} testId="copy-target-folder" />
          </Field>
          <Field label="Its passphrase (empty = unencrypted)">
            <input className={inputCls} type="password" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
          </Field>
        </div>
      )}
      <p className="text-zinc-500">Only data the other backup does not have yet is uploaded. Notes, tags and pins are kept.</p>
      {error && <Banner tone="error">{error}</Banner>}
      {report && (
        <Banner tone={report.warnings.length ? "warning" : "success"}>
          {report.dry_run ? "Would copy" : "Copied"} {report.versions_copied} version(s) ({report.blocks_copied} new block(s), {formatBytes(report.bytes_uploaded)}), {report.versions_skipped} already there
          {report.rechunked ? "; the data was split into blocks again for the other backup" : ""}.
          {report.warnings.map((w) => (
            <div key={w}>{w}</div>
          ))}
        </Banner>
      )}
      <div className="flex gap-1">
        <Btn disabled={busy || !ready} onClick={() => void go(true)}>
          Check first (dry run)
        </Btn>
        <Btn tone="primary" disabled={busy || !ready} onClick={() => void go(false)} testId="copy-versions-run">
          {busy ? "Working…" : "Copy"}
        </Btn>
        <Btn onClick={onClose}>Close</Btn>
      </div>
    </div>
  );
}

export function BackupVersionTable({
  job,
  versions,
  selected,
  onSelect,
  source,
  onChanged,
}: {
  job: BackupJob;
  versions: Fileset[];
  selected: number;
  onSelect: (i: number) => void;
  /** null = the job's own source. */
  source: string | null;
  onChanged: () => Promise<void>;
}) {
  const setVersionMetadata = useBackupStore((s) => s.setVersionMetadata);
  const advanced = useIsAdvanced();
  const [tagFilter, setTagFilter] = useState<Record<string, string>>({});
  const [host, setHost] = useState("");
  const [copying, setCopying] = useState<number[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const readOnly = job.read_only;

  const allTags = useMemo(() => {
    const m = new Map<string, string>();
    for (const v of versions) for (const [k, val] of Object.entries(v.tags ?? {})) m.set(tagText(k, val), k);
    return [...m.keys()].sort();
  }, [versions]);
  const hosts = useMemo(() => [...new Set(versions.map((v) => v.identity?.hostname).filter((h): h is string => !!h))].sort(), [versions]);
  const rows: Row[] = useMemo(
    () => versions.map((v, i) => ({ v, i })).filter(({ v }) => matchesTags(v.tags ?? {}, tagFilter) && (!host || v.identity?.hostname === host)),
    [versions, tagFilter, host],
  );

  const update = async (i: number, patch: Parameters<typeof setVersionMetadata>[2]) => {
    setBusy(true);
    setError(null);
    try {
      await setVersionMetadata(job.id, i, patch, source);
      await onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  // Rebuilt each render so the cells always use the current job, source and busy state.
  const columns: ColumnDef<Row>[] = [
    {
      id: "time",
      header: "Time",
      accessorFn: ({ v }) => v.timestamp,
      cell: ({ row: { original: { v, i } } }) => (
        <button type="button" className="hover:underline whitespace-nowrap" onClick={() => onSelect(i)}>
          {i === 0 ? "Latest — " : ""}
          {formatDate(v.timestamp)}
          {v.partial ? " (partial)" : ""}
        </button>
      ),
    },
    { id: "items", header: "Items", accessorFn: ({ v }) => v.file_count, meta: { className: "text-right" } },
    {
      id: "size",
      header: "Size",
      accessorFn: ({ v }) => v.total_size,
      cell: ({ row: { original: { v } } }) => formatBytes(v.total_size),
      meta: { className: "text-right whitespace-nowrap" },
    },
    {
      id: "added",
      header: "Added",
      accessorFn: ({ v }) => v.stats?.bytes_added ?? -1,
      cell: ({ row: { original: { v } } }) => (v.stats ? formatBytes(v.stats.bytes_added) : "—"),
      meta: { className: "text-right whitespace-nowrap", title: "Storage this version added (after compression and encryption)" },
    },
    {
      id: "keep",
      header: "Keep",
      accessorFn: ({ v }) => (v.pinned ? 1 : 0),
      sortDescFirst: true,
      meta: { title: "Keep forever" },
      cell: ({ row: { original: { v, i } } }) => (
        <button
          type="button"
          aria-pressed={v.pinned}
          aria-label={v.pinned ? "Kept forever (click to unpin)" : "Keep forever"}
          title={v.pinned ? `Kept forever${v.pin_reason ? `: ${v.pin_reason}` : ""} — retention never removes it` : "Keep forever"}
          disabled={busy || readOnly}
          className={v.pinned ? "" : "opacity-30 hover:opacity-80"}
          onClick={() => void update(i, { pinned: !v.pinned })}
          data-testid={`version-pin-${i}`}
        >
          📌
        </button>
      ),
    },
    {
      id: "note",
      header: "Note",
      accessorFn: ({ v }) => v.note ?? "",
      cell: ({ row: { original: { v, i } } }) => <NoteCell value={v.note} disabled={busy || readOnly} onSave={(note) => update(i, { note })} />,
    },
    {
      id: "tags",
      header: "Tags",
      enableSorting: false,
      cell: ({ row: { original: { v, i } } }) => <TagsCell tags={v.tags ?? {}} disabled={busy || readOnly} onSave={(tags) => update(i, { tags })} />,
    },
    ...(advanced
      ? [
          {
            id: "computer",
            header: "Computer",
            accessorFn: ({ v }: Row) => (v.identity ? `${v.identity.username}@${v.identity.hostname}` : ""),
            cell: ({ row: { original: { v } } }: { row: { original: Row } }) => (v.identity ? `${v.identity.username}@${v.identity.hostname}` : "—"),
            meta: { className: "whitespace-nowrap text-zinc-500" },
          } as ColumnDef<Row>,
        ]
      : []),
  ];

  const toggleTag = (t: string) =>
    setTagFilter((f) => {
      const parsed = parseTags(t);
      const [k] = Object.keys(parsed);
      const n = { ...f };
      if (k in n && n[k] === parsed[k]) delete n[k];
      else n[k] = parsed[k];
      return n;
    });

  return (
    <div className="space-y-1" data-testid="backup-version-table">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <AdvancedOnly>
        {(allTags.length > 0 || hosts.length > 1) && (
          <div className="flex flex-wrap gap-1 items-center text-[11px]" data-testid="version-filters">
            {allTags.map((t) => {
              const [k] = Object.keys(parseTags(t));
              const on = k in tagFilter && tagText(k, tagFilter[k]) === t;
              return (
                <button
                  key={t}
                  type="button"
                  aria-pressed={on}
                  className={`px-1.5 rounded-full border font-mono ${on ? "bg-blue-600 text-white border-blue-600" : "border-zinc-300 dark:border-zinc-600"}`}
                  onClick={() => toggleTag(t)}
                >
                  {t}
                </button>
              );
            })}
            {hosts.length > 1 && (
              <select className={`${inputCls} w-auto py-0`} value={host} onChange={(e) => setHost(e.target.value)} aria-label="Computer">
                <option value="">All computers</option>
                {hosts.map((h) => (
                  <option key={h} value={h}>
                    {h}
                  </option>
                ))}
              </select>
            )}
          </div>
        )}
      </AdvancedOnly>
      <DataGrid<Row>
        data={rows}
        columns={columns}
        label="Versions"
        dense
        pagination
        prefsKey="backup-versions"
        getRowId={(r) => String(r.v.id)}
        rowProps={({ i }) => ({ className: i === selected ? "bg-blue-50 dark:bg-blue-950" : undefined, "data-testid": `version-row-${i}` })}
        onRowActivate={({ i }) => onSelect(i)}
        emptyText="No version matches the filter."
        resetKey={`${host}|${JSON.stringify(tagFilter)}`}
      />
      <AdvancedOnly>
        <div className="flex gap-1 flex-wrap">
          <Btn disabled={busy} onClick={() => setCopying([selected])} testId="copy-versions-open">
            Copy to another backup…
          </Btn>
          {rows.length > 1 && (
            <Btn disabled={busy} onClick={() => setCopying(rows.map((r) => r.i))}>
              Copy the {rows.length} shown…
            </Btn>
          )}
        </div>
        {copying && <CopyVersionsDialog job={job} versions={copying} source={source} onClose={() => setCopying(null)} />}
      </AdvancedOnly>
      {readOnly && <p className="text-[11px] text-zinc-500">Notes and pins cannot change on a read-only job.</p>}
    </div>
  );
}
