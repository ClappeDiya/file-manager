/**
 * In-flow folder picker for this computer and any saved connection: roots
 * on the left (this computer's drives, then the connections), a lazily
 * loaded folder tree on the right, "New folder" inside the selected folder,
 * and Choose. Everything goes through the location provider, so a
 * connection folder is listed and created exactly as the panes do it.
 *
 * It returns a pane path (local path or `conn://<id>/<path>`); callers that
 * store a connector path take `parseLocation(path).path`. `FolderField`
 * pairs it with a text field and keeps the native picker button for local
 * folders.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Cloud, Folder, FolderOpen, FolderPlus, HardDrive } from "lucide-react";
import { cn } from "@ufop/ui-components";
import { useAllDrives } from "@/hooks/use-drive-space";
import { useConnectionsStore } from "@/stores/connections-store";
import { listDirectory, makeDirectory } from "@/lib/location-provider";
import { formatAddress, isRemotePath, joinLocation, locationConnectionId, parseLocation, remoteLocation } from "@/lib/location";
import { pickFolder } from "@/lib/folder-picker";
import { promptText } from "@/lib/prompt-dialog";
import { errorText } from "@/lib/error-text";

export interface RemoteFolderPickerProps {
  title: string;
  /** Where it opens (pane path). */
  initialPath?: string | null;
  /** Offer this computer's folders (default true). */
  allowLocal?: boolean;
  /** Offer saved connections (default true). */
  allowConnections?: boolean;
  /** Only this pane folder and below (e.g. one connection's `remote_path`). */
  lockRoot?: string | null;
  onPick: (path: string) => void;
  onCancel: () => void;
}

interface Root {
  key: string;
  label: string;
  path: string;
  remote: boolean;
}

interface Node {
  loading: boolean;
  children: { path: string; name: string }[] | null;
  error: string | null;
}

export function RemoteFolderPicker({ title, initialPath, allowLocal = true, allowConnections = true, lockRoot = null, onPick, onCancel }: RemoteFolderPickerProps) {
  const drives = useAllDrives();
  const connections = useConnectionsStore((s) => s.connections);
  const nameOf = useConnectionsStore((s) => s.nameOf);
  useEffect(() => {
    if (allowConnections || lockRoot) void useConnectionsStore.getState().ensureLoaded();
  }, [allowConnections, lockRoot]);

  const roots = useMemo<Root[]>(() => {
    if (lockRoot) {
      const id = locationConnectionId(lockRoot);
      return [{ key: lockRoot, label: id ? `${nameOf(id) ?? "Connection"}${parseLocation(lockRoot).path === "/" ? "" : ` ${parseLocation(lockRoot).path}`}` : lockRoot, path: lockRoot, remote: !!id }];
    }
    const out: Root[] = [];
    if (allowLocal) {
      const seen = new Set<string>();
      for (const d of drives) {
        if (seen.has(d.mount_point)) continue;
        seen.add(d.mount_point);
        out.push({ key: `local:${d.mount_point}`, label: d.name || d.mount_point, path: d.mount_point, remote: false });
      }
      if (!seen.has("/") && !drives.some((d) => /^[A-Za-z]:/.test(d.mount_point))) out.unshift({ key: "local:/", label: "This computer", path: "/", remote: false });
    }
    if (allowConnections) {
      for (const c of connections) out.push({ key: c.id, label: c.name, path: remoteLocation(c.id, c.remote_path || "/"), remote: true });
    }
    return out;
  }, [lockRoot, allowLocal, allowConnections, drives, connections, nameOf]);

  const [selected, setSelected] = useState<string | null>(initialPath ?? lockRoot ?? null);
  const [root, setRoot] = useState<Root | null>(null);
  const [nodes, setNodes] = useState<Record<string, Node>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  // Start at the root that holds the initial path (else the first root).
  useEffect(() => {
    if (root || roots.length === 0) return;
    const start = initialPath ?? lockRoot;
    const holding = start
      ? roots
          .filter((r) => (r.remote ? locationConnectionId(r.path) === locationConnectionId(start) && isRemotePath(start) : !isRemotePath(start) && start.startsWith(r.path)))
          .sort((a, b) => b.path.length - a.path.length)[0]
      : undefined;
    setRoot(holding ?? roots[0]);
  }, [roots, root, initialPath, lockRoot]);

  const load = useCallback(async (path: string) => {
    setNodes((n) => ({ ...n, [path]: { loading: true, children: n[path]?.children ?? null, error: null } }));
    try {
      const entries = await listDirectory(path);
      const children = entries.filter((e) => e.is_dir).map((e) => ({ path: e.path, name: e.name })).sort((a, b) => a.name.localeCompare(b.name));
      setNodes((n) => ({ ...n, [path]: { loading: false, children, error: null } }));
    } catch (err) {
      setNodes((n) => ({ ...n, [path]: { loading: false, children: [], error: errorText(err) } }));
    }
  }, []);

  useEffect(() => {
    if (!root) return;
    setExpanded(new Set([root.path]));
    if (!selected || (root.remote ? locationConnectionId(selected) !== locationConnectionId(root.path) : isRemotePath(selected))) setSelected(root.path);
    void load(root.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reload only when the root changes
  }, [root, load]);

  const toggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else {
        next.add(path);
        if (!nodes[path]) void load(path);
      }
      return next;
    });
  };

  const createFolder = async () => {
    if (!selected) return;
    const name = (await promptText("New folder name:", "New Folder"))?.trim();
    if (!name) return;
    setError(null);
    try {
      const path = joinLocation(selected, name);
      await makeDirectory(path);
      await load(selected);
      setExpanded((prev) => new Set(prev).add(selected));
      setSelected(path);
    } catch (err) {
      setError(`Could not create the folder: ${errorText(err)}`);
    }
  };

  const renderNode = (path: string, label: string, depth: number) => {
    const node = nodes[path];
    const open = expanded.has(path);
    return (
      <li key={path}>
        <div
          className={cn("flex items-center gap-1 px-1 py-0.5 rounded cursor-pointer text-xs", selected === path ? "bg-[var(--color-primary)]/15 text-[color:var(--color-text)]" : "hover:bg-[var(--color-hover-bg)]")}
          style={{ paddingLeft: depth * 14 + 4 }}
          onClick={() => setSelected(path)}
          onDoubleClick={() => toggle(path)}
          role="treeitem"
          aria-expanded={open}
          aria-selected={selected === path}
          data-testid={`picker-node-${path}`}
        >
          <button type="button" className="w-4 h-4 flex items-center justify-center" onClick={(e) => { e.stopPropagation(); toggle(path); }} aria-label={open ? `Collapse ${label}` : `Expand ${label}`}>
            {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          </button>
          {open ? <FolderOpen className="h-3.5 w-3.5 shrink-0" /> : <Folder className="h-3.5 w-3.5 shrink-0" />}
          <span className="truncate">{label}</span>
        </div>
        {open && node && (
          <ul role="group">
            {node.loading && !node.children && <li className="text-[11px] text-gray-400" style={{ paddingLeft: (depth + 1) * 14 + 24 }}>Loading…</li>}
            {node.error && <li className="text-[11px] text-red-500" style={{ paddingLeft: (depth + 1) * 14 + 24 }} role="alert">{node.error}</li>}
            {node.children?.length === 0 && !node.error && <li className="text-[11px] text-gray-400" style={{ paddingLeft: (depth + 1) * 14 + 24 }}>No folders</li>}
            {node.children?.map((c) => renderNode(c.path, c.name, depth + 1))}
          </ul>
        )}
      </li>
    );
  };

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40" role="dialog" aria-modal="true" aria-label={title} data-testid="remote-folder-picker">
      <div className="w-[640px] max-w-[95vw] h-[440px] max-h-[90vh] flex flex-col rounded-lg bg-[var(--color-bg)] border border-[var(--color-border)] shadow-xl">
        <div className="px-4 py-2 border-b border-[var(--color-border)] text-sm font-semibold">{title}</div>
        <div className="flex flex-1 min-h-0">
          {roots.length > 1 && (
            <ul className="w-44 shrink-0 border-r border-[var(--color-border)] overflow-y-auto py-1" aria-label="Locations">
              {roots.map((r) => (
                <li key={r.key}>
                  <button
                    type="button"
                    onClick={() => setRoot(r)}
                    className={cn("w-full flex items-center gap-1.5 px-2 py-1 text-xs text-left", root?.key === r.key ? "bg-[var(--color-hover-bg)] font-medium" : "hover:bg-[var(--color-hover-bg)]")}
                    data-testid={`picker-root-${r.key}`}
                  >
                    {r.remote ? <Cloud className="h-3.5 w-3.5 shrink-0" /> : <HardDrive className="h-3.5 w-3.5 shrink-0" />}
                    <span className="truncate">{r.label}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          <ul className="flex-1 overflow-y-auto p-1" role="tree" aria-label="Folders">
            {root && renderNode(root.path, root.label, 0)}
            {roots.length === 0 && <li className="text-xs text-gray-500 p-2">No locations available.</li>}
          </ul>
        </div>
        {error && <div className="px-4 py-1 text-xs text-red-600" role="alert">{error}</div>}
        <div className="flex items-center gap-2 px-4 py-2 border-t border-[var(--color-border)]">
          <span className="flex-1 truncate text-[11px] font-mono text-[color:var(--color-text-secondary)]" title={selected ?? undefined} data-testid="picker-selected">
            {selected ? formatAddress(selected, nameOf) : "Choose a folder"}
          </span>
          <button type="button" onClick={() => void createFolder()} disabled={!selected} className="flex items-center gap-1 text-xs px-2 py-1 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)] disabled:opacity-50" data-testid="picker-new-folder">
            <FolderPlus className="h-3.5 w-3.5" /> New folder
          </button>
          <button type="button" onClick={onCancel} className="text-xs px-2 py-1 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)]">
            Cancel
          </button>
          <button type="button" onClick={() => selected && onPick(selected)} disabled={!selected} className="text-xs px-3 py-1 rounded bg-[var(--color-primary)] text-white disabled:opacity-50" data-testid="picker-choose">
            Choose
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * A folder text field with "Browse…" (the native picker, local folders —
 * kept as it was) and "Pick…" (the in-app picker, with connections and
 * New folder). `toValue` / `fromValue` convert between the stored value and
 * pane paths (e.g. a connector path relative to a connection's root).
 */
export function FolderField({
  value,
  onChange,
  placeholder,
  testId,
  pickerTitle = "Choose a folder",
  allowLocal = true,
  allowConnections = true,
  lockRoot = null,
  nativeBrowse = true,
  toValue = (p) => p,
  fromValue = (v) => v || null,
  inputClassName,
  inputId,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  testId?: string;
  pickerTitle?: string;
  allowLocal?: boolean;
  allowConnections?: boolean;
  lockRoot?: string | null;
  nativeBrowse?: boolean;
  toValue?: (panePath: string) => string;
  fromValue?: (value: string) => string | null;
  inputClassName?: string;
  /** `id` of the text field (for a `<label htmlFor>`). */
  inputId?: string;
}) {
  const [open, setOpen] = useState(false);
  const btn = "text-xs px-2 py-1 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)] shrink-0";
  return (
    <div className="flex gap-1">
      <input className={inputClassName ?? "flex-1 min-w-0 px-2 py-1 text-xs border rounded bg-[var(--color-bg)] border-[var(--color-border)]"} id={inputId} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} data-testid={testId} />
      {nativeBrowse && allowLocal && !lockRoot && (
        <button
          type="button"
          className={btn}
          onClick={async () => {
            const p = await pickFolder(pickerTitle);
            if (p) onChange(p);
          }}
        >
          Browse…
        </button>
      )}
      <button type="button" className={btn} onClick={() => setOpen(true)} data-testid={testId ? `${testId}-pick` : undefined} title="Pick a folder here or on a connection, or create one">
        Pick…
      </button>
      {open && (
        <RemoteFolderPicker
          title={pickerTitle}
          initialPath={fromValue(value)}
          allowLocal={allowLocal}
          allowConnections={allowConnections}
          lockRoot={lockRoot}
          onCancel={() => setOpen(false)}
          onPick={(p) => {
            setOpen(false);
            onChange(toValue(p));
          }}
        />
      )}
    </div>
  );
}
