import React, { useMemo, useRef, useCallback, useState, useEffect } from "react";
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  flexRender,
  type ColumnDef,
  type SortingState,
  type ColumnResizeMode,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { cn } from "@ufop/ui-components";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  File,
  Folder,
  FileText,
  FileImage,
  FileCode,
  FileArchive,
  Film,
  Music,
} from "lucide-react";
import type { ViewMode } from "@/stores/file-manager-store";

// ──────────────────────────────────────────────
// Types (maps to Rust FileEntry)
// ──────────────────────────────────────────────

export interface FileEntryData {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: string | null;
  created: string | null;
  is_hidden: boolean;
  extension: string | null;
  permissions: string | null;
}

// ──────────────────────────────────────────────
// Utility functions
// ──────────────────────────────────────────────

export function formatFileSize(bytes: number): string {
  if (bytes === 0) return "--";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

export function formatDate(dateStr: string | null): string {
  if (!dateStr) return "--";
  const date = new Date(dateStr);
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function getFileIcon(entry: FileEntryData): React.ReactNode {
  if (entry.is_dir) {
    return <Folder className="h-4 w-4 text-[color:var(--color-primary)]" aria-hidden="true" />;
  }
  const ext = entry.extension?.toLowerCase();
  switch (ext) {
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
    case "py":
    case "rs":
    case "go":
    case "java":
    case "c":
    case "cpp":
    case "h":
    case "css":
    case "html":
    case "json":
    case "yaml":
    case "yml":
    case "toml":
      return <FileCode className="h-4 w-4 text-[color:var(--color-info)]" aria-hidden="true" />;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "svg":
    case "webp":
    case "ico":
      return <FileImage className="h-4 w-4 text-[color:var(--color-success)]" aria-hidden="true" />;
    case "zip":
    case "tar":
    case "gz":
    case "7z":
    case "rar":
      return <FileArchive className="h-4 w-4 text-[color:var(--color-warning)]" aria-hidden="true" />;
    case "mp4":
    case "mov":
    case "avi":
    case "mkv":
    case "webm":
      return <Film className="h-4 w-4 text-[color:var(--color-error)]" aria-hidden="true" />;
    case "mp3":
    case "wav":
    case "flac":
    case "aac":
    case "ogg":
      return <Music className="h-4 w-4 text-[color:var(--color-accent)]" aria-hidden="true" />;
    case "md":
    case "txt":
    case "doc":
    case "docx":
    case "pdf":
      return <FileText className="h-4 w-4 text-[color:var(--color-text-secondary)]" aria-hidden="true" />;
    default:
      return <File className="h-4 w-4 text-[color:var(--color-text-tertiary)]" aria-hidden="true" />;
  }
}

// ──────────────────────────────────────────────
// Git status badge (#46)
// ──────────────────────────────────────────────

function GitStatusBadge({ status }: { status: string }) {
  let letter: string;
  let colorClass: string;
  switch (status) {
    case "modified":
      letter = "M";
      colorClass = "text-orange-500";
      break;
    case "added":
      letter = "A";
      colorClass = "text-green-500";
      break;
    case "deleted":
      letter = "D";
      colorClass = "text-red-500";
      break;
    case "untracked":
      letter = "?";
      colorClass = "text-gray-400";
      break;
    default:
      letter = status.charAt(0).toUpperCase();
      colorClass = "text-gray-400";
      break;
  }
  return (
    <span
      className={`ml-1 text-[10px] font-bold ${colorClass} leading-none`}
      title={`Git: ${status}`}
      aria-label={`Git status: ${status}`}
    >
      {letter}
    </span>
  );
}

// ──────────────────────────────────────────────
// Group files utility (#77)
// ──────────────────────────────────────────────

function groupFiles(files: FileEntryData[], groupBy: string): Record<string, FileEntryData[]> {
  const groups: Record<string, FileEntryData[]> = {};
  for (const file of files) {
    let key: string;
    switch (groupBy) {
      case "type":
        key = file.is_dir ? "Folders" : (file.extension?.toUpperCase() || "Other");
        break;
      case "extension":
        key = file.extension?.toUpperCase() || "No Extension";
        break;
      case "date":
        key = file.modified ? new Date(file.modified).toLocaleDateString() : "Unknown";
        break;
      case "size": {
        if (file.is_dir) key = "Folders";
        else if (file.size < 1024) key = "Tiny (< 1 KB)";
        else if (file.size < 1024 * 1024) key = "Small (< 1 MB)";
        else if (file.size < 100 * 1024 * 1024) key = "Medium (< 100 MB)";
        else key = "Large (100+ MB)";
        break;
      }
      default:
        key = "All";
        break;
    }
    (groups[key] ??= []).push(file);
  }
  return groups;
}

// ──────────────────────────────────────────────
// FileList component
// ──────────────────────────────────────────────

interface FileListProps {
  files: FileEntryData[];
  viewMode: ViewMode;
  className?: string;
  selectedPaths?: string[];
  onSelect?: (path: string, event: React.MouseEvent) => void;
  onOpen?: (entry: FileEntryData) => void;
  onContextMenu?: (entry: FileEntryData, event: React.MouseEvent) => void;
  /** Index of the focused item for keyboard nav */
  focusedIndex?: number;
  onFocusedIndexChange?: (index: number) => void;
  rowHeight?: number;
  /** Git status per file path (#46) */
  gitStatus?: Record<string, string>;
  /** Group files by attribute (#77) */
  groupBy?: string | null;
}

export function FileList({
  files,
  viewMode,
  className,
  selectedPaths = [],
  onSelect,
  onOpen,
  onContextMenu,
  focusedIndex = 0,
  onFocusedIndexChange,
  rowHeight: customRowHeight,
  gitStatus,
  groupBy,
}: FileListProps) {
  const rowHeight = customRowHeight || (viewMode === "compact" ? 28 : viewMode === "grid" ? 120 : 36);

  if (viewMode === "grid") {
    return (
      <FileGridView
        files={files}
        className={className}
        selectedPaths={selectedPaths}
        onSelect={onSelect}
        onOpen={onOpen}
        onContextMenu={onContextMenu}
        focusedIndex={focusedIndex}
        onFocusedIndexChange={onFocusedIndexChange}
      />
    );
  }

  return (
    <FileTableView
      files={files}
      viewMode={viewMode}
      className={className}
      selectedPaths={selectedPaths}
      onSelect={onSelect}
      onOpen={onOpen}
      onContextMenu={onContextMenu}
      focusedIndex={focusedIndex}
      onFocusedIndexChange={onFocusedIndexChange}
      rowHeight={rowHeight}
      gitStatus={gitStatus}
      groupBy={groupBy}
    />
  );
}

// ──────────────────────────────────────────────
// Table-based views (list, detail, compact)
// ──────────────────────────────────────────────

interface FileTableViewProps {
  files: FileEntryData[];
  viewMode: ViewMode;
  className?: string;
  selectedPaths: string[];
  onSelect?: (path: string, event: React.MouseEvent) => void;
  onOpen?: (entry: FileEntryData) => void;
  onContextMenu?: (entry: FileEntryData, event: React.MouseEvent) => void;
  focusedIndex: number;
  onFocusedIndexChange?: (index: number) => void;
  rowHeight: number;
  gitStatus?: Record<string, string>;
  groupBy?: string | null;
}

const ALL_COLUMNS = ["name", "size", "modified", "type", "permissions", "created", "extension"] as const;

function FileTableView({
  files,
  viewMode,
  className,
  selectedPaths,
  onSelect,
  onOpen,
  onContextMenu,
  focusedIndex,
  onFocusedIndexChange,
  rowHeight,
  gitStatus,
  groupBy,
}: FileTableViewProps) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [columnResizeMode] = useState<ColumnResizeMode>("onChange");
  const parentRef = useRef<HTMLDivElement>(null);
  const selectedSet = useMemo(() => new Set(selectedPaths), [selectedPaths]);
  const [visibleColumns, setVisibleColumns] = useState<Set<string>>(new Set(["name", "size", "modified", "type"]));
  const [columnMenuPos, setColumnMenuPos] = useState<{x: number, y: number} | null>(null);
  const columnMenuRef = useRef<HTMLDivElement>(null);

  // Close column menu on click outside
  useEffect(() => {
    if (!columnMenuPos) return;
    function handleClick(e: MouseEvent) {
      if (columnMenuRef.current && !columnMenuRef.current.contains(e.target as Node)) {
        setColumnMenuPos(null);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [columnMenuPos]);

  const handleColumnHeaderContextMenu = useCallback((e: React.MouseEvent) => {
    if (viewMode !== "detail") return;
    e.preventDefault();
    setColumnMenuPos({ x: e.clientX, y: e.clientY });
  }, [viewMode]);

  const toggleColumn = useCallback((colId: string) => {
    if (colId === "name") return; // name is always visible
    setVisibleColumns((prev) => {
      const next = new Set(prev);
      if (next.has(colId)) {
        next.delete(colId);
      } else {
        next.add(colId);
      }
      return next;
    });
  }, []);

  const columns = useMemo<ColumnDef<FileEntryData>[]>(() => {
    const cols: ColumnDef<FileEntryData>[] = [
      {
        accessorKey: "name",
        header: "Name",
        size: 300,
        minSize: 120,
        cell: ({ row }) => {
          const entry = row.original;
          const git = gitStatus?.[entry.path];
          return (
            <div className="flex items-center gap-2 min-w-0">
              {getFileIcon(entry)}
              <span
                className={cn(
                  "truncate",
                  viewMode === "compact"
                    ? "text-[length:var(--font-size-xs)]"
                    : "text-[length:var(--font-size-sm)]",
                  entry.is_dir
                    ? "font-medium text-[color:var(--color-primary)]"
                    : "text-[color:var(--color-text)]",
                  entry.is_hidden && "opacity-60",
                )}
              >
                {entry.name}
              </span>
              {entry.is_symlink && (
                <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
                  (link)
                </span>
              )}
              {git && (viewMode === "detail" || viewMode === "list") && (
                <GitStatusBadge status={git} />
              )}
            </div>
          );
        },
      },
    ];

    if (viewMode === "detail") {
      if (visibleColumns.has("size")) {
        cols.push({
          accessorKey: "size",
          header: "Size",
          size: 100,
          minSize: 60,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
              {row.original.is_dir ? "--" : formatFileSize(row.original.size)}
            </span>
          ),
        });
      }
      if (visibleColumns.has("modified")) {
        cols.push({
          accessorKey: "modified",
          header: "Modified",
          size: 180,
          minSize: 100,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
              {formatDate(row.original.modified)}
            </span>
          ),
        });
      }
      if (visibleColumns.has("type")) {
        cols.push({
          accessorKey: "extension",
          header: "Type",
          size: 80,
          minSize: 50,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-tertiary)] text-[length:var(--font-size-xs)] uppercase">
              {row.original.is_dir ? "Folder" : row.original.extension || "--"}
            </span>
          ),
        });
      }
      if (visibleColumns.has("permissions")) {
        cols.push({
          id: "permissions",
          header: "Permissions",
          accessorFn: (row) => row.permissions || "--",
          size: 100,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-tertiary)] text-[length:var(--font-size-xs)] font-mono">
              {row.original.permissions || "--"}
            </span>
          ),
        });
      }
      if (visibleColumns.has("created")) {
        cols.push({
          id: "created",
          header: "Created",
          accessorFn: (row) => row.created ? formatDate(row.created) : "--",
          size: 180,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
              {row.original.created ? formatDate(row.original.created) : "--"}
            </span>
          ),
        });
      }
      if (visibleColumns.has("extension")) {
        cols.push({
          id: "ext-column",
          header: "Extension",
          accessorFn: (row) => row.extension || "--",
          size: 80,
          cell: ({ row }) => (
            <span className="text-[color:var(--color-text-tertiary)] text-[length:var(--font-size-xs)]">
              {row.original.extension || "--"}
            </span>
          ),
        });
      }
    }

    if (viewMode === "list") {
      cols.push({
        accessorKey: "size",
        header: "Size",
        size: 100,
        minSize: 60,
        cell: ({ row }) => (
          <span className="text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]">
            {row.original.is_dir ? "--" : formatFileSize(row.original.size)}
          </span>
        ),
      });
    }

    return cols;
  }, [viewMode, visibleColumns, gitStatus]);

  const table = useReactTable({
    data: files,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    columnResizeMode,
    enableColumnResizing: true,
  });

  const { rows } = table.getRowModel();

  // Flatten grouped data into a virtualizable list of header + file items
  const GROUP_HEADER_HEIGHT = 28;
  type GroupedItem =
    | { type: "header"; groupName: string; count: number }
    | { type: "file"; entry: FileEntryData };

  const groupedFlatItems = useMemo<GroupedItem[]>(() => {
    if (!groupBy) return [];
    const groups = groupFiles(files, groupBy);
    const items: GroupedItem[] = [];
    for (const [groupName, groupFilesList] of Object.entries(groups)) {
      items.push({ type: "header", groupName, count: groupFilesList.length });
      for (const entry of groupFilesList) {
        items.push({ type: "file", entry });
      }
    }
    return items;
  }, [files, groupBy]);

  const groupedVirtualizer = useVirtualizer({
    count: groupBy ? groupedFlatItems.length : 0,
    getScrollElement: () => parentRef.current,
    estimateSize: (index) => {
      if (!groupBy) return rowHeight;
      const item = groupedFlatItems[index];
      return item?.type === "header" ? GROUP_HEADER_HEIGHT : rowHeight;
    },
    overscan: 15,
    enabled: !!groupBy,
  });

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 15,
    enabled: !groupBy,
  });

  // Keyboard navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const maxIndex = rows.length - 1;
      let newIndex = focusedIndex;

      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          newIndex = Math.min(focusedIndex + 1, maxIndex);
          break;
        case "ArrowUp":
          e.preventDefault();
          newIndex = Math.max(focusedIndex - 1, 0);
          break;
        case "Home":
          e.preventDefault();
          newIndex = 0;
          break;
        case "End":
          e.preventDefault();
          newIndex = maxIndex;
          break;
        case "PageDown": {
          e.preventDefault();
          const pageSize = Math.floor((parentRef.current?.clientHeight || 400) / rowHeight);
          newIndex = Math.min(focusedIndex + pageSize, maxIndex);
          break;
        }
        case "PageUp": {
          e.preventDefault();
          const pageSizeUp = Math.floor((parentRef.current?.clientHeight || 400) / rowHeight);
          newIndex = Math.max(focusedIndex - pageSizeUp, 0);
          break;
        }
        case "Enter": {
          e.preventDefault();
          const row = rows[focusedIndex];
          if (row) onOpen?.(row.original);
          return;
        }
        default:
          return;
      }

      if (newIndex !== focusedIndex) {
        onFocusedIndexChange?.(newIndex);
        virtualizer.scrollToIndex(newIndex, { align: "auto" });
      }
    },
    [focusedIndex, rows, rowHeight, onOpen, onFocusedIndexChange, virtualizer],
  );

  return (
    <div
      className={cn("flex flex-col h-full", className)}
      data-testid="file-list"
    >
      {/* Table Header */}
      <div
        className="flex bg-[var(--color-bg-secondary)] border-b border-[var(--color-border)]"
        role="row"
      >
        {table.getHeaderGroups().map((headerGroup) =>
          headerGroup.headers.map((header) => (
            <div
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
                "flex items-center h-8 px-3",
                "text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] uppercase tracking-wider",
                "select-none relative",
                header.column.getCanSort() && "cursor-pointer",
              )}
              style={{ width: header.getSize() }}
              onClick={header.column.getToggleSortingHandler()}
              onContextMenu={handleColumnHeaderContextMenu}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  header.column.getToggleSortingHandler()?.(e);
                }
              }}
              tabIndex={header.column.getCanSort() ? 0 : undefined}
              data-testid={`column-header-${header.id}`}
            >
              <div className="flex items-center gap-1">
                {flexRender(header.column.columnDef.header, header.getContext())}
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

              {/* Column resize handle */}
              {header.column.getCanResize() && (
                <div
                  onMouseDown={header.getResizeHandler()}
                  onTouchStart={header.getResizeHandler()}
                  className={cn(
                    "absolute right-0 top-0 h-full w-1 cursor-col-resize select-none touch-none",
                    "hover:bg-[var(--color-primary)]",
                    header.column.getIsResizing() && "bg-[var(--color-primary)]",
                  )}
                  role="separator"
                  aria-orientation="vertical"
                />
              )}
            </div>
          )),
        )}
      </div>

      {/* Column visibility context menu */}
      {columnMenuPos && viewMode === "detail" && (
        <div
          ref={columnMenuRef}
          className="fixed z-50 min-w-[180px] bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-[var(--radius-md)] shadow-lg py-1"
          style={{ left: columnMenuPos.x, top: columnMenuPos.y }}
          data-testid="column-menu"
        >
          <div className="px-3 py-1.5 text-[length:var(--font-size-xs)] font-semibold text-[color:var(--color-text-secondary)] uppercase tracking-wider border-b border-[var(--color-border)]">
            Show Columns
          </div>
          {ALL_COLUMNS.map((colId) => {
            const isName = colId === "name";
            const label = colId.charAt(0).toUpperCase() + colId.slice(1);
            return (
              <label
                key={colId}
                className={cn(
                  "flex items-center gap-2 px-3 py-1.5 text-[length:var(--font-size-xs)] cursor-pointer",
                  "hover:bg-[var(--color-hover-bg)]",
                  isName && "opacity-50 cursor-not-allowed",
                )}
              >
                <input
                  type="checkbox"
                  checked={visibleColumns.has(colId)}
                  disabled={isName}
                  onChange={() => toggleColumn(colId)}
                  className="h-3 w-3 rounded border-[var(--color-border)]"
                />
                <span className="text-[color:var(--color-text)]">{label}</span>
              </label>
            );
          })}
        </div>
      )}

      {/* Group headers + Virtualized rows */}
      {groupBy ? (
        <div
          ref={parentRef}
          className={cn(
            "flex-1 overflow-auto",
            "focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-focus-ring)]",
          )}
          role="grid"
          aria-label="File listing"
          aria-rowcount={groupedFlatItems.length}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          data-testid="file-list-body"
        >
          <div
            style={{
              height: `${groupedVirtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {groupedVirtualizer.getVirtualItems().map((virtualRow) => {
              const item = groupedFlatItems[virtualRow.index];

              if (item.type === "header") {
                return (
                  <div
                    key={`group-${item.groupName}`}
                    className="absolute left-0 top-0 w-full px-3 py-1 text-xs font-semibold text-[color:var(--color-text-secondary)] bg-[var(--color-hover-bg)] z-10 border-b border-[var(--color-border)]"
                    style={{
                      height: `${virtualRow.size}px`,
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    {item.groupName} ({item.count})
                  </div>
                );
              }

              const entry = item.entry;
              const isSelected = selectedSet.has(entry.path);

              return (
                <div
                  key={entry.path}
                  role="row"
                  aria-rowindex={virtualRow.index + 1}
                  aria-selected={isSelected}
                  className={cn(
                    "absolute left-0 top-0 w-full flex",
                    "border-b border-[var(--color-border)]",
                    "transition-theme cursor-default",
                    isSelected
                      ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                      : "hover:bg-[var(--color-hover-bg)]",
                  )}
                  style={{
                    height: `${virtualRow.size}px`,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                  onClick={(e) => onSelect?.(entry.path, e)}
                  onDoubleClick={() => onOpen?.(entry)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onContextMenu?.(entry, e);
                  }}
                  data-path={entry.path}
                >
                  <div className="flex items-center gap-2 px-3 min-w-0 flex-1" style={{ height: `${virtualRow.size}px` }}>
                    {getFileIcon(entry)}
                    <span className={cn(
                      "truncate text-[length:var(--font-size-sm)]",
                      entry.is_dir ? "font-medium text-[color:var(--color-primary)]" : "text-[color:var(--color-text)]",
                      entry.is_hidden && "opacity-60",
                    )}>
                      {entry.name}
                    </span>
                    {gitStatus?.[entry.path] && <GitStatusBadge status={gitStatus[entry.path]} />}
                  </div>
                  {viewMode === "detail" && (
                    <>
                      <div className="flex items-center px-3 text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]" style={{ width: 100 }}>
                        {entry.is_dir ? "--" : formatFileSize(entry.size)}
                      </div>
                      <div className="flex items-center px-3 text-[color:var(--color-text-secondary)] text-[length:var(--font-size-sm)]" style={{ width: 180 }}>
                        {formatDate(entry.modified)}
                      </div>
                    </>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      ) : (
        <div
          ref={parentRef}
          className={cn(
            "flex-1 overflow-auto",
            "focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-focus-ring)]",
          )}
          role="grid"
          aria-label="File listing"
          aria-rowcount={rows.length}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          data-testid="file-list-body"
        >
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const row = rows[virtualRow.index];
              const isSelected = selectedSet.has(row.original.path);
              const isFocused = virtualRow.index === focusedIndex;

              return (
                <div
                  key={row.id}
                  role="row"
                  aria-rowindex={virtualRow.index + 1}
                  aria-selected={isSelected}
                  className={cn(
                    "absolute left-0 top-0 w-full flex",
                    "border-b border-[var(--color-border)]",
                    "transition-theme cursor-default",
                    isSelected
                      ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                      : "hover:bg-[var(--color-hover-bg)]",
                    isFocused && !isSelected && "bg-[var(--color-hover-bg)]",
                    isFocused && "ring-1 ring-inset ring-[var(--color-focus-ring)]",
                  )}
                  style={{
                    height: `${virtualRow.size}px`,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                  onClick={(e) => onSelect?.(row.original.path, e)}
                  onDoubleClick={() => onOpen?.(row.original)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onContextMenu?.(row.original, e);
                  }}
                  data-testid={`file-row-${virtualRow.index}`}
                  data-path={row.original.path}
                >
                  {row.getVisibleCells().map((cell) => (
                    <div
                      key={cell.id}
                      role="gridcell"
                      className="flex items-center px-3"
                      style={{
                        width: cell.column.getSize(),
                        height: `${virtualRow.size}px`,
                      }}
                    >
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Grid view
// ──────────────────────────────────────────────

interface FileGridViewProps {
  files: FileEntryData[];
  className?: string;
  selectedPaths: string[];
  onSelect?: (path: string, event: React.MouseEvent) => void;
  onOpen?: (entry: FileEntryData) => void;
  onContextMenu?: (entry: FileEntryData, event: React.MouseEvent) => void;
  focusedIndex: number;
  onFocusedIndexChange?: (index: number) => void;
}

function FileGridView({
  files,
  className,
  selectedPaths,
  onSelect,
  onOpen,
  onContextMenu,
  focusedIndex,
  onFocusedIndexChange,
}: FileGridViewProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const selectedSet = useMemo(() => new Set(selectedPaths), [selectedPaths]);

  // Grid: estimate columns based on item width
  const ITEM_WIDTH = 96;
  const ITEM_HEIGHT = 110;
  const GAP = 8; // gap-2 = 0.5rem = 8px

  // Track container width for column count calculation
  const [containerWidth, setContainerWidth] = useState(0);

  useEffect(() => {
    const container = parentRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerWidth(entry.contentRect.width);
      }
    });
    observer.observe(container);
    // Initialize with current width
    setContainerWidth(container.clientWidth);
    return () => observer.disconnect();
  }, []);

  const columnCount = useMemo(
    () => Math.max(1, Math.floor((containerWidth + GAP) / (ITEM_WIDTH + GAP))),
    [containerWidth],
  );

  const rowCount = useMemo(
    () => Math.ceil(files.length / columnCount),
    [files.length, columnCount],
  );

  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ITEM_HEIGHT + GAP,
    overscan: 5,
  });

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const cols = columnCount;
      const maxIndex = files.length - 1;
      let newIndex = focusedIndex;

      switch (e.key) {
        case "ArrowRight":
          e.preventDefault();
          newIndex = Math.min(focusedIndex + 1, maxIndex);
          break;
        case "ArrowLeft":
          e.preventDefault();
          newIndex = Math.max(focusedIndex - 1, 0);
          break;
        case "ArrowDown":
          e.preventDefault();
          newIndex = Math.min(focusedIndex + cols, maxIndex);
          break;
        case "ArrowUp":
          e.preventDefault();
          newIndex = Math.max(focusedIndex - cols, 0);
          break;
        case "Home":
          e.preventDefault();
          newIndex = 0;
          break;
        case "End":
          e.preventDefault();
          newIndex = maxIndex;
          break;
        case "Enter": {
          e.preventDefault();
          const entry = files[focusedIndex];
          if (entry) onOpen?.(entry);
          return;
        }
        default:
          return;
      }

      if (newIndex !== focusedIndex) {
        onFocusedIndexChange?.(newIndex);
        // Scroll the row containing the new index into view
        const targetRow = Math.floor(newIndex / cols);
        rowVirtualizer.scrollToIndex(targetRow, { align: "auto" });
      }
    },
    [focusedIndex, files, columnCount, onOpen, onFocusedIndexChange, rowVirtualizer],
  );

  return (
    <div
      ref={parentRef}
      className={cn(
        "flex-1 overflow-auto p-3",
        "focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--color-focus-ring)]",
        className,
      )}
      role="grid"
      aria-label="File grid"
      aria-rowcount={rowCount}
      tabIndex={0}
      onKeyDown={handleKeyDown}
      data-testid="file-grid"
    >
      <div
        style={{
          height: `${rowVirtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {rowVirtualizer.getVirtualItems().map((virtualRow) => {
          const startIndex = virtualRow.index * columnCount;
          const endIndex = Math.min(startIndex + columnCount, files.length);
          const rowItems = files.slice(startIndex, endIndex);

          return (
            <div
              key={virtualRow.key}
              role="row"
              aria-rowindex={virtualRow.index + 1}
              className="absolute left-0 top-0 w-full flex gap-2"
              style={{
                height: `${virtualRow.size}px`,
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              {rowItems.map((entry, colIndex) => {
                const globalIndex = startIndex + colIndex;
                const isSelected = selectedSet.has(entry.path);
                const isFocused = globalIndex === focusedIndex;

                return (
                  <div
                    key={entry.path}
                    role="gridcell"
                    aria-selected={isSelected}
                    className={cn(
                      "flex flex-col items-center justify-center p-2 rounded-[var(--radius-md)]",
                      "cursor-default select-none transition-theme",
                      isSelected
                        ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                        : "hover:bg-[var(--color-hover-bg)]",
                      isFocused && "ring-2 ring-[var(--color-focus-ring)]",
                    )}
                    style={{ width: ITEM_WIDTH, height: ITEM_HEIGHT }}
                    onClick={(e) => onSelect?.(entry.path, e)}
                    onDoubleClick={() => onOpen?.(entry)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      onContextMenu?.(entry, e);
                    }}
                    data-testid={`file-grid-item-${globalIndex}`}
                  >
                    <div className="flex items-center justify-center h-12 w-12">
                      {entry.is_dir ? (
                        <Folder className="h-10 w-10 text-[color:var(--color-primary)]" aria-hidden="true" />
                      ) : (
                        <File className="h-10 w-10 text-[color:var(--color-text-secondary)]" aria-hidden="true" />
                      )}
                    </div>
                    <span
                      className={cn(
                        "text-[length:var(--font-size-xs)] text-center leading-tight mt-1",
                        "w-full truncate px-1",
                        entry.is_dir
                          ? "font-medium text-[color:var(--color-primary)]"
                          : "text-[color:var(--color-text)]",
                      )}
                      title={entry.name}
                    >
                      {entry.name}
                    </span>
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
