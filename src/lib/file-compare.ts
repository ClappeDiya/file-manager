/**
 * file-compare
 *
 * Pure line-by-line file comparison. Given two strings, produces an
 * aligned list of paired rows that the CompareFilesModal renders
 * side-by-side with diff highlighting.
 *
 * # Design
 *
 * - **Pure**: no side effects, no DOM, no IPC. Takes two strings,
 *   returns a DiffResult. Trivially unit-testable across the
 *   important corner cases (empty, identical, all-different,
 *   different lengths, trailing newline).
 * - **Naive alignment**: rows are paired by line number, not by
 *   longest-common-subsequence. The first N lines are compared
 *   pairwise; whichever side is longer contributes "extra" rows at
 *   the bottom with a null counterpart. This is the simplest
 *   semantically correct rendering for the common case where users
 *   compare two versions of the same file (most lines line up).
 *   Misaligned files (e.g. one inserts a line near the top) will
 *   show cascading diff highlights from that point — acceptable for
 *   v1; a future LCS-based alignment can drop in behind this helper
 *   without changing any caller.
 * - **DRY**: no new dependencies. The diff format is a single flat
 *   array consumed by the modal's row-by-row renderer; no nested
 *   data structures, no states.
 */

/** One aligned row in the diff. Either side may be null when the
 *  files have different line counts. `changed` is true when both
 *  sides exist and differ, or when exactly one side is null. */
export interface DiffRow {
  left: string | null;
  right: string | null;
  /** 1-based line number in the LEFT file, or null when no left line. */
  leftLineNum: number | null;
  /** 1-based line number in the RIGHT file, or null when no right line. */
  rightLineNum: number | null;
  changed: boolean;
}

export interface DiffResult {
  rows: DiffRow[];
  /** Count of rows where `changed` is true. Useful for the modal
   *  header ("3 differences"). */
  changedCount: number;
  /** True iff both files have identical content (no `changed` rows). */
  identical: boolean;
}

/** Split a string into lines without consuming trailing empty
 *  artifacts from a final newline. An empty string yields a single
 *  empty line, matching what every text editor renders for an
 *  empty file. A string ending in "\n" yields N lines, not N+1. */
function splitLines(text: string): string[] {
  if (text === "") return [""];
  const lines = text.split(/\r\n|\r|\n/);
  // Trim the trailing empty produced by a final newline.
  if (lines.length > 1 && lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines;
}

/**
 * Compare two file contents line by line.
 *
 * The result aligns the first `min(L, R)` lines pairwise. Any
 * trailing lines from the longer file are appended as rows with the
 * shorter side set to null. Rows where the two sides differ are
 * marked `changed: true`; identical rows are `changed: false`.
 */
export function compareFilesByLine(left: string, right: string): DiffResult {
  const leftLines = splitLines(left);
  const rightLines = splitLines(right);
  const max = Math.max(leftLines.length, rightLines.length);
  const rows: DiffRow[] = [];
  let changedCount = 0;
  for (let i = 0; i < max; i++) {
    const l = i < leftLines.length ? leftLines[i] : null;
    const r = i < rightLines.length ? rightLines[i] : null;
    const changed = l !== r;
    if (changed) changedCount += 1;
    rows.push({
      left: l,
      right: r,
      leftLineNum: l !== null ? i + 1 : null,
      rightLineNum: r !== null ? i + 1 : null,
      changed,
    });
  }
  return {
    rows,
    changedCount,
    identical: changedCount === 0,
  };
}
