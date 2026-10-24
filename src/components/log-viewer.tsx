/**
 * In-app log viewer (Advanced): the app's own log files, newest at the
 * bottom, filterable by level, module, time and text, with a live tail,
 * older pages on demand, and copy of the selected lines. Rows are
 * virtualised, so a day with hundreds of thousands of lines stays quick.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { copyToClipboardWithToast } from "@/lib/clipboard";
import { errorText } from "@/lib/error-text";
import { VirtualRows } from "@/components/virtual-list";
import {
  LEVEL_OPTIONS,
  LEVEL_TONE,
  NO_FILTERS,
  clockTime,
  isFiltered,
  logQuery,
  mergeTail,
  oldestFirst,
  recordText,
  type LogFilters,
  type LogPage,
  type LogRecord,
} from "@/lib/log-viewer";

const PAGE = 500;
const TAIL_MS = 1500;
const field = "text-xs px-2 py-1 rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-900";
const btn = "text-xs px-2 py-1 rounded border border-zinc-300 dark:border-zinc-600 hover:bg-zinc-100 dark:hover:bg-zinc-800 disabled:opacity-50";

export function LogViewer({ onClose, listHeight = 420, tailMs = TAIL_MS }: { onClose?: () => void; listHeight?: number; tailMs?: number }) {
  const [filters, setFilters] = useState<LogFilters>(NO_FILTERS);
  const [settledText, setSettledText] = useState("");
  const [records, setRecords] = useState<LogRecord[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [sources, setSources] = useState<string[]>([]);
  const [directory, setDirectory] = useState("");
  const [scanLimited, setScanLimited] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState(false);
  const [selected, setSelected] = useState<number[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  // Later answers win: a slow older request must not overwrite a newer one.
  const generation = useRef(0);
  const recordsRef = useRef<LogRecord[]>([]);
  recordsRef.current = records;

  // Typing in the text box queries once it pauses.
  useEffect(() => {
    const t = setTimeout(() => setSettledText(filters.text), 300);
    return () => clearTimeout(t);
  }, [filters.text]);
  // Only the filters that changed what the query asks for re-run it (the text box waits for a pause).
  const { level, source, since, until } = filters;
  const effective = useMemo<LogFilters>(() => ({ level, source, since, until, text: settledText }), [level, source, since, until, settledText]);

  const load = useCallback(async () => {
    const mine = ++generation.current;
    setLoading(true);
    try {
      const page = await tauriInvoke<LogPage>("query_logs", logQuery(effective, { limit: PAGE }));
      if (mine !== generation.current) return;
      setRecords(oldestFirst(page.records));
      setCursor(page.next_cursor);
      setSources(page.sources);
      setDirectory(page.directory);
      setScanLimited(page.scan_limited);
      setError(null);
      setSelected([]);
    } catch (e) {
      if (mine === generation.current) setError(errorText(e));
    } finally {
      if (mine === generation.current) setLoading(false);
    }
  }, [effective]);

  useEffect(() => {
    void load();
  }, [load]);

  const loadOlder = async () => {
    if (!cursor) return;
    const mine = ++generation.current;
    setLoading(true);
    try {
      const page = await tauriInvoke<LogPage>("query_logs", logQuery(effective, { limit: PAGE, cursor }));
      if (mine !== generation.current) return;
      setRecords((prev) => [...oldestFirst(page.records), ...prev]);
      setCursor(page.next_cursor);
      setScanLimited(page.scan_limited);
      setError(null);
    } catch (e) {
      if (mine === generation.current) setError(errorText(e));
    } finally {
      if (mine === generation.current) setLoading(false);
    }
  };

  // Live tail: ask again from the newest line's time; lines already shown are dropped by id.
  // A fixed end time has nothing newer to follow.
  const canTail = live && !filters.until.trim();
  useEffect(() => {
    if (!canTail) return;
    const timer = setInterval(async () => {
      const newest = recordsRef.current.at(-1);
      const mine = generation.current;
      try {
        const page = await tauriInvoke<LogPage>("query_logs", logQuery(effective, { limit: PAGE, ...(newest ? { since: newest.ts } : {}) }));
        if (mine !== generation.current) return;
        setRecords((prev) => mergeTail(prev, page.records));
        setError(null);
      } catch (e) {
        setError(errorText(e));
      }
    }, tailMs);
    return () => clearInterval(timer);
  }, [canTail, effective, tailMs]);

  const set = (patch: Partial<LogFilters>) => setFilters((f) => ({ ...f, ...patch }));
  const filtered = isFiltered(effective);

  const copySelected = async (indexes: number[] = selected) => {
    if (indexes.length === 0) return;
    await copyToClipboardWithToast(indexes.map((i) => recordText(records[i])).join("\n"), `Copy ${indexes.length} log line${indexes.length === 1 ? "" : "s"}`);
  };
  const openFolder = async () => {
    setNotice(null);
    try {
      await tauriInvoke("reveal_in_os", { path: directory });
    } catch (e) {
      setNotice(errorText(e));
    }
  };

  const single = selected.length === 1 ? records[selected[0]] : null;

  return (
    <div className="flex flex-col gap-2 p-3 text-zinc-900 dark:text-zinc-100" data-testid="log-viewer">
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold">Log viewer</h2>
        <span className="text-xs text-zinc-500" aria-live="polite">
          {records.length.toLocaleString()} line{records.length === 1 ? "" : "s"} shown{filtered ? " (filtered)" : ""}
        </span>
        {onClose && (
          <button type="button" className={`${btn} ml-auto`} onClick={onClose}>
            Close
          </button>
        )}
      </div>

      <div className="flex flex-wrap items-end gap-2">
        <select className={field} value={filters.level} onChange={(e) => set({ level: e.target.value })} aria-label="Level">
          {LEVEL_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <input className={`${field} w-44`} list="log-sources" value={filters.source} placeholder="Source (module)" aria-label="Source" onChange={(e) => set({ source: e.target.value })} />
        <datalist id="log-sources">
          {sources.map((s) => (
            <option key={s} value={s} />
          ))}
        </datalist>
        <input className={`${field} w-52`} value={filters.text} placeholder="Search the text…" aria-label="Search text" onChange={(e) => set({ text: e.target.value })} />
        <label className="text-xs flex items-center gap-1">
          From
          <input type="datetime-local" className={field} value={filters.since} aria-label="From time" onChange={(e) => set({ since: e.target.value })} />
        </label>
        <label className="text-xs flex items-center gap-1">
          To
          <input type="datetime-local" className={field} value={filters.until} aria-label="To time" onChange={(e) => set({ until: e.target.value })} />
        </label>
        {filtered && (
          <button type="button" className={btn} onClick={() => setFilters(NO_FILTERS)}>
            Clear filters
          </button>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <label className="text-xs flex items-center gap-1">
          <input type="checkbox" checked={live} onChange={(e) => setLive(e.target.checked)} aria-label="Live tail" />
          Live tail
          {live && filters.until.trim() && <span className="text-amber-600">(off while an end time is set)</span>}
        </label>
        <button type="button" className={btn} onClick={() => void load()} disabled={loading}>
          Refresh
        </button>
        <button type="button" className={btn} onClick={() => void copySelected()} disabled={selected.length === 0}>
          Copy selected{selected.length > 0 ? ` (${selected.length})` : ""}
        </button>
        {directory && (
          <span className="ml-auto flex items-center gap-2 text-xs text-zinc-500">
            <span className="font-mono truncate max-w-[22rem]" title={directory}>
              {directory}
            </span>
            <button type="button" className={btn} onClick={() => void openFolder()}>
              Open folder
            </button>
          </span>
        )}
      </div>

      {error && (
        <div className="text-xs text-red-600" role="alert">
          {error}
        </div>
      )}
      {notice && (
        <div className="text-xs text-red-600" role="alert">
          {notice}
        </div>
      )}

      {cursor && (
        <div className="flex items-center gap-2 text-xs">
          <button type="button" className={btn} onClick={() => void loadOlder()} disabled={loading}>
            Load older lines
          </button>
          {scanLimited && <span className="text-amber-600">Stopped searching after a large amount of log; load more to keep looking further back.</span>}
        </div>
      )}

      {records.length === 0 && !loading && !error ? (
        <div className="text-xs text-zinc-500 py-6 text-center" data-testid="log-empty">
          {filtered ? "No log lines match the filters." : "There are no log lines yet."}
        </div>
      ) : (
        <VirtualRows<LogRecord>
          items={records}
          ariaLabel="Log lines"
          height={listHeight}
          rowHeight={20}
          selectable
          followTail={live}
          rowKey={(r) => r.id}
          onSelectionChange={setSelected}
          onCopy={(idx) => void copySelected(idx)}
          renderRow={(r) => (
            <>
              <span className="w-24 shrink-0 text-[11px] font-mono text-zinc-500" title={r.ts}>
                {clockTime(r.ts)}
              </span>
              <span className={`w-12 shrink-0 text-[11px] font-semibold ${LEVEL_TONE[r.level] ?? ""}`}>{r.level}</span>
              <span className="w-40 shrink-0 truncate text-[11px] text-zinc-500" title={r.target}>
                {r.target.split("::").slice(-2).join("::")}
              </span>
              <span className="truncate flex-1 text-[11px] font-mono" title={r.message}>
                {r.message.split("\n")[0]}
              </span>
            </>
          )}
        />
      )}

      {single && (
        <pre className="max-h-40 overflow-auto text-[11px] font-mono p-2 rounded bg-zinc-50 dark:bg-zinc-800 whitespace-pre-wrap" data-testid="log-detail">
          {recordText(single)}
        </pre>
      )}
    </div>
  );
}
