import { useMemo, useState } from "react";
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  flexRender,
  type ColumnDef,
  type SortingState,
} from "@tanstack/react-table";
import { ArrowUpDown, ArrowUp, ArrowDown } from "lucide-react";
import { cn } from "@ufop/ui-components";

export interface FileEntry {
  id: string;
  name: string;
  type: "file" | "folder";
  size: number;
  modified: string;
  permissions: string;
}

/**
 * Format file size to human-readable string.
 */
function formatFileSize(bytes: number): string {
  if (bytes === 0) return "--";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
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

interface DataGridProps {
  data: FileEntry[];
  className?: string;
}

/**
 * TanStack Table-based data grid for file listings.
 * Supports sorting, filtering, and keyboard navigation.
 */
export function DataGrid({ data, className }: DataGridProps) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [globalFilter, setGlobalFilter] = useState("");

  const columns = useMemo<ColumnDef<FileEntry>[]>(
    () => [
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
            {formatFileSize(getValue<number>())}
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
    ],
    [],
  );

  const table = useReactTable({
    data,
    columns,
    state: { sorting, globalFilter },
    onSortingChange: setSorting,
    onGlobalFilterChange: setGlobalFilter,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
  });

  return (
    <div className={cn("space-y-3", className)}>
      {/* Search/filter */}
      <div className="flex items-center gap-2">
        <label htmlFor="data-grid-filter" className="sr-only">
          Filter files
        </label>
        <input
          id="data-grid-filter"
          type="text"
          placeholder="Filter files..."
          value={globalFilter}
          onChange={(e) => setGlobalFilter(e.target.value)}
          className={cn(
            "flex h-8 w-64 rounded-[var(--radius-md)] px-3 py-1",
            "text-[length:var(--font-size-sm)] text-[color:var(--color-text)]",
            "bg-[var(--color-bg)] border border-[var(--color-border)]",
            "placeholder:text-[color:var(--color-text-tertiary)]",
            "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          aria-label="Filter files by name"
        />
        <span
          className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]"
          aria-live="polite"
        >
          {table.getFilteredRowModel().rows.length} items
        </span>
      </div>

      {/* Table */}
      <div
        className="overflow-hidden rounded-[var(--radius-md)] border border-[var(--color-border)]"
        role="grid"
        aria-label="File listing"
        aria-rowcount={table.getFilteredRowModel().rows.length}
      >
        <table className="w-full border-collapse">
          <thead>
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id} role="row">
                {headerGroup.headers.map((header) => (
                  <th
                    key={header.id}
                    role="columnheader"
                    aria-sort={
                      header.column.getIsSorted() === "asc"
                        ? "ascending"
                        : header.column.getIsSorted() === "desc"
                          ? "descending"
                          : "none"
                    }
                    className={cn(
                      "h-9 px-3 text-left align-middle",
                      "text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] uppercase tracking-wider",
                      "bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)]",
                      header.column.getCanSort() && "cursor-pointer select-none",
                    )}
                    onClick={header.column.getToggleSortingHandler()}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        header.column.getToggleSortingHandler()?.(e);
                      }
                    }}
                    tabIndex={header.column.getCanSort() ? 0 : undefined}
                  >
                    <div className="flex items-center gap-1">
                      {flexRender(
                        header.column.columnDef.header,
                        header.getContext(),
                      )}
                      {header.column.getCanSort() && (
                        <span className="ml-1" aria-hidden="true">
                          {header.column.getIsSorted() === "asc" ? (
                            <ArrowUp className="h-3 w-3" />
                          ) : header.column.getIsSorted() === "desc" ? (
                            <ArrowDown className="h-3 w-3" />
                          ) : (
                            <ArrowUpDown className="h-3 w-3 opacity-40" />
                          )}
                        </span>
                      )}
                    </div>
                  </th>
                ))}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.map((row, index) => (
              <tr
                key={row.id}
                role="row"
                aria-rowindex={index + 1}
                className={cn(
                  "border-b border-[var(--color-border)] last:border-b-0",
                  "hover:bg-[var(--color-hover-bg)] transition-theme",
                  "focus-within:bg-[var(--color-selection-bg)]",
                )}
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    // Future: open file/folder
                  }
                }}
              >
                {row.getVisibleCells().map((cell) => (
                  <td
                    key={cell.id}
                    role="gridcell"
                    className="px-3 align-middle"
                    style={{ height: "var(--file-row-height)" }}
                  >
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
