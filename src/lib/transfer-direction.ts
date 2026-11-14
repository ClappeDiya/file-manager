/**
 * Which way a transfer goes, for the Upload / Download / Sync / Local chips
 * of the transfer panel. A path with a scheme (`sftp://host/dir`, `s3://…`)
 * is on a server; a drive letter (`C:\`) or a plain path is on this
 * computer. Sync runs are their own kind (they appear as groups).
 */

export type TransferDirection = "upload" | "download" | "local" | "remote";

/** Two or more scheme letters, so a Windows drive (`C://`) is never a server. */
const SCHEME = /^[a-z][a-z0-9+.-]+:\/\//i;

export const isRemotePath = (p: string): boolean => SCHEME.test(p.trim());

export function transferDirection(source: string, dest: string): TransferDirection {
  const from = isRemotePath(source);
  const to = isRemotePath(dest);
  if (from && to) return "remote";
  if (to) return "upload";
  if (from) return "download";
  return "local";
}

export type DirectionFilter = "all" | "upload" | "download" | "sync" | "local";

export const DIRECTION_CHIPS: { id: DirectionFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "upload", label: "Upload" },
  { id: "download", label: "Download" },
  { id: "sync", label: "Sync" },
  { id: "local", label: "Local" },
];

/** Whether a queued transfer shows under a chip (syncs are groups, never queue jobs). */
export function jobMatches(filter: DirectionFilter, source: string, dest: string): boolean {
  if (filter === "all") return true;
  if (filter === "sync") return false;
  return transferDirection(source, dest) === filter;
}

/** Whether running syncs show under a chip. */
export const syncsMatch = (filter: DirectionFilter): boolean => filter === "all" || filter === "sync";
