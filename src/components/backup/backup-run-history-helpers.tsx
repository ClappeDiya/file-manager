


/** One destination's result in a run (a replication report's `replica_runs`). */
export interface ReplicaResult {
  id: string;
  name: string;
  status: string;
  started_at: string;
  finished_at: string | null;
  files: number;
  bytes: number;
  error: string | null;
}


/** The per-destination results a run's report carries: a replication's own, or those of the replications a backup ran after itself. */
export function replicaResults(report: Record<string, unknown>): ReplicaResult[] {
  const own = Array.isArray(report.replica_runs) ? (report.replica_runs as ReplicaResult[]) : [];
  const after = Array.isArray(report.replication) ? (report.replication as Record<string, unknown>[]).flatMap((r) => (Array.isArray(r.replica_runs) ? (r.replica_runs as ReplicaResult[]) : [])) : [];
  return [...own, ...after];
}
