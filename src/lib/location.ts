/**
 * Pane locations.
 *
 * A pane path is either a local path (unchanged — every existing path,
 * tab and persisted state keeps meaning what it did) or a folder of a
 * saved connection written `conn://<connection id><connector path>`, where
 * the connector path is what `connector_list_remote` returns (it starts with
 * `/`; for Google Drive it is a chain of ids, for OneDrive Graph's
 * `/drive/root:/…` form). Mirrors `transfer_engine::location` in Rust.
 *
 * Pure helpers only: routing lives in `location-provider.ts`.
 */

export const CONN_SCHEME = "conn://";

export type PaneLocation =
  | { kind: "local"; path: string }
  | { kind: "remote"; connectionId: string; path: string };

/** True for `conn://…` pane paths. */
export function isRemotePath(p: string | null | undefined): boolean {
  return !!p && p.startsWith(CONN_SCHEME);
}

/** Split a pane path; anything not `conn://` is local, unchanged. */
export function parseLocation(p: string): PaneLocation {
  if (!isRemotePath(p)) return { kind: "local", path: p };
  const rest = p.slice(CONN_SCHEME.length);
  const slash = rest.indexOf("/");
  const connectionId = slash < 0 ? rest : rest.slice(0, slash);
  const path = slash < 0 ? "/" : rest.slice(slash) || "/";
  return { kind: "remote", connectionId, path };
}

/** The pane path of `path` on connection `connectionId`. */
export function remoteLocation(connectionId: string, path = "/"): string {
  const p = path.startsWith("/") ? path : `/${path}`;
  return `${CONN_SCHEME}${connectionId}${p}`;
}

/** The root folder of a pane path: `/` locally (the existing Home button), the connection's root remotely. */
export function locationRoot(p: string): string {
  const loc = parseLocation(p);
  return loc.kind === "remote" ? remoteLocation(loc.connectionId, "/") : "/";
}

/** `name` inside folder `dir`, for either kind of location. */
export function joinLocation(dir: string, name: string): string {
  if (isRemotePath(dir)) return dir.endsWith("/") ? `${dir}${name}` : `${dir}/${name}`;
  if (dir === "/") return `/${name}`;
  const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  return dir.endsWith(sep) ? `${dir}${name}` : `${dir}${sep}${name}`;
}

/** The root a local path starts with: `C:\`, `\\server\share`, or `/`. */
function localRoot(p: string): string {
  const drive = /^([A-Za-z]:)([\\/]?)/.exec(p);
  if (drive) return `${drive[1]}${drive[2] || "\\"}`;
  const unc = /^(\\\\[^\\/]+[\\/][^\\/]+)/.exec(p);
  if (unc) return unc[1];
  return "/";
}

/**
 * Whether `p` is the top of its location (`/`, a drive, a network share, a
 * connection's root), where going to the parent folder stays put.
 */
export function isTopFolder(p: string): boolean {
  const loc = parseLocation(p);
  if (loc.kind === "remote") return normalizeRemoteFolder(loc.path) === "/";
  const root = localRoot(p);
  const trimmed = p.replace(/[\\/]+$/, "");
  return trimmed === "" || trimmed === root.replace(/[\\/]+$/, "");
}

/** The folder containing `p` (a root — `/`, `C:\`, a share, a connection's root — has itself as parent). */
export function parentLocation(p: string): string {
  const loc = parseLocation(p);
  if (loc.kind === "remote") {
    if (normalizeRemoteFolder(loc.path) === "/") return p;
    const trimmed = loc.path.replace(/\/+$/, "");
    const cut = trimmed.lastIndexOf("/");
    return remoteLocation(loc.connectionId, cut <= 0 ? "/" : trimmed.slice(0, cut));
  }
  if (isTopFolder(p)) return /^[A-Za-z]:$/.test(p) ? `${p}\\` : p;
  const parent = p.replace(/[\\/]+$/, "").replace(/[\\/][^\\/]*$/, "");
  if (!parent) return "/";
  const root = localRoot(p);
  return parent.length < root.length || parent === root.replace(/[\\/]+$/, "") ? root : parent;
}

/**
 * A connector folder path in one spelling for comparisons: OneDrive's Graph
 * prefixes (`/drive/root:`, `/drives/<id>/root:`) name the drive root, which
 * a pane opened at `/` calls `/`; trailing slashes are dropped. Mirrors
 * `normalize_remote_folder` in `transfer_engine/location.rs`.
 */
export function normalizeRemoteFolder(path: string): string {
  const p = path.trim();
  const m = /^(?:\/drive\/root:|\/drives\/[^/]+\/root:)(\/.*)?$/.exec(p);
  const rest = (m ? (m[1] ?? "") : p).replace(/\/+$/, "");
  if (!rest) return "/";
  return rest.startsWith("/") ? rest : `/${rest}`;
}

/** Whether two pane paths name the same folder (remote paths compared normalised). */
export function sameFolder(a: string, b: string): boolean {
  const la = parseLocation(a);
  const lb = parseLocation(b);
  if (la.kind === "local" || lb.kind === "local") return a === b;
  return la.connectionId === lb.connectionId && normalizeRemoteFolder(la.path) === normalizeRemoteFolder(lb.path);
}

/** Both on this computer, or both on the same connection. */
export function sameLocationRoot(a: string, b: string): boolean {
  const la = parseLocation(a);
  const lb = parseLocation(b);
  if (la.kind === "local" && lb.kind === "local") return true;
  return la.kind === "remote" && lb.kind === "remote" && la.connectionId === lb.connectionId;
}

/** The connection a pane path is on, or null for local paths. */
export function locationConnectionId(p: string): string | null {
  const loc = parseLocation(p);
  return loc.kind === "remote" ? loc.connectionId : null;
}

// ── Display names of remote paths ──
// Listings give each remote path its name (Google Drive paths are ids, so
// a name cannot be derived from the path). Remembered here so breadcrumbs,
// tab labels and history show names instead of ids.

const REMOTE_NAME_LIMIT = 5000;
const remoteNames = new Map<string, string>();

/** Remember the names of listed entries of a remote folder. */
export function rememberRemoteNames(entries: ReadonlyArray<{ path: string; name: string }>) {
  for (const e of entries) {
    if (!isRemotePath(e.path)) continue;
    const key = e.path.replace(/\/+$/, "");
    remoteNames.delete(key); // re-insert: most recently seen last
    remoteNames.set(key, e.name);
  }
  while (remoteNames.size > REMOTE_NAME_LIMIT) {
    const first = remoteNames.keys().next().value;
    if (first === undefined) break;
    remoteNames.delete(first);
  }
}

/** The remembered name of a remote path, else its last segment. */
export function remoteName(p: string): string {
  const key = p.replace(/\/+$/, "");
  const known = remoteNames.get(key);
  if (known) return known;
  const loc = parseLocation(p);
  const segs = loc.path.split("/").filter(Boolean);
  const last = segs[segs.length - 1] ?? "";
  // OneDrive's Graph form `/drive/root:` names the drive root.
  return last.endsWith(":") ? "" : last;
}

/** Test hook. */
export function clearRemoteNames() {
  remoteNames.clear();
}

/**
 * Tab / history label of a pane path. Local paths keep the established
 * rule ("Root" for `/`, else the last segment); a connection's root is the
 * connection's name.
 */
export function locationLabel(p: string, connectionName?: (id: string) => string | undefined): string {
  if (!isRemotePath(p)) return p === "/" ? "Root" : p.split("/").filter(Boolean).pop() || "Root";
  const loc = parseLocation(p) as Extract<PaneLocation, { kind: "remote" }>;
  return remoteName(p) || connectionName?.(loc.connectionId) || "Connection";
}

export interface LocationSegment {
  label: string;
  path: string;
}

/**
 * Breadcrumb segments below the root: local paths split on `/` (as
 * before); a remote path into one segment per folder, named from the
 * listing names when known. OneDrive's `/drive/root:` prefix is the root
 * (no segments of its own).
 */
export function locationSegments(p: string): LocationSegment[] {
  if (!isRemotePath(p)) {
    const parts = p.split("/").filter(Boolean);
    return parts.map((part, index) => ({ label: part, path: "/" + parts.slice(0, index + 1).join("/") }));
  }
  const loc = parseLocation(p) as Extract<PaneLocation, { kind: "remote" }>;
  const parts = loc.path.split("/").filter(Boolean);
  const out: LocationSegment[] = [];
  // Graph's `/drive/root:` (or `/drives/<id>/root:`) is the root itself.
  let start = 0;
  parts.forEach((p, i) => {
    if (p.endsWith(":")) start = i + 1;
  });
  for (let i = start; i < parts.length; i++) {
    const path = remoteLocation(loc.connectionId, "/" + parts.slice(0, i + 1).join("/"));
    const label = remoteName(path);
    if (!label) continue; // e.g. Graph's `root:` segment
    out.push({ label, path });
  }
  return out;
}

export interface AddressConnection {
  id: string;
  name: string;
}

export type AddressParse = { ok: true; path: string } | { ok: false; error: string };

/**
 * What the user typed in the address bar: a local absolute path, a
 * `conn://<id>/path` location, or `<connection name>:path`. A Windows drive
 * path (`C:\…`, `D:`) stays local unless a connection is named exactly
 * that letter.
 */
export function parseAddressInput(text: string, connections: ReadonlyArray<AddressConnection>): AddressParse {
  const t = text.trim();
  if (!t) return { ok: false, error: "Type a folder path" };
  if (isRemotePath(t)) {
    const loc = parseLocation(t) as Extract<PaneLocation, { kind: "remote" }>;
    if (!connections.some((c) => c.id === loc.connectionId)) return { ok: false, error: "No saved connection has that id" };
    return { ok: true, path: remoteLocation(loc.connectionId, loc.path) };
  }
  // A full local path stays local even with ":" in a name (`/notes:2024`).
  const absoluteLocal = t.startsWith("/") || t.startsWith("\\\\");
  const colon = absoluteLocal ? -1 : t.indexOf(":");
  if (colon > 0) {
    const name = t.slice(0, colon);
    const rest = t.slice(colon + 1);
    const conn =
      connections.find((c) => c.name === name) ?? connections.find((c) => c.name.toLowerCase() === name.toLowerCase());
    const windowsDrive = /^[A-Za-z]$/.test(name) && (rest === "" || /^[\\/]/.test(rest));
    if (conn && !(windowsDrive && conn.name !== name)) return { ok: true, path: remoteLocation(conn.id, rest.trim() || "/") };
    if (windowsDrive) return { ok: true, path: rest === "" ? `${name}:\\` : t };
    return { ok: false, error: `No saved connection is named "${name}"` };
  }
  if (absoluteLocal) return { ok: true, path: t.length > 1 ? t.replace(/[\\/]+$/, "") || "/" : t };
  return { ok: false, error: "Type a full path (starting with /), or <connection name>:path" };
}

/**
 * The typed text split into the folder to list for suggestions and the
 * partial name being typed: `/home/us` → (`/home`, `us`);
 * `Drive:/Docs/Re` → (`Drive:/Docs`, `Re`).
 */
export function splitAddressForCompletion(text: string): { parent: string; partial: string } {
  const cut = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"));
  if (cut < 0) return { parent: text, partial: "" };
  const parent = text.slice(0, cut) || text.slice(0, cut + 1);
  return { parent: parent.endsWith(":") ? `${parent}/` : parent, partial: text.slice(cut + 1) };
}

/** How a pane path reads in the address bar: `<connection name>:<path>` for remotes. */
export function formatAddress(p: string, connectionName?: (id: string) => string | undefined): string {
  if (!isRemotePath(p)) return p;
  const loc = parseLocation(p) as Extract<PaneLocation, { kind: "remote" }>;
  const name = connectionName?.(loc.connectionId);
  return name ? `${name}:${loc.path}` : p;
}

/** Connector path `full` relative to connection root `root` (mount subfolders). */
export function relativeToRoot(root: string, full: string): string {
  const r = root.replace(/\/+$/, "");
  if (r === "" || r === "/") return full.replace(/^\/+/, "");
  if (full === r) return "";
  return full.startsWith(`${r}/`) ? full.slice(r.length + 1) : full.replace(/^\/+/, "");
}
