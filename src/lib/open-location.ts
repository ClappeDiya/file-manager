/**
 * "Open source" / "Open destination" on history rows: a local folder is
 * revealed in the system file manager, a folder on a saved connection opens
 * in the connection browser. A location with neither (a cloud store that has
 * no saved connection) says so instead of pretending.
 */
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { useConnectionBrowse } from "@/stores/connection-browse-store";
import type { BackupDestination, SavedDestination } from "@/stores/backup-store";

export type OpenTarget =
  | { kind: "local"; path: string }
  | { kind: "connection"; connectionId: string; path: string }
  | { kind: "none"; reason: string };

export const localTarget = (path: string | null | undefined): OpenTarget =>
  path?.trim() ? { kind: "local", path } : { kind: "none", reason: "No folder is set." };

/** Where a backup destination lives. A library entry follows the saved
 *  destination it names (`library` is the saved-destinations list). */
export function destinationTarget(dest: BackupDestination, library: SavedDestination[] = []): OpenTarget {
  switch (dest.type) {
    case "local":
      return localTarget(dest.path);
    case "connection":
      return { kind: "connection", connectionId: dest.connection_id, path: dest.remote_path || "/" };
    case "library": {
      const saved = library.find((s) => s.id === dest.destination_id);
      if (!saved) return { kind: "none", reason: "This destination is no longer in the destination library." };
      const inner = destinationTarget(saved.destination, library);
      if (inner.kind === "local" && dest.remote_path) return { kind: "local", path: `${inner.path.replace(/[/\\]+$/, "")}/${dest.remote_path.replace(/^[/\\]+/, "")}` };
      if (inner.kind === "connection" && dest.remote_path) return { ...inner, path: `${inner.path.replace(/\/+$/, "")}/${dest.remote_path.replace(/^\/+/, "")}` };
      return inner;
    }
    default:
      return { kind: "none", reason: "This cloud store has no saved connection to browse; use the job's Restore view to see its files." };
  }
}

/** Whether a target can be opened at all. */
export const canOpen = (t: OpenTarget): boolean => t.kind !== "none";

/** Open a target; resolves with an error message when the system refused. */
export async function openTarget(t: OpenTarget): Promise<string | null> {
  if (t.kind === "local") {
    try {
      await tauriInvoke("reveal_in_os", { path: t.path });
      return null;
    } catch (e) {
      return errorText(e);
    }
  }
  if (t.kind === "connection") {
    useConnectionBrowse.getState().open(t.connectionId, t.path);
    return null;
  }
  return t.reason;
}
