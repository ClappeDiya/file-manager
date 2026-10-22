'use client';

/**
 * A table over a paged list route (the `lib/pagination` convention): the
 * server filters and pages; the viewer picks visible columns and page size,
 * remembered per table in this browser.
 */
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Columns3 } from 'lucide-react';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import type { PageInfo } from '@/lib/pagination';

export interface Column<T> {
  key: string;
  label: string;
  render: (row: T) => React.ReactNode;
  /** Hidden until the viewer turns it on. */
  defaultHidden?: boolean;
  /** Always shown (no checkbox). */
  required?: boolean;
  className?: string;
}

const PAGE_SIZES = [25, 50, 100, 200];

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? (JSON.parse(v) as T) : fallback;
  } catch {
    return fallback;
  }
}
function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // private mode / blocked storage: the choice lasts for this page only
  }
}

/**
 * Fetch one page of `path` with `filters`; resets to the first page when the
 * filters change.
 */
export function useServerTable<R>(id: string, path: string, listKey: string, filters: Record<string, string>) {
  const [limit, setLimitState] = useState(() => load(`ufop.table.${id}.limit`, 50));
  const [offset, setOffset] = useState(0);
  const filterKey = JSON.stringify(filters);
  useEffect(() => setOffset(0), [filterKey]);
  const q = new URLSearchParams({ ...Object.fromEntries(Object.entries(filters).filter(([, v]) => v)), limit: String(limit), offset: String(offset) });
  const res = useApi<Record<string, unknown> & { pagination?: PageInfo }>(`${path}${path.includes('?') ? '&' : '?'}${q}`);
  const setLimit = (n: number) => {
    setLimitState(n);
    setOffset(0);
    save(`ufop.table.${id}.limit`, n);
  };
  return { rows: (res.data?.[listKey] as R[] | undefined) ?? [], pagination: res.data?.pagination ?? null, data: res.data, loading: res.loading, error: res.error, reload: res.reload, limit, setLimit, offset, setOffset };
}

interface Props<T> {
  id: string;
  columns: Column<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  loading?: boolean;
  error?: string | null;
  empty?: string;
  pagination?: PageInfo | null;
  limit?: number;
  onLimit?: (n: number) => void;
  onOffset?: (offset: number) => void;
}

export function DataTable<T>({ id, columns, rows, rowKey, loading, error, empty = 'Nothing to show', pagination, limit, onLimit, onOffset }: Props<T>) {
  const defaults = useMemo(() => columns.filter((c) => !c.defaultHidden).map((c) => c.key), [columns]);
  const [visible, setVisible] = useState<string[]>(() => load(`ufop.table.${id}.columns`, defaults));
  const [picker, setPicker] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!picker) return;
    const close = (e: MouseEvent) => {
      if (!pickerRef.current?.contains(e.target as Node)) setPicker(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [picker]);
  const shown = columns.filter((c) => c.required || visible.includes(c.key));
  const toggle = (key: string) => {
    const next = visible.includes(key) ? visible.filter((k) => k !== key) : [...visible, key];
    setVisible(next);
    save(`ufop.table.${id}.columns`, next);
  };
  const from = pagination ? pagination.offset + 1 : 1;
  const to = pagination ? pagination.offset + rows.length : rows.length;

  return (
    <div>
      <ErrorBanner message={error ?? null} />
      <div className="mb-2 flex items-center justify-end gap-2">
        <div className="relative" ref={pickerRef}>
          <Button size="sm" variant="outline" aria-expanded={picker} aria-haspopup="true" onClick={() => setPicker(!picker)}>
            <Columns3 size={14} aria-hidden="true" /> Columns
          </Button>
          {picker && (
            <div role="menu" className="absolute right-0 z-20 mt-1 w-56 rounded-md border border-border bg-background p-2 shadow-lg">
              {columns
                .filter((c) => !c.required)
                .map((c) => (
                  <label key={c.key} className="flex items-center gap-2 py-0.5 text-sm">
                    <input type="checkbox" checked={visible.includes(c.key)} onChange={() => toggle(c.key)} />
                    {c.label}
                  </label>
                ))}
              <button
                className="mt-1 text-xs underline"
                onClick={() => {
                  setVisible(defaults);
                  save(`ufop.table.${id}.columns`, defaults);
                }}
              >
                Reset
              </button>
            </div>
          )}
        </div>
      </div>
      {loading && !rows.length ? (
        <Loading />
      ) : (
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                {shown.map((c) => (
                  <TableHead key={c.key} className={c.className}>
                    {c.label}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((r) => (
                <TableRow key={rowKey(r)}>
                  {shown.map((c) => (
                    <TableCell key={c.key} className={c.className}>
                      {c.render(r)}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
              {!rows.length && (
                <TableRow>
                  <TableCell colSpan={shown.length} className="py-8 text-center text-foreground-secondary">
                    {empty}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      )}
      {pagination && onOffset && (
        <div className="mt-3 flex flex-wrap items-center gap-2 text-sm">
          <Button size="sm" variant="outline" disabled={pagination.offset === 0} onClick={() => onOffset(Math.max(0, pagination.offset - (limit ?? rows.length)))}>
            Previous
          </Button>
          <span>
            {rows.length ? `${from}–${to}` : '0'}
            {pagination.total !== null ? ` of ${pagination.total}` : ''}
          </span>
          <Button size="sm" variant="outline" disabled={!pagination.hasMore} onClick={() => onOffset(pagination.offset + rows.length)}>
            Next
          </Button>
          {onLimit && (
            <label className="ml-auto flex items-center gap-1 text-xs">
              Rows
              <select className="rounded border border-border bg-background px-1 py-0.5" value={limit} onChange={(e) => onLimit(Number(e.target.value))}>
                {PAGE_SIZES.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
      )}
    </div>
  );
}
