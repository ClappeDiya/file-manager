/**
 * Pure helpers for the status-bar free-space indicator (iter 16).
 *
 * The backend's `detect_drives` IPC returns every mounted volume with
 * `mount_point`, `total_bytes`, and `free_bytes`. These helpers cover
 * (a) picking the drive that owns a given path and (b) formatting the
 * free / total numbers for the status bar.
 *
 * Living in `src/lib` keeps the path-matching logic unit-testable in
 * isolation from any IPC or React concerns, and lets future surfaces
 * (device sidebar, transfer preflight UI, automation rules) reuse the
 * same matching rule without re-inventing it.
 */

import { formatBytes } from "./format-bytes";

/** Subset of the Rust `DriveInfo` we actually need at this layer.
 *  Declared structurally so the helpers stay decoupled from the full
 *  drive-info type that the IPC layer surfaces (which carries 7 more
 *  fields callers below do not care about — device, filesystem,
 *  drive_type, read_only, label, uuid, used_bytes).
 *
 *  `removable` is optional so iter-16 status-bar callers (which only
 *  ever needed the four counting fields) keep working unchanged. The
 *  sidebar device picker added in iter 17 reads it to decide whether
 *  to render the eject affordance. */
export interface DriveInfoLike {
  name: string;
  mount_point: string;
  total_bytes: number;
  free_bytes: number;
  removable?: boolean;
}

/**
 * Pick the drive whose `mount_point` is the longest prefix of `path`.
 *
 * Longest-prefix wins because nested mounts are common — `/Volumes/USB`
 * is a sub-mount of `/`, so a file under `/Volumes/USB/foo` belongs to
 * the USB drive, not the root volume. Returns `null` when no drive
 * matches (an empty drives list, or a path that's outside every known
 * mount — common in browser dev mode where the IPC returned `[]`).
 *
 * The match is purely string-prefix on normalised paths. Both inputs
 * are normalised by stripping a trailing `/` (except for the root)
 * so `/foo` matches mount `/foo/` and `/foo/bar` matches mount `/foo`.
 */
export function findMountForPath<T extends DriveInfoLike>(
  drives: readonly T[],
  path: string,
): T | null {
  if (drives.length === 0 || path === "") return null;
  const normalisedPath = normalise(path);
  let best: T | null = null;
  let bestLen = -1;
  for (const drive of drives) {
    const mount = normalise(drive.mount_point);
    if (!pathIsUnder(normalisedPath, mount)) continue;
    if (mount.length > bestLen) {
      best = drive;
      bestLen = mount.length;
    }
  }
  return best;
}

function normalise(p: string): string {
  if (p === "/" || p === "") return "/";
  return p.endsWith("/") ? p.slice(0, -1) : p;
}

/** True iff `child` lies inside `parent` — either an exact match or a
 *  proper descendant. The "/" mount is treated specially so every
 *  absolute path matches the root volume. */
function pathIsUnder(child: string, parent: string): boolean {
  if (parent === "/") return child.startsWith("/");
  return child === parent || child.startsWith(`${parent}/`);
}

/** Terse format suitable for inlining into the status bar — e.g.
 *  "245 GB free". Returns `null` when the drive's `total_bytes` is 0
 *  (a special-purpose mount with no space accounting). */
export function formatDriveSpace(drive: DriveInfoLike): string | null {
  if (drive.total_bytes <= 0) return null;
  return `${formatBytes(drive.free_bytes)} free`;
}

/** Rich format suitable for a tooltip — includes the drive name and
 *  both free/total. Returns `null` for the same reason as
 *  `formatDriveSpace`. */
export function formatDriveSpaceDetail(drive: DriveInfoLike): string | null {
  if (drive.total_bytes <= 0) return null;
  return `${drive.name}: ${formatBytes(drive.free_bytes)} free of ${formatBytes(
    drive.total_bytes,
  )}`;
}
