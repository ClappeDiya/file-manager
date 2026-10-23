import { useEffect, useMemo, useRef, useState } from "react";
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  flexRender,
  type ColumnDef,
  type SortingState,
  type Updater,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowUpDown, ArrowUp, ArrowDown } from "lucide-react";
import { cn } from "@ufop/ui-components";
import { formatBytes } from "@/lib/format-bytes";
import { TABLE_PAGE_SIZES, type TablePageSize, type TableView } from "@/stores/ui-store";
import { useTableView } from "@/hooks/use-table-view";

export interface FileEntry {
  id: string;
  name: string;
  type: "file" | "folder";
  size: number;
  modified: string;
  permissions: string;
}

/**
 * Generate demo file entries for testing.
 */
export function generateDemoFiles(count: number): FileEntry[] {
  const extensions = [".ts", ".tsx", ".json", ".css", ".md", ".txt", ".png", ".svg"];
  const folders = ["src", "components", "hooks", "lib", "pages", "assets", "styles", "utils"];

  return Array.from({ length: count }, (_, i) => {
    const isFolder = i < Math.floor(count * 0.2); // 20% folders
    const ext = extensions[i % extensions.length];
    const folder = folders[i % folders.length];

    return {
      id: `file-${i}`,
      name: isFolder ? folder : `file-${i}${ext}`,
      type: isFolder ? ("folder" as const) : ("file" as const),
      size: isFolder ? 0 : Math.floor(Math.random() * 1024 * 1024),
      modified: new Date(
        Date.now() - Math.floor(Math.random() * 30 * 24 * 60 * 60 * 1000),
      ).toISOString(),
      permissions: isFolder ? "drwxr-xr-x" : "-rw-r--r--",
    };
  });
}

const FILE_COLUMNS: ColumnDef<FileEntry>[] = [
  {
    accessorKey: "name",
    header: "Name",
    cell: ({ row }) => {
      const entry = row.original;
      return (
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "text-[length:var(--font-size-base)]",
              entry.type === "folder"
                ? "font-medium text-[color:var(--color-primary)]"
                : "text-[color:var(--color-text)]",
            )}
          >
            {entry.name}
          </span>
        </div>
      );
    },
  },
  {
    accessorKey: "size",
    header: "Size",
    cell: ({ getValue }) => (
      <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
        {getValue<number>() === 0 ? "--" : formatBytes(getValue<number>())}
      </span>
    ),
  },
  {
    accessorKey: "modified",
    header: "Modified",
    cell: ({ getValue }) => (
      <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
        {new Date(getValue<string>()).toLocaleDateString()}
      </span>
    ),
  },
  {
    accessorKey: "permissions",
    header: "Permissions",
    cell: ({ getValue }) => (
      <code className="text-[color:var(--color-text-tertiary)] text-[length:var(--font-size-xs)] font-mono">
        {getValue<string>()}
      </code>
    ),
  },
];

/** Optional per-column extras, set as `meta` on a column definition. */
export interface DataGridColumnMeta {
  /** Header tooltip. */
  title?: string;
  /** Extra classes for the header and cells (e.g. `text-right`). */
  className?: string;
}

/** What a server-paged grid asks its data source for. */
export interface GridQuery {
  offset: number;
  /** `null` = all rows ("All" page size). */
  limit: number | null;
  sort: { id: string; desc: boolean } | null;
}

export interface DataGridProps<T> {
  data: T[];
  /** Column definitions (default: the demo file listing columns). */
  columns?: ColumnDef<T, never>[] | ColumnDef<T>[];
  className?: string;
  /** Accessible name of the grid. */
  label?: string;
  /** Text filter box over every column (client-side); `false` hides it. */
  filter?: false | { placeholder: string; label: string };
  /** Shows the pager (page size 25/50/100/250/All and page buttons). */
  pagination?: boolean;
  /** Remembers page size and sort under this id in the UI store. */
  prefsKey?: string;
  /** Default view when nothing is remembered yet. */
  defaultView?: Partial<TableView>;
  /** Server-side paging and sorting: `data` is the current page, `total`
   *  the number of rows, and `onQueryChange` asks for another page. */
  server?: { total: number; onQueryChange: (q: GridQuery) => void };
  getRowId?: (row: T, index: number) => string;
  /** Extra attributes per row (classes, test ids). */
  rowProps?: (row: T) => { className?: string; "data-testid"?: string };
  /** Enter on a focused row. */
  onRowActivate?: (row: T) => void;
  /** Shown when there are no rows. */
  emptyText?: string;
  /** Compact rows and small text (backup tables). */
  dense?: boolean;
  /** Going back to the first page whenever this changes (e.g. an outside filter). */
  resetKey?: unknown;
}

const FILE_FILTER = { placeholder: "Filter files...", label: "Filter files by name" };
/** Height of the scroll area used by the "All" page size. */
const VIRTUAL_HEIGHT = 448;

/** "Rows a–b of n" pager with a page size selector and First/Previous/Next/Last. */
export function GridPager({
  total,
  pageIndex,
  pageSize,
  onPage,
  onPageSize,
  label,
}: {
  total: number;
  pageIndex: number;
  pageSize: TablePageSize;
  onPage: (i: number) => void;
  onPageSize: (s: TablePageSize) => void;
  label: string;
}) {
  const size = pageSize === "all" ? Math.max(total, 1) : pageSize;
  const pages = Math.max(1, Math.ceil(total / size));
  const from = total === 0 ? 0 : pageIndex * size + 1;
  const to = Math.min(total, (pageIndex + 1) * size);
  const btn =
    "px-1.5 rounded border border-[var(--color-border)] disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]";
  return (
    <nav aria-label={`${label} pages`} className="flex flex-wrap items-center gap-2 text-[11px] text-[color:var(--color-text-secondary)]" data-testid="grid-pager">
      <label className="flex items-center gap-1">
        Rows per page
        <select
          className="rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-1"
          value={String(pageSize)}
          onChange={(e) => onPageSize(e.target.value === "all" ? "all" : (Number(e.target.value) as TablePageSize))}
          data-testid="grid-page-size"
        >
          {TABLE_PAGE_SIZES.map((s) => (
            <option key={String(s)} value={String(s)}>
              {s === "all" ? "All" : s}
            </option>
          ))}
        </select>
      </label>
      <span aria-live="polite" data-testid="grid-range">
        {from}–{to} of {total.toLocaleString()}
      </span>
      {pageSize !== "all" && pages > 1 && (
        <span className="flex items-center gap-1">
          <button type="button" className={btn} disabled={pageIndex === 0} onClick={() => onPage(0)} aria-label="First page">
            «
          </button>
          <button type="button" className={btn} disabled={pageIndex === 0} onClick={() => onPage(pageIndex - 1)} aria-label="Previous page">
            ‹
          </button>
          <span>
            Page {pageIndex + 1} of {pages}
          </span>
          <button type="button" className={btn} disabled={pageIndex >= pages - 1} onClick={() => onPage(pageIndex + 1)} aria-label="Next page">
            ›
          </button>
          <button type="button" className={btn} disabled={pageIndex >= pages - 1} onClick={() => onPage(pages - 1)} aria-label="Last page">
            »
          </button>
        </span>
      )}
    </nav>
  );
}

/**
 * TanStack Table-based data grid: sortable headers (click or Enter/Space,
 * `aria-sort`), optional text filter, optional pager with remembered page
 * size and sort, virtualized rows for the "All" page size, and client- or
 * server-side paging. Without `columns` it renders the demo file listing.
 */
export function DataGrid<T = FileEntry>({
  data,
  columns,
  className,
  label = "File listing",
  filter,
  pagination = false,
  prefsKey,
  defaultView,
  server,
  getRowId,
  rowProps,
  onRowActivate,
  emptyText,
  dense = false,
  resetKey,
}: DataGridProps<T>) {
  const [globalFilter, setGlobalFilter] = useState("");
  const [view, setView] = useTableView(prefsKey, defaultView);
  const [pageIndex, setPageIndex] = useState(0);
  const [lastReset, setLastReset] = useState(resetKey);
  if (lastReset !== resetKey) {
    setLastReset(resetKey);
    setPageIndex(0);
  }
  const cols = (columns ?? FILE_COLUMNS) as ColumnDef<T>[];
  const filterBox = filter === undefined ? (columns ? false : FILE_FILTER) : filter;
  const sorting: SortingState = useMemo(() => (view.sort ? [view.sort] : []), [view.sort]);

  const onSortingChange = (u: Updater<SortingState>) => {
    const next = typeof u === "function" ? u(sorting) : u;
    setView({ sort: next[0] ? { id: next[0].id, desc: next[0].desc } : null });
    setPageIndex(0);
  };

  const table = useReactTable<T>({
    data,
    columns: cols,
    state: { sorting, globalFilter },
    onSortingChange,
    onGlobalFilterChange: setGlobalFilter,
    enableMultiSort: false,
    // Every column starts ascending (columns may opt into descending first).
    sortDescFirst: false,
    manualSorting: !!server,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getRowId,
  });

  const pageSize: TablePageSize = pagination ? view.pageSize : "all";
  const allRows = table.getRowModel().rows;
  const total = server ? server.total : allRows.length;
  const size = pageSize === "all" ? Number.MAX_SAFE_INTEGER : pageSize;
  const pages = Math.max(1, Math.ceil(total / size));
  const page = Math.min(pageIndex, pages - 1);
  const rows = server || pageSize === "all" ? allRows : allRows.slice(page * size, page * size + size);
  const offset = pageSize === "all" ? 0 : page * size;

  // Ask the server for the page this view shows.
  const onQuery = server?.onQueryChange;
  useEffect(() => {
    onQuery?.({ offset: pageSize === "all" ? 0 : page * (pageSize as number), limit: pageSize === "all" ? null : pageSize, sort: view.sort });
  }, [onQuery, page, pageSize, view.sort]);

  // "All" with a pager: one virtualized scroll area instead of pages.
  const virtual = pagination && pageSize === "all";
  const scrollRef = useRef<HTMLDivElement>(null);
  const rowHeight = dense ? 26 : 36;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
    enabled: virtual,
    initialRect: { width: 800, height: VIRTUAL_HEIGHT },
  });
  const vItems = virtual ? virtualizer.getVirtualItems() : [];
  const shown = virtual ? vItems.map((v) => rows[v.index]).filter(Boolean) : rows;
  const padTop = virtual && vItems.length ? vItems[0].start : 0;
  const padBottom = virtual && vItems.length ? virtualizer.getTotalSize() - vItems[vItems.length - 1].end : 0;
  const firstIndex = virtual && vItems.length ? vItems[0].index : 0;

  const cell = dense ? "px-1 py-0.5 text-[11px]" : "px-3";
  const colSpan = table.getVisibleLeafColumns().length;
  const pager = pagination && (
    <GridPager
      total={total}
      pageIndex={page}
      pageSize={pageSize}
      label={label}
      onPage={setPageIndex}
      onPageSize={(s) => {
        setView({ pageSize: s });
        setPageIndex(0);
      }}
    />
  );

  return (
    <div className={cn("space-y-2", className)}>
      {filterBox && (
        <div className="flex items-center gap-2">
          <label htmlFor="data-grid-filter" className="sr-only">
            {filterBox.label}
          </label>
          <input
            id="data-grid-filter"
            type="text"
            placeholder={filterBox.placeholder}
            value={globalFilter}
            onChange={(e) => {
              setGlobalFilter(e.target.value);
              setPageIndex(0);
            }}
            className={cn(
              "flex h-8 w-64 rounded-[var(--radius-md)] px-3 py-1",
              "text-[length:var(--font-size-sm)] text-[color:var(--color-text)]",
              "bg-[var(--color-bg)] border border-[var(--color-border)]",
              "placeholder:text-[color:var(--color-text-tertiary)]",
              "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
            )}
            aria-label={filterBox.label}
          />
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]" aria-live="polite">
            {total} items
          </span>
        </div>
      )}

      <div
        ref={scrollRef}
        className={cn("rounded-[var(--radius-md)] border border-[var(--color-border)]", virtual ? "overflow-auto" : "overflow-x-auto")}
        style={virtual ? { maxHeight: VIRTUAL_HEIGHT } : undefined}
        role="grid"
        aria-label={label}
        aria-rowcount={total}
      >
        <table className="w-full border-collapse">
          <thead className={virtual ? "sticky top-0 z-[1]" : undefined}>
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id} role="row">
                {headerGroup.headers.map((header) => {
                  const meta = header.column.columnDef.meta as DataGridColumnMeta | undefined;
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th
                      key={header.id}
                      role="columnheader"
                      title={meta?.title}
                      aria-sort={sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : "none"}
                      className={cn(
                        dense ? "h-7 px-1 font-normal text-[11px]" : "h-9 px-3 text-[length:var(--font-size-xs)] font-medium uppercase tracking-wider",
                        "text-left align-middle text-[color:var(--color-text-secondary)]",
                        "bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)]",
                        canSort && "cursor-pointer select-none",
                        meta?.className,
                      )}
                      onClick={header.column.getToggleSortingHandler()}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          header.column.getToggleSortingHandler()?.(e);
                        }
                      }}
                      tabIndex={canSort ? 0 : undefined}
                    >
                      <div className={cn("flex items-center gap-1", meta?.className?.includes("text-right") && "justify-end")}>
                        {flexRender(header.column.columnDef.header, header.getContext())}
                        {canSort && (
                          <span className="ml-1" aria-hidden="true">
                            {sorted === "asc" ? (
                              <ArrowUp className="h-3 w-3" />
                            ) : sorted === "desc" ? (
                              <ArrowDown className="h-3 w-3" />
                            ) : (
                              <ArrowUpDown className="h-3 w-3 opacity-40" />
                            )}
                          </span>
                        )}
                      </div>
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {padTop > 0 && (
              <tr aria-hidden="true">
                <td colSpan={colSpan} style={{ height: padTop, padding: 0 }} />
              </tr>
            )}
            {shown.map((row, index) => {
              const extra = rowProps?.(row.original);
              return (
                <tr
                  key={row.id}
                  role="row"
                  aria-rowindex={offset + firstIndex + index + 1}
                  data-testid={extra?.["data-testid"]}
                  className={cn(
                    "border-b border-[var(--color-border)] last:border-b-0",
                    "hover:bg-[var(--color-hover-bg)] transition-theme",
                    "focus-within:bg-[var(--color-selection-bg)]",
                    extra?.className,
                  )}
                  style={virtual ? { height: rowHeight } : undefined}
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && e.target === e.currentTarget) onRowActivate?.(row.original);
                  }}
                >
                  {row.getVisibleCells().map((c) => (
                    <td
                      key={c.id}
                      role="gridcell"
                      className={cn(cell, "align-middle", (c.column.columnDef.meta as DataGridColumnMeta | undefined)?.className)}
                      style={dense ? undefined : { height: "var(--file-row-height)" }}
                    >
                      {flexRender(c.column.columnDef.cell, c.getContext())}
                    </td>
                  ))}
                </tr>
              );
            })}
            {padBottom > 0 && (
              <tr aria-hidden="true">
                <td colSpan={colSpan} style={{ height: padBottom, padding: 0 }} />
              </tr>
            )}
          </tbody>
        </table>
        {total === 0 && emptyText && <p className="text-xs text-[color:var(--color-text-tertiary)] p-2">{emptyText}</p>}
      </div>
      {pager}
    </div>
  );
}
