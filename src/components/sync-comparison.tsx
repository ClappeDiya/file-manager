/**
 * Side-by-side Source | Destination view of a sync dry run, so it's clear
 * exactly what a run will add, overwrite and delete before you confirm.
 */
import { useMemo, useState } from "react";
import { formatBytes } from "@/lib/format-bytes";
import {
  buildComparisonRows,
  summarizeRows,
  type CompareRow,
  type CompareSide,
  type PreviewLike,
  type RowChange,
} from "@/lib/sync-compare";

const CHANGE_STYLE: Record<RowChange, { label: string; cls: string }> = {
  add: { label: "New", cls: "text-green-700 dark:text-green-400" },
  modify: { label: "Overwrite", cls: "text-blue-700 dark:text-blue-400" },
  delete: { label: "Delete", cls: "text-red-700 dark:text-red-400" },
  conflict: { label: "Conflict", cls: "text-yellow-700 dark:text-yellow-400" },
  skip: { label: "Skip", cls: "text-zinc-500" },
  mkdir: { label: "New folder", cls: "text-green-700 dark:text-green-400" },
  rmdir: { label: "Remove folder", cls: "text-red-700 dark:text-red-400" },
  folder: { label: "", cls: "text-zinc-500" },
};

function Cell({ side, row, which }: { side: CompareSide | null; row: CompareRow; which: "source" | "dest" }) {
  const struck = which === "dest" && (row.change === "delete" || row.change === "rmdir");
  if (!side) {
    return <span className="text-zinc-300 dark:text-zinc-600">—</span>;
  }
  return (
    <span className={`flex min-w-0 items-baseline gap-1.5 ${struck ? "line-through text-red-600 dark:text-red-400" : ""}`}>
      <span className="truncate" style={{ paddingLeft: `${row.depth * 10}px` }} title={row.path}>
        {row.isDir ? `${row.name}/` : row.name}
      </span>
      {!row.isDir && side.size !== null && (
        <span className="ml-auto shrink-0 tabular-nums text-zinc-400">{formatBytes(side.size)}</span>
      )}
    </span>
  );
}

export function SyncComparison({ preview }: { preview: PreviewLike }) {
  const [showSkipped, setShowSkipped] = useState(false);
  const rows = useMemo(() => buildComparisonRows(preview, showSkipped), [preview, showSkipped]);
  const totals = useMemo(() => summarizeRows(rows), [rows]);

  return (
    <div className="space-y-1.5" data-testid="sync-comparison">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
        <span className={CHANGE_STYLE.add.cls}>{totals.add + totals.mkdir} new</span>
        <span className={CHANGE_STYLE.modify.cls}>{totals.modify} overwritten</span>
        <span className={CHANGE_STYLE.delete.cls}>{totals.delete} deleted</span>
        {totals.conflict > 0 && <span className={CHANGE_STYLE.conflict.cls}>{totals.conflict} conflicts</span>}
        <label className="ml-auto flex items-center gap-1 text-zinc-500">
          <input type="checkbox" checked={showSkipped} onChange={(e) => setShowSkipped(e.target.checked)} />
          Show skipped
        </label>
      </div>
      {rows.length === 0 ? (
        <p className="py-4 text-center text-xs text-zinc-500">Nothing to change — both sides already match.</p>
      ) : (
        <div className="max-h-72 overflow-auto rounded border border-zinc-200 dark:border-zinc-700">
          <table className="w-full table-fixed text-[11px]">
            <thead className="sticky top-0 bg-zinc-50 dark:bg-zinc-800 text-zinc-500">
              <tr>
                <th className="w-[42%] px-2 py-1 text-left font-medium">Source</th>
                <th className="w-[16%] px-1 py-1 text-center font-medium">Change</th>
                <th className="w-[42%] px-2 py-1 text-left font-medium">Destination</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const style = CHANGE_STYLE[row.change];
                return (
                  <tr
                    key={row.path}
                    className="border-t border-zinc-100 dark:border-zinc-800"
                    title={row.reason || undefined}
                  >
                    <td className="px-2 py-0.5 text-zinc-700 dark:text-zinc-300">
                      <Cell side={row.source} row={row} which="source" />
                    </td>
                    <td className={`px-1 py-0.5 text-center ${style.cls}`}>{style.label}</td>
                    <td className="px-2 py-0.5 text-zinc-700 dark:text-zinc-300">
                      <Cell side={row.dest} row={row} which="dest" />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
