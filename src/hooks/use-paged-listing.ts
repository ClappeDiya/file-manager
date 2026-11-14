import { useCallback, useEffect, useMemo, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import type { GridQuery } from "@/components/data-grid";
import type { TableView } from "@/stores/ui-store";

/** One page of a backend list (Rust `paging::Page`). */
export interface Paged<T> {
  items: T[];
  total: number;
  offset: number;
  limit: number;
}

/** The `page` argument of paged IPC calls (Rust `paging::PageRequest`). */
export interface PageRequest {
  offset: number;
  limit: number | null;
  sort: string | null;
  descending: boolean;
  filter: string | null;
}

export function pageRequest(q: GridQuery, filter: string): PageRequest {
  return { offset: q.offset, limit: q.limit, sort: q.sort?.id ?? null, descending: q.sort?.desc ?? false, filter: filter.trim() || null };
}

/** The first query of a table, from its remembered view. */
export function initialQuery(view: TableView): GridQuery {
  return { offset: 0, limit: view.pageSize === "all" ? null : view.pageSize, sort: view.sort };
}

const sameQuery = (a: GridQuery | null, b: GridQuery) =>
  !!a && a.offset === b.offset && a.limit === b.limit && a.sort?.id === b.sort?.id && a.sort?.desc === b.sort?.desc;

/**
 * Loads a backend list one page at a time (K122). `args === null` skips
 * loading. When the backend answers a plain array (a backend without paging),
 * the rows are filtered here and the grid pages them itself (`paged: false`).
 */
export function usePagedListing<T>(command: string, args: Record<string, unknown> | null, opts: { view: TableView; filter: string; textOf: (row: T) => string }) {
  const [query, setQuery] = useState<GridQuery>(() => initialQuery(opts.view));
  const [state, setState] = useState<{ rows: T[]; total: number; paged: boolean }>({ rows: [], total: 0, paged: true });
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [tick, setTick] = useState(0);
  const argsKey = args === null ? null : JSON.stringify(args);
  const { filter, textOf } = opts;

  useEffect(() => {
    if (argsKey === null) return;
    let live = true;
    setLoading(true);
    tauriInvoke<T[] | Paged<T>>(command, { ...JSON.parse(argsKey), page: pageRequest(query, filter) }, [])
      .then((r) => {
        if (!live) return;
        setError(null);
        if (Array.isArray(r)) {
          const f = filter.trim().toLowerCase();
          const rows = f ? r.filter((x) => textOf(x).toLowerCase().includes(f)) : r;
          setState({ rows, total: rows.length, paged: false });
        } else {
          setState({ rows: r.items, total: r.total, paged: true });
        }
      })
      .catch((e) => live && setError(errorText(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
    // textOf is a row accessor; changing it alone does not need a reload.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [command, argsKey, query, filter, tick]);

  const onQueryChange = useCallback((q: GridQuery) => setQuery((prev) => (sameQuery(prev, q) ? prev : q)), []);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  const server = useMemo(() => (state.paged ? { total: state.total, onQueryChange } : undefined), [state.paged, state.total, onQueryChange]);
  return { rows: state.rows, total: state.total, server, error, loading, reload };
}
