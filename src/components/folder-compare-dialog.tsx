/**
 * Folder compare window — compares two folders (local folders and folders
 * on saved connections, in any combination) recursively and lets the user
 * reconcile them: copy selected differences either way, or delete them on
 * one side.
 *
 * - Standard: options (compare by, depth, include/exclude patterns), the
 *   five status chips (OR-combined filters with counts), the virtualised
 *   tree (folders roll up what is inside them), checkbox + shift-range
 *   selection and the four actions.
 * - Advanced adds "next difference", "largest difference by size / by
 *   file count" and sorting by size difference.
 *
 * Every action is planned by the backend (which never copies entries that
 * are already the same), assessed by the Safety Interlock, run through the
 * transfer engine / connectors, recorded in the ledger (Undo for new
 * copies), and only the entries it touched are re-compared.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowLeft, ArrowRight, ChevronDown, ChevronRight, Trash2, X } from "lucide-react";
import { isTauriAvailable, tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { assessBeforeExecute } from "@/lib/safety";
import type { CompareMode } from "@/lib/copy-options";
import {
  COMPARE_PROGRESS_EVENT,
  COMPARE_STATUSES,
  DEFAULT_COMPARE_OPTIONS,
  STATUS_LABEL,
  ancestorsOf,
  buildTree,
  largestDifference,
  mergeOutcome,
  nextDifferencePath,
  parsePatterns,
  rangeBetween,
  selectionRoots,
  sideLabel,
  totalsCount,
  visibleRows,
  type ApplyOutcome,
  type CompareAction,
  type CompareActionPlan,
  type CompareOptions,
  type CompareProgress,
  type CompareResult,
  type CompareSort,
  type CompareStatus,
  type TreeNode,
} from "@/lib/folder-compare";
import { useCompareStore, type CompareRequest } from "@/stores/compare-store";
import type { FilterRule } from "@/stores/backup-store";
import { FilterPresetPicker, FilterRulesEditor, FilterRulesSummary, ImportRulesButton, appendRules, useFilterGroups } from "./filter-rules-editor";
import { useUIStore } from "@/stores/ui-store";

const ROW_HEIGHT = 26;

const STATUS_STYLE: Record<CompareStatus, string> = {
  left_only: "text-green-700 dark:text-green-400",
  right_only: "text-violet-700 dark:text-violet-400",
  different: "text-blue-700 dark:text-blue-400",
  same: "text-zinc-500",
  error: "text-red-700 dark:text-red-400",
};

const ACTION_LABEL: Record<CompareAction, string> = {
  copy_to_right: "Copy → right",
  copy_to_left: "Copy ← left",
  delete_left: "Delete on left",
  delete_right: "Delete on right",
};

const isDelete = (a: CompareAction) => a === "delete_left" || a === "delete_right";

let taskCounter = 0;
const newTaskId = () => `cmp-${Date.now().toString(36)}-${++taskCounter}`;

/** Global host: renders the window while a comparison is requested. */
export function FolderCompareDialog() {
  const request = useCompareStore((s) => s.request);
  const close = useCompareStore((s) => s.close);
  if (!request) return null;
  return <FolderCompareWindow key={request.nonce} request={request} onClose={close} />;
}

interface Message {
  tone: "ok" | "info" | "error";
  text: string;
  details?: string[];
  undoId?: string | null;
}

function describeSkips(plan: CompareActionPlan["plan"]): string {
  const same = plan.skipped.filter((s) => s.reason.startsWith("Already the same")).length;
  if (plan.skipped.length === 0) return "Nothing to do for this selection.";
  return same === plan.skipped.length
    ? `Nothing to do: ${same} selected file${same === 1 ? " is" : "s are"} already the same on both sides.`
    : `Nothing to do: ${plan.skipped.length} selected entr${plan.skipped.length === 1 ? "y was" : "ies were"} skipped.`;
}

function describeOutcome(o: ApplyOutcome): Message {
  const parts = [`${o.done.length} done`];
  if (o.failed.length) parts.push(`${o.failed.length} failed`);
  if (o.skipped.length) parts.push(`${o.skipped.length} skipped`);
  if (o.cancelled) parts.push("stopped early");
  const details = [...o.failed.map((f) => `${f.path}: ${f.error}`), ...o.notes];
  return { tone: o.failed.length ? "error" : "ok", text: `${ACTION_LABEL[o.action]}: ${parts.join(", ")}.`, details, undoId: o.undo_id };
}

function SideCell({ present, size, modified, isDir }: { present: boolean; size: number | null; modified: string | null; isDir: boolean }) {
  if (!present) return <span className="text-zinc-300 dark:text-zinc-600">—</span>;
  return (
    <span className="flex min-w-0 items-baseline justify-end gap-2 tabular-nums text-zinc-500">
      {!isDir && size !== null && <span>{formatBytes(size)}</span>}
      {modified && <span className="hidden truncate lg:inline">{new Date(modified).toLocaleString()}</span>}
    </span>
  );
}

function RollUp({ node }: { node: TreeNode }) {
  const parts = (["different", "left_only", "right_only", "error"] as CompareStatus[])
    .filter((s) => node.counts[s] > 0)
    .map((s) => (
      <span key={s} className={STATUS_STYLE[s]} title={`${node.counts[s]} ${STATUS_LABEL[s].toLowerCase()} below`}>
        {node.counts[s]} {STATUS_LABEL[s].toLowerCase()}
      </span>
    ));
  if (parts.length === 0) return null;
  return (
    <span className="ml-2 flex shrink-0 gap-1.5 text-[10px]" data-testid={`compare-rollup-${node.entry.path}`}>
      {parts}
    </span>
  );
}

function FolderCompareWindow({ request, onClose }: { request: CompareRequest; onClose: () => void }) {
  const isAdvanced = useUIStore((s) => s.appMode === "advanced");
  const anyRemote = request.left.type === "remote" || request.right.type === "remote";
  const [options, setOptions] = useState<CompareOptions>({ ...DEFAULT_COMPARE_OPTIONS, time_tolerance: anyRemote });
  const [include, setInclude] = useState("");
  const [exclude, setExclude] = useState("");
  const [result, setResult] = useState<CompareResult | null>(null);
  const [running, setRunning] = useState<{ id: string; kind: "compare" | "apply" } | null>(null);
  const [progress, setProgress] = useState<CompareProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Set<CompareStatus>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [anchor, setAnchor] = useState<string | null>(null);
  const [focused, setFocused] = useState<string | null>(null);
  const [sort, setSort] = useState<CompareSort>("name");
  const [permanent, setPermanent] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<{ action: CompareAction; planned: CompareActionPlan; paths: string[] } | null>(null);
  const [message, setMessage] = useState<Message | null>(null);
  const [showRules, setShowRules] = useState(false);
  const groups = useFilterGroups();
  const rules: FilterRule[] = options.filter.rules ?? [];
  const setRules = (next: FilterRule[]) => setOptions((o) => ({ ...o, filter: { ...o.filter, rules: next } }));
  const taskRef = useRef<string | null>(null);
  const resultRef = useRef<CompareResult | null>(null);
  resultRef.current = result;
  const runningRef = useRef(running);
  runningRef.current = running;

  // Progress events of this window's task.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<CompareProgress>(COMPARE_PROGRESS_EVENT, (e) => {
          if (e.payload.task_id === taskRef.current) setProgress(e.payload);
        }),
      )
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const runCompare = useCallback(async () => {
    const previous = resultRef.current;
    if (previous) void tauriInvokeSafe("compare_release", { taskId: previous.task_id }, false);
    const taskId = newTaskId();
    taskRef.current = taskId;
    setRunning({ id: taskId, kind: "compare" });
    setError(null);
    setProgress(null);
    setMessage(null);
    setPendingDelete(null);
    const opts: CompareOptions = {
      ...options,
      filter: { ...options.filter, include_patterns: parsePatterns(include), exclude_patterns: parsePatterns(exclude) },
    };
    try {
      const r = await tauriInvoke<CompareResult>("compare_folders", { left: request.left, right: request.right, options: opts, taskId });
      setResult(r);
      setSelected(new Set());
      setAnchor(null);
      setFocused(null);
      // Folders that match are collapsed; the differences are open.
      setCollapsed(new Set(r.entries.filter((e) => e.is_dir && e.status === "same").map((e) => e.path)));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setRunning(null);
    }
  }, [options, include, exclude, request.left, request.right]);

  // Compare as soon as the window opens.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void runCompare();
  }, [runCompare]);

  const cancelRunning = useCallback(() => {
    const r = runningRef.current;
    if (r) void tauriInvokeSafe("compare_cancel", { taskId: r.id }, false);
  }, []);

  const handleClose = useCallback(() => {
    cancelRunning();
    const r = resultRef.current;
    if (r) void tauriInvokeSafe("compare_release", { taskId: r.task_id }, false);
    onClose();
  }, [cancelRunning, onClose]);

  const containerRef = useFocusTrap<HTMLDivElement>({ active: true, onEscape: handleClose });

  const tree = useMemo(() => (result ? buildTree(result.entries) : null), [result]);
  const rows = useMemo(() => (tree ? visibleRows(tree, { statuses, collapsed, sort }) : []), [tree, statuses, collapsed, sort]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
    initialRect: { width: 900, height: 420 },
  });

  // Bring the focused row (navigation) into view once it is shown.
  useEffect(() => {
    if (!focused) return;
    const i = rows.findIndex((r) => r.path === focused);
    if (i >= 0) virtualizer.scrollToIndex(i, { align: "center" });
  }, [focused, rows, virtualizer]);

  const jumpTo = useCallback((path: string | null) => {
    if (!path) {
      setMessage({ tone: "info", text: "No differences to jump to." });
      return;
    }
    setCollapsed((c) => {
      const next = new Set(c);
      for (const a of ancestorsOf(path)) next.delete(a);
      return next;
    });
    setFocused(path);
  }, []);

  const toggleSelect = useCallback(
    (path: string, shift: boolean) => {
      setSelected((sel) => {
        const next = new Set(sel);
        if (shift && anchor !== null) {
          const on = sel.has(anchor);
          for (const p of rangeBetween(rows, anchor, path)) {
            if (on) next.add(p);
            else next.delete(p);
          }
        } else if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }
        return next;
      });
      if (!shift || anchor === null) setAnchor(path);
    },
    [anchor, rows],
  );

  const toggleCollapse = (path: string) =>
    setCollapsed((c) => {
      const next = new Set(c);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const toggleStatus = (s: CompareStatus) =>
    setStatuses((cur) => {
      const next = new Set(cur);
      if (next.has(s)) next.delete(s);
      else next.add(s);
      return next;
    });

  const execute = useCallback(
    async (action: CompareAction, planned: CompareActionPlan, paths: string[]) => {
      const current = resultRef.current;
      if (!current) return;
      const applyId = `${current.task_id}:apply:${Date.now().toString(36)}`;
      taskRef.current = current.task_id;
      setRunning({ id: applyId, kind: "apply" });
      setProgress(null);
      setMessage(null);
      try {
        const outcome = await assessBeforeExecute(planned.intent, () =>
          tauriInvoke<ApplyOutcome>("compare_apply", { taskId: current.task_id, paths, action, permanent, applyId }),
        );
        if (outcome === null) {
          setMessage({ tone: "info", text: "Cancelled — nothing was changed." });
          return;
        }
        setResult((r) => (r ? mergeOutcome(r, outcome) : r));
        setSelected(new Set());
        setMessage(describeOutcome(outcome));
      } catch (err) {
        setMessage({ tone: "error", text: `${ACTION_LABEL[action]} failed: ${errorText(err)}` });
      } finally {
        setRunning(null);
      }
    },
    [permanent],
  );

  const runAction = useCallback(
    async (action: CompareAction) => {
      const current = resultRef.current;
      if (!current || selected.size === 0) return;
      const paths = selectionRoots(selected);
      const targetLabel = action === "copy_to_left" || action === "delete_left" ? current.left_label : current.right_label;
      setMessage(null);
      setPendingDelete(null);
      let planned: CompareActionPlan;
      try {
        planned = await tauriInvoke<CompareActionPlan>("compare_plan_action", { taskId: current.task_id, paths, action, permanent, targetLabel });
      } catch (err) {
        setMessage({ tone: "error", text: errorText(err) });
        return;
      }
      if (planned.plan.steps.length === 0) {
        setMessage({ tone: "info", text: describeSkips(planned.plan), details: planned.plan.skipped.slice(0, 20).map((s) => `${s.path}: ${s.reason}`) });
        return;
      }
      if (isDelete(action)) {
        setPendingDelete({ action, planned, paths });
        return;
      }
      await execute(action, planned, paths);
    },
    [selected, permanent, execute],
  );

  const handleUndo = useCallback(
    async (undoId: string) => {
      try {
        const out = await tauriInvoke<{ success: boolean; summary: string }>("undo_by_correlation", { correlationId: undoId });
        setMessage({ tone: out.success ? "ok" : "error", text: out.summary });
        if (out.success) await runCompare();
      } catch (err) {
        setMessage({ tone: "error", text: `Undo failed: ${errorText(err)}` });
      }
    },
    [runCompare],
  );

  const busy = running !== null;
  const summary = result?.summary;
  const leftLabel = result?.left_label ?? sideLabel(request.left);
  const rightLabel = result?.right_label ?? sideLabel(request.right);

  const progressText = (() => {
    if (!running) return null;
    if (running.kind === "apply") {
      if (progress?.phase === "refreshing") return "Re-comparing the changed entries…";
      if (progress?.phase === "applying" && progress.total > 0) return `Working ${Math.min(progress.done + 1, progress.total)} of ${progress.total}${progress.current ? ` — ${progress.current}` : ""}`;
      return "Working…";
    }
    if (progress?.phase === "comparing") return `Comparing ${progress.entries_found.toLocaleString()} entries…`;
    if (progress) return `Listing… ${progress.folders_listed.toLocaleString()} folders, ${progress.entries_found.toLocaleString()} entries`;
    return "Listing…";
  })();

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" data-testid="folder-compare-backdrop">
      <div
        ref={containerRef}
        role="dialog"
        aria-modal="true"
        aria-label="Compare folders"
        className="flex h-[min(860px,94vh)] w-[min(1200px,97vw)] flex-col overflow-hidden rounded-lg border border-[var(--color-border)] bg-[var(--color-surface,var(--color-bg))] shadow-2xl"
        data-testid="folder-compare"
      >
        {/* Header */}
        <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-2">
          <div className="min-w-0 flex-1">
            <div className="text-xs font-semibold text-[color:var(--color-text)]">Compare folders</div>
            <div className="flex min-w-0 gap-2 text-[11px] text-[color:var(--color-text-muted)]">
              <span className="truncate" title={leftLabel} data-testid="compare-left-label">{leftLabel}</span>
              <span aria-hidden="true">⇄</span>
              <span className="truncate" title={rightLabel} data-testid="compare-right-label">{rightLabel}</span>
            </div>
          </div>
          <button type="button" onClick={handleClose} aria-label="Close compare folders" className="rounded p-1 text-[color:var(--color-text-muted)] hover:bg-[var(--color-hover-bg)]" data-testid="compare-close">
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        {/* Options */}
        <div className="flex flex-wrap items-end gap-3 border-b border-[var(--color-border)] px-3 py-2 text-[11px]">
          <label className="flex flex-col gap-0.5">
            <span className="text-zinc-500">Compare by</span>
            <select
              value={options.compare_mode}
              onChange={(e) => setOptions((o) => ({ ...o, compare_mode: e.target.value as CompareMode }))}
              className="rounded border border-[var(--color-border)] bg-transparent px-1 py-0.5"
              data-testid="compare-mode"
            >
              <option value="size_mtime">Size and modification time</option>
              <option value="size_only">Size only</option>
              <option value="checksum">Checksum (content)</option>
            </select>
          </label>
          <label className="flex flex-col gap-0.5">
            <span className="text-zinc-500">Folder depth (0 = all)</span>
            <input
              type="number"
              min={0}
              value={options.max_depth}
              onChange={(e) => setOptions((o) => ({ ...o, max_depth: Math.max(0, Number(e.target.value) || 0) }))}
              className="w-20 rounded border border-[var(--color-border)] bg-transparent px-1 py-0.5"
              data-testid="compare-depth"
            />
          </label>
          <label className="flex flex-col gap-0.5">
            <span className="text-zinc-500">Include (e.g. *.jpg, docs/*)</span>
            <input value={include} onChange={(e) => setInclude(e.target.value)} className="w-40 rounded border border-[var(--color-border)] bg-transparent px-1 py-0.5" data-testid="compare-include" />
          </label>
          <label className="flex flex-col gap-0.5">
            <span className="text-zinc-500">Exclude (e.g. *.tmp, node_modules/*)</span>
            <input value={exclude} onChange={(e) => setExclude(e.target.value)} className="w-40 rounded border border-[var(--color-border)] bg-transparent px-1 py-0.5" data-testid="compare-exclude" />
          </label>
          <button type="button" onClick={() => setShowRules((v) => !v)} aria-expanded={showRules} className="pb-0.5 text-zinc-500 underline" data-testid="compare-rules-toggle">
            Filter rules{rules.length > 0 ? ` (${rules.length})` : ""}
          </button>
          <label className="flex items-center gap-1 pb-0.5">
            <input type="checkbox" checked={options.time_tolerance} onChange={(e) => setOptions((o) => ({ ...o, time_tolerance: e.target.checked }))} />
            Allow 2 s time difference
          </label>
          <div className="ml-auto flex items-center gap-2 pb-0.5">
            {busy ? (
              <button type="button" onClick={cancelRunning} className="rounded border border-[var(--color-border)] px-2 py-1 hover:bg-[var(--color-hover-bg)]" data-testid="compare-cancel">
                Cancel
              </button>
            ) : (
              <button type="button" onClick={() => void runCompare()} className="rounded bg-[var(--color-primary)] px-2 py-1 text-[color:var(--color-primary-foreground)]" data-testid="compare-run">
                Compare again
              </button>
            )}
          </div>
        </div>

        {showRules && (
          <div className="space-y-1.5 border-b border-[var(--color-border)] px-3 py-2 text-[11px]" data-testid="compare-rules">
            <div className="text-zinc-500">Ordered include/exclude rules (first match wins), the same engine sync pairs and backups use. Compare again to apply.</div>
            <FilterPresetPicker rules={rules} onApply={(extra) => setRules(appendRules(rules, extra))} />
            {isAdvanced ? (
              <>
                <FilterRulesEditor rules={rules} onChange={setRules} groups={groups} />
                <ImportRulesButton onImport={(extra) => setRules(appendRules(rules, extra))} />
              </>
            ) : (
              <FilterRulesSummary rules={rules} onChange={setRules} />
            )}
          </div>
        )}

        {/* Status chips (OR filters) + Advanced navigation */}
        <div className="flex flex-wrap items-center gap-1.5 border-b border-[var(--color-border)] px-3 py-1.5 text-[11px]" role="group" aria-label="Show only">
          {COMPARE_STATUSES.map((s) => {
            const count = summary ? totalsCount(summary[s]) : 0;
            const on = statuses.has(s);
            return (
              <button
                key={s}
                type="button"
                aria-pressed={on}
                onClick={() => toggleStatus(s)}
                className={`rounded-full border px-2 py-0.5 ${on ? "border-[var(--color-primary)] bg-[var(--color-selection-bg)]" : "border-[var(--color-border)]"} ${STATUS_STYLE[s]}`}
                data-testid={`compare-chip-${s}`}
              >
                {STATUS_LABEL[s]} <span className="tabular-nums">{count.toLocaleString()}</span>
              </button>
            );
          })}
          {statuses.size > 0 && (
            <button type="button" onClick={() => setStatuses(new Set())} className="text-zinc-500 underline" data-testid="compare-chip-clear">
              Show all
            </button>
          )}
          {isAdvanced && tree && (
            <span className="ml-auto flex items-center gap-1.5" data-testid="compare-advanced-nav">
              <button type="button" className="rounded border border-[var(--color-border)] px-2 py-0.5" onClick={() => jumpTo(nextDifferencePath(tree, { statuses, sort }, focused))} data-testid="compare-next-diff">
                Next difference
              </button>
              <button type="button" className="rounded border border-[var(--color-border)] px-2 py-0.5" onClick={() => jumpTo(largestDifference(tree, "size"))} data-testid="compare-largest-size">
                Largest by size
              </button>
              <button type="button" className="rounded border border-[var(--color-border)] px-2 py-0.5" onClick={() => jumpTo(largestDifference(tree, "count"))} data-testid="compare-largest-count">
                Largest by file count
              </button>
              <label className="flex items-center gap-1 text-zinc-500">
                Sort
                <select value={sort} onChange={(e) => setSort(e.target.value as CompareSort)} className="rounded border border-[var(--color-border)] bg-transparent px-1" data-testid="compare-sort">
                  <option value="name">Name</option>
                  <option value="size_delta">Size difference</option>
                </select>
              </label>
            </span>
          )}
        </div>

        {/* Column headers */}
        <div className="grid grid-cols-[28px_minmax(0,2fr)_minmax(0,1fr)_88px_minmax(0,1fr)] gap-2 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-3 py-1 text-[11px] font-medium text-zinc-500">
          <span />
          <span>Name</span>
          <span className="text-right">Left</span>
          <span className="text-center">Status</span>
          <span className="text-right">Right</span>
        </div>

        {/* Tree */}
        <div ref={scrollRef} className="relative flex-1 overflow-auto" data-testid="compare-tree" role="treegrid" aria-label="Comparison results" aria-busy={busy}>
          {error && <p className="p-4 text-xs text-red-600 dark:text-red-400" data-testid="compare-error">{error}</p>}
          {!error && !result && <p className="p-4 text-xs text-zinc-500">{progressText}</p>}
          {result && rows.length === 0 && (
            <p className="p-4 text-center text-xs text-zinc-500" data-testid="compare-empty">
              {result.entries.length === 0 ? "Both folders are empty." : "Nothing matches the selected filters."}
            </p>
          )}
          {rows.length > 0 && (
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((v) => {
                const { node, path } = rows[v.index];
                const e = node.entry;
                const isSel = selected.has(path);
                return (
                  <div
                    key={path}
                    role="row"
                    aria-selected={isSel}
                    data-testid={`compare-row-${path}`}
                    className={`absolute left-0 top-0 grid w-full grid-cols-[28px_minmax(0,2fr)_minmax(0,1fr)_88px_minmax(0,1fr)] items-center gap-2 border-b border-[var(--color-border)] px-3 text-[11px] ${isSel ? "bg-[var(--color-selection-bg)]" : focused === path ? "bg-[var(--color-hover-bg)]" : ""}`}
                    style={{ height: ROW_HEIGHT, transform: `translateY(${v.start}px)` }}
                  >
                    <input
                      type="checkbox"
                      checked={isSel}
                      readOnly
                      onClick={(ev) => toggleSelect(path, ev.shiftKey)}
                      aria-label={`Select ${e.name}`}
                      data-testid={`compare-select-${path}`}
                    />
                    <span className="flex min-w-0 items-center" style={{ paddingLeft: node.depth * 14 }}>
                      {e.is_dir ? (
                        <button type="button" onClick={() => toggleCollapse(path)} aria-label={collapsed.has(path) ? `Expand ${e.name}` : `Collapse ${e.name}`} className="mr-0.5 shrink-0" data-testid={`compare-toggle-${path}`}>
                          {collapsed.has(path) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                        </button>
                      ) : (
                        <span className="mr-0.5 w-3 shrink-0" />
                      )}
                      <span className={`truncate ${e.is_dir ? "font-medium" : ""}`} title={path}>
                        {e.is_dir ? `${e.name}/` : e.name}
                      </span>
                      {e.is_dir && <RollUp node={node} />}
                    </span>
                    <SideCell present={e.in_left} size={e.left_size} modified={e.left_modified} isDir={e.is_dir} />
                    <span className={`text-center ${STATUS_STYLE[e.status]}`} title={e.error ?? e.reason ?? undefined} data-testid={`compare-status-${path}`}>
                      {STATUS_LABEL[e.status]}
                    </span>
                    <SideCell present={e.in_right} size={e.right_size} modified={e.right_modified} isDir={e.is_dir} />
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Messages */}
        {(message || pendingDelete || (result && result.notes.length > 0)) && (
          <div className="space-y-1 border-t border-[var(--color-border)] px-3 py-1.5 text-[11px]">
            {pendingDelete && (
              <div className="flex flex-wrap items-center gap-2 text-red-700 dark:text-red-400" data-testid="compare-confirm-delete">
                <span>
                  Delete {pendingDelete.planned.plan.files} file{pendingDelete.planned.plan.files === 1 ? "" : "s"}
                  {pendingDelete.planned.plan.folders > 0 ? ` and up to ${pendingDelete.planned.plan.folders} folder${pendingDelete.planned.plan.folders === 1 ? "" : "s"}` : ""} ({formatBytes(pendingDelete.planned.plan.bytes)}) on the {pendingDelete.action === "delete_left" ? "left" : "right"}
                  {permanent ? " permanently" : " (to the Trash where the side is a local folder)"}?
                </span>
                <button type="button" className="rounded bg-red-600 px-2 py-0.5 text-white" onClick={() => { const p = pendingDelete; setPendingDelete(null); void execute(p.action, p.planned, p.paths); }} data-testid="compare-confirm-delete-yes">
                  Delete
                </button>
                <button type="button" className="rounded border border-[var(--color-border)] px-2 py-0.5" onClick={() => setPendingDelete(null)}>
                  Keep
                </button>
              </div>
            )}
            {message && (
              <div className={message.tone === "error" ? "text-red-700 dark:text-red-400" : message.tone === "ok" ? "text-green-700 dark:text-green-400" : "text-zinc-500"} data-testid="compare-message">
                <span>{message.text}</span>
                {message.undoId && (
                  <button type="button" className="ml-2 underline" onClick={() => void handleUndo(message.undoId as string)} data-testid="compare-undo">
                    Undo copies
                  </button>
                )}
                {message.details && message.details.length > 0 && (
                  <ul className="mt-0.5 list-disc pl-4 text-zinc-500">
                    {message.details.map((d) => (
                      <li key={d}>{d}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
            {result?.notes.map((n) => (
              <p key={n} className="text-zinc-500">{n}</p>
            ))}
          </div>
        )}

        {/* Actions */}
        <div className="flex flex-wrap items-center gap-2 border-t border-[var(--color-border)] px-3 py-2 text-[11px]">
          <span className="text-zinc-500" data-testid="compare-progress">
            {busy ? progressText : summary ? `${selected.size.toLocaleString()} selected · compared in ${(summary.duration_ms / 1000).toFixed(1)} s` : ""}
          </span>
          <span className="ml-auto flex flex-wrap items-center gap-1.5">
            <label className="flex items-center gap-1 text-zinc-500" title="Deleted and replaced files are removed instead of going to the Trash">
              <input type="checkbox" checked={permanent} onChange={(e) => setPermanent(e.target.checked)} data-testid="compare-permanent" />
              Delete permanently
            </label>
            {(["copy_to_right", "copy_to_left", "delete_left", "delete_right"] as CompareAction[]).map((a) => (
              <button
                key={a}
                type="button"
                disabled={busy || selected.size === 0 || !result}
                onClick={() => void runAction(a)}
                className={`flex items-center gap-1 rounded border px-2 py-1 disabled:opacity-40 ${isDelete(a) ? "border-red-500/50 text-red-700 dark:text-red-400" : "border-[var(--color-border)]"}`}
                data-testid={`compare-action-${a}`}
              >
                {a === "copy_to_right" && <ArrowRight className="h-3 w-3" aria-hidden="true" />}
                {a === "copy_to_left" && <ArrowLeft className="h-3 w-3" aria-hidden="true" />}
                {isDelete(a) && <Trash2 className="h-3 w-3" aria-hidden="true" />}
                {ACTION_LABEL[a]}
              </button>
            ))}
          </span>
        </div>
      </div>
    </div>
  );
}
