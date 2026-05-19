/**
 * CompareFilesModal — finally renders the result of the long-orphaned
 * `handleCompareFiles` flow.
 *
 * # Why this exists
 *
 * `handleCompareFiles` in `file-manager.tsx` has been wired into the
 * context menu ("Compare Selected") and the dual-selection keyboard
 * path for a while. It fetches both files' contents via the existing
 * `preview_file` IPC and stashes the pair in component state — but
 * until now there was no UI consumer. The state variable was even
 * prefixed `_compareData` to silence the unused-var warning. This
 * component is the missing renderer.
 *
 * # Design
 *
 * - **Side-by-side**: two equal columns, scrollable, with file name
 *   headers and a "N differences" badge.
 * - **Naive line-pair alignment**: uses the pure `compareFilesByLine`
 *   helper. Differing rows are highlighted (left red-tint for
 *   left-only / changed, right green-tint for right-only / changed)
 *   so the user can scan and find what differs.
 * - **DRY visual grammar**: re-uses the modal backdrop pattern from
 *   the existing PreviewPane / TextEditorModal / SafetyInterlockDialog
 *   modals. No new design tokens.
 * - **Keyboard-first**: Esc closes, the modal traps initial focus on
 *   the Close button.
 */
import { useEffect, useMemo, useRef } from "react";
import { X } from "lucide-react";
import { compareFilesByLine } from "@/lib/file-compare";

export interface CompareData {
  left: string;
  right: string;
  leftName: string;
  rightName: string;
}

interface CompareFilesModalProps {
  data: CompareData | null;
  onClose: () => void;
}

export function CompareFilesModal({ data, onClose }: CompareFilesModalProps) {
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  // Compute the diff lazily — same memo key as the inputs so a fresh
  // open with different content recomputes, but switching between
  // tabs/panes does not re-diff stable content.
  const diff = useMemo(() => {
    if (!data) return null;
    return compareFilesByLine(data.left, data.right);
  }, [data]);

  // Esc to close + autofocus the close button when the modal opens.
  useEffect(() => {
    if (!data) return;
    closeBtnRef.current?.focus();
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [data, onClose]);

  if (!data || !diff) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Compare files"
      data-testid="compare-files-backdrop"
      onClick={onClose}
    >
      <div
        className="w-[min(1080px,96vw)] h-[min(800px,92vh)] flex flex-col rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between gap-2 border-b border-[var(--color-border)] px-3 py-2">
          <div className="min-w-0 flex-1">
            <div className="text-xs font-semibold text-[color:var(--color-text)]">
              Compare files
            </div>
            <div className="text-[10px] text-[color:var(--color-text-muted)]">
              {diff.identical
                ? "Files are identical"
                : `${diff.changedCount} difference${diff.changedCount === 1 ? "" : "s"}`}
            </div>
          </div>
          <button
            ref={closeBtnRef}
            type="button"
            onClick={onClose}
            aria-label="Close compare files"
            className="rounded p-1 text-[color:var(--color-text-muted)] hover:bg-[var(--color-hover-bg,rgba(255,255,255,0.06))] focus:outline-none focus:ring-2 focus:ring-sky-500/40"
            data-testid="compare-files-close"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        {/* Column headers */}
        <div className="grid grid-cols-2 gap-px border-b border-[var(--color-border)] bg-[var(--color-border)]">
          <div className="bg-[var(--color-bg-secondary)] px-3 py-1.5 text-[11px] font-medium text-[color:var(--color-text)] truncate" title={data.leftName}>
            {data.leftName}
          </div>
          <div className="bg-[var(--color-bg-secondary)] px-3 py-1.5 text-[11px] font-medium text-[color:var(--color-text)] truncate" title={data.rightName}>
            {data.rightName}
          </div>
        </div>

        {/* Diff body — side-by-side with synced row heights. The
            grid-cols-2 layout lays cells out pairwise so a single
            flatMap producing [leftCell, rightCell] per DiffRow lands
            them on the same visual row. */}
        <div className="flex-1 overflow-auto" data-testid="compare-files-body">
          <div className="grid grid-cols-2 gap-px bg-[var(--color-border)]">
            {diff.rows.flatMap((row, idx) => [
              <DiffCell
                key={`l-${idx}`}
                side="left"
                line={row.left}
                lineNum={row.leftLineNum}
                changed={row.changed}
              />,
              <DiffCell
                key={`r-${idx}`}
                side="right"
                line={row.right}
                lineNum={row.rightLineNum}
                changed={row.changed}
              />,
            ])}
          </div>
        </div>
      </div>
    </div>
  );
}

function DiffCell({
  side,
  line,
  lineNum,
  changed,
}: {
  side: "left" | "right";
  line: string | null;
  lineNum: number | null;
  changed: boolean;
}) {
  const isBlank = line === null;
  // Highlight differing rows. Left side uses an amber tint, right
  // side a sky tint — same hue family as the rest of the product's
  // before/after surfaces (path-recall amber, drill-in sky).
  const bg = isBlank
    ? "bg-[var(--color-bg-tertiary,rgba(0,0,0,0.04))]"
    : changed
      ? side === "left"
        ? "bg-amber-500/8"
        : "bg-sky-500/8"
      : "bg-[var(--color-bg)]";
  return (
    <div className={`flex items-start gap-2 px-3 py-0.5 font-mono text-[11px] leading-relaxed ${bg}`}>
      <span className="w-8 flex-shrink-0 text-right text-[10px] text-[color:var(--color-text-muted)] select-none">
        {lineNum ?? ""}
      </span>
      <span className="flex-1 whitespace-pre-wrap break-words text-[color:var(--color-text)]">
        {line ?? ""}
      </span>
    </div>
  );
}
