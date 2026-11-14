import type { WorkspaceJob } from "@/stores/transfer-workspace-store";
const terminal = new Set(["completed", "failed", "cancelled"]);
export function transferSummary(jobs: WorkspaceJob[]) {
  const completed = jobs.filter((j) => j.status === "completed").length;
  const failed = jobs.filter((j) => j.status === "failed").length;
  const cancelled = jobs.filter((j) => j.status === "cancelled").length;
  const finished = jobs.filter((j) => terminal.has(j.status)).length;
  const bytesTotal = jobs.reduce(
    (sum, j) => sum + Math.max(0, j.total_bytes || 0),
    0,
  );
  const bytesDone = jobs.reduce(
    (sum, j) =>
      sum + Math.max(0, Math.min(j.transferred_bytes || 0, j.total_bytes || 0)),
    0,
  );
  const percent =
    bytesTotal > 0
      ? Math.min(100, Math.floor((bytesDone / bytesTotal) * 100))
      : jobs.length && finished === jobs.length
        ? 100
        : null;
  const verified = jobs.filter(
    (j) =>
      j.status === "completed" &&
      j.source_checksum &&
      j.dest_checksum &&
      j.source_checksum === j.dest_checksum,
  ).length;
  return {
    total: jobs.length,
    completed,
    failed,
    cancelled,
    finished,
    bytesTotal,
    bytesDone,
    percent,
    verified,
    settled: jobs.length > 0 && finished === jobs.length,
    successPercent: jobs.length
      ? Math.round((completed / jobs.length) * 100)
      : null,
    speed: jobs.reduce(
      (sum, j) => sum + (j.status === "active" ? j.speed_bps || 0 : 0),
      0,
    ),
  };
}
