/**
 * Files of one version in the restore view (K122): sortable columns, a
 * pager with remembered page size and sort ("backup-files"), and the text
 * filter, all answered by the backend one page at a time so versions with
 * hundreds of thousands of files stay quick. Selection survives paging.
 */
import { useEffect, useState } from "react";
import type { ColumnDef } from "@tanstack/react-table";
import { formatBytes } from "@/lib/format-bytes";
import { formatDate, inputCls } from "@/lib/backup-helpers";
import type { BackupFileEntry, FoundFile } from "@/stores/backup-store";
import { DataGrid } from "../data-grid";
import { useTableView } from "@/hooks/use-table-view";
import { usePagedListing } from "@/hooks/use-paged-listing";
import { Banner, Section } from "./backup-ui";

const PREFS = "backup-files";

/** `value`, once it has stopped changing for `ms`. */
function useSettled<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export function BackupFileBrowser({
  jobId,
  version,
  source,
  selected,
  onToggle,
  onPreview,
}: {
  jobId: string;
  version: number;
  /** Another source's versions (shared repositories); null = the job's own. */
  source: string | null;
  selected: Set<string>;
  onToggle: (path: string) => void;
  onPreview: (path: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const settled = useSettled(filter, 250);
  const [view] = useTableView(PREFS);
  const args = { id: jobId, version, prefix: null, ...(source ? { source } : {}) };
  const list = usePagedListing<BackupFileEntry>("backup_list_files", args, { view, filter: settled, textOf: (f) => f.path });

  const columns: ColumnDef<BackupFileEntry>[] = [
    {
      id: "select",
      header: () => <span className="sr-only">Select</span>,
      enableSorting: false,
      cell: ({ row: { original: f } }) => <input type="checkbox" aria-label={`Select ${f.path}`} checked={selected.has(f.path)} onChange={() => onToggle(f.path)} />,
    },
    {
      id: "path",
      header: "Path",
      accessorFn: (f) => f.path,
      cell: ({ row: { original: f } }) => (
        <span className="block truncate max-w-[32rem] font-mono" title={f.path}>
          {f.kind === "dir" ? "📁 " : f.kind === "symlink" ? "↪ " : ""}
          {f.path}
        </span>
      ),
    },
    {
      id: "size",
      header: "Size",
      accessorFn: (f) => f.size,
      cell: ({ row: { original: f } }) => (f.kind === "file" ? formatBytes(f.size) : ""),
      meta: { className: "text-right whitespace-nowrap" },
    },
    {
      id: "modified",
      header: "Modified",
      accessorFn: (f) => f.mtime ?? 0,
      cell: ({ row: { original: f } }) => (f.mtime ? formatDate(new Date(f.mtime * 1000).toISOString()) : "—"),
      meta: { className: "whitespace-nowrap text-zinc-500" },
    },
    {
      id: "preview",
      header: () => <span className="sr-only">Preview</span>,
      enableSorting: false,
      cell: ({ row: { original: f } }) =>
        f.kind === "file" && (
          <button className="text-blue-600 dark:text-blue-400 hover:underline" onClick={() => onPreview(f.path)} title="Show this file without restoring it" data-testid="backup-file-preview-open">
            Preview
          </button>
        ),
    },
  ];

  return (
    <Section title={`Files (${list.total.toLocaleString()}${selected.size ? `, ${selected.size} selected` : ""})`} hint="Select nothing to restore the whole version. Selecting a folder restores everything inside it.">
      <input className={inputCls} value={filter} placeholder="Filter…" aria-label="Filter files" onChange={(e) => setFilter(e.target.value)} />
      {list.error && <Banner tone="error">{list.error}</Banner>}
      <div data-testid="backup-file-list">
        <DataGrid<BackupFileEntry>
          data={list.rows}
          columns={columns}
          label="Files in this version"
          dense
          pagination
          prefsKey={PREFS}
          server={list.server}
          getRowId={(f) => f.path}
          onRowActivate={(f) => onToggle(f.path)}
          emptyText={settled ? "No file matches the filter." : "This version has no files."}
          resetKey={`${version}|${source}|${settled}`}
        />
      </div>
    </Section>
  );
}

/** "Search all versions" results: sortable, paged ("backup-search"); picking
 *  one selects that file in its version. */
export function BackupSearchHits({ hits, onPick }: { hits: FoundFile[]; onPick: (h: FoundFile) => void }) {
  const columns: ColumnDef<FoundFile>[] = [
    {
      id: "path",
      header: "Path",
      accessorFn: (h) => h.path,
      cell: ({ row: { original: h } }) => (
        <button className="block truncate max-w-[28rem] text-left font-mono hover:underline" onClick={() => onPick(h)} title="Select this version of the file">
          {h.path}
        </button>
      ),
    },
    { id: "time", header: "Version", accessorFn: (h) => h.timestamp, cell: ({ row: { original: h } }) => formatDate(h.timestamp), meta: { className: "whitespace-nowrap text-zinc-500" } },
    { id: "size", header: "Size", accessorFn: (h) => h.size, cell: ({ row: { original: h } }) => formatBytes(h.size), meta: { className: "text-right whitespace-nowrap text-zinc-500" } },
  ];
  return (
    <DataGrid<FoundFile>
      data={hits}
      columns={columns}
      label="Search results"
      dense
      pagination
      prefsKey="backup-search"
      defaultView={{ pageSize: 25 }}
      getRowId={(h) => `${h.version}:${h.path}`}
      onRowActivate={onPick}
      emptyText="No matches."
      resetKey={hits}
    />
  );
}
