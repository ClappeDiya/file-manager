/**
 * Pane-to-pane copy planning.
 *
 * `transferPaths` (file-manager.tsx) routes copies two ways:
 *   - FILES go through the transfer engine (`enqueue_transfer`) so the
 *     queue dock shows a real progress bar, pause/resume/cancel work, and
 *     each job lands in Transfer History with a verified checksum — the
 *     old `copy_files` route completed invisibly.
 *   - DIRECTORIES stay on `copy_files`, which already recurses. The
 *     engine moves single files only.
 *
 * The engine never overwrites, while `copy_files` keeps both files by
 * renaming the incoming copy to " (copy)" / " (copy N)". To preserve that
 * keep-both semantic on the engine path, the caller resolves the
 * destination file name up front (`resolveCopyDestName`) and enqueues a
 * FILE-path destination — a non-directory dest is used verbatim by the
 * engine instead of getting the basename appended.
 */

export interface PaneCopyJob {
  source: string;
  /** Verbatim destination FILE path. */
  dest: string;
  totalBytes: number;
}

export interface PaneCopyPlan {
  /** Directories to route through `copy_files` in one call. */
  dirPaths: string[];
  /** Per-file engine jobs, in selection order. */
  fileJobs: PaneCopyJob[];
}

/**
 * Mirrors Rust `next_available_copy_name` (file_ops_commands.rs):
 * "a.txt" → "a (copy).txt" → "a (copy 2).txt". `file_stem`/`extension`
 * semantics — a leading dot is not an extension (".env" → ".env (copy)"),
 * and only the LAST dot splits ("a.tar.gz" → "a.tar (copy).gz").
 */
export function resolveCopyDestName(name: string, taken: ReadonlySet<string>): string {
  if (!taken.has(name)) return name;
  const dot = name.lastIndexOf(".");
  const hasExt = dot > 0;
  const stem = hasExt ? name.slice(0, dot) : name;
  const ext = hasExt ? name.slice(dot) : "";
  for (let i = 1; i <= 1000; i++) {
    const candidate = i === 1 ? `${stem} (copy)${ext}` : `${stem} (copy ${i})${ext}`;
    if (!taken.has(candidate)) return candidate;
  }
  return name;
}

/**
 * Split `paths` into engine jobs (files) and a `copy_files` batch (dirs).
 * `entryByPath` carries `is_dir`/`size` for each source (from
 * `list_directory`); `destNames` is the set of names already present in
 * `destDir`. Names planned this call are added to the taken set so two
 * colliding sources in one batch get distinct destinations.
 */
export function planPaneCopy(
  paths: string[],
  destDir: string,
  entryByPath: ReadonlyMap<string, { is_dir: boolean; size: number }>,
  destNames: ReadonlySet<string>,
): PaneCopyPlan {
  const dirPaths: string[] = [];
  const fileJobs: PaneCopyJob[] = [];
  const taken = new Set(destNames);
  const prefix = destDir === "/" ? "" : destDir;
  for (const source of paths) {
    const entry = entryByPath.get(source);
    if (entry?.is_dir === true) {
      dirPaths.push(source);
      continue;
    }
    const base = source.split("/").pop() ?? source;
    const destName = resolveCopyDestName(base, taken);
    taken.add(destName);
    fileJobs.push({ source, dest: `${prefix}/${destName}`, totalBytes: entry?.size ?? 0 });
  }
  return { dirPaths, fileJobs };
}

/**
 * Wait for enqueued pane-copy jobs to settle. The engine is async and
 * emits no finish event, so callers poll `list_transfers` until every
 * job reaches a terminal state. `onSettle` fires each time a job lands
 * in a terminal state — used to refresh the destination pane as files
 * appear, rather than once at the end (a job parked in `conflict` must
 * not hide the evidence of files that already completed).
 */
export async function awaitPaneCopyJobs(
  ids: string[],
  listJobs: () => Promise<{ id: string; status: string }[]>,
  opts: { intervalMs?: number; timeoutMs?: number; onSettle?: () => void } = {},
): Promise<{ completed: number; failed: number }> {
  const interval = opts.intervalMs ?? 500;
  const deadline = Date.now() + (opts.timeoutMs ?? 10 * 60 * 1000);
  const pending = new Set(ids);
  let completed = 0;
  let failed = 0;
  while (pending.size > 0 && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, interval));
    let jobs: { id: string; status: string }[];
    try {
      jobs = await listJobs();
    } catch {
      continue;
    }
    for (const j of jobs) {
      if (!pending.has(j.id)) continue;
      if (j.status === "completed") {
        pending.delete(j.id);
        completed += 1;
        opts.onSettle?.();
      } else if (j.status === "failed" || j.status === "cancelled") {
        pending.delete(j.id);
        failed += 1;
        opts.onSettle?.();
      }
    }
  }
  return { completed, failed };
}
