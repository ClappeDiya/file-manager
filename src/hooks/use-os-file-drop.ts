/**
 * Files dragged in from the OS (Finder, Explorer, a desktop).
 *
 * The webview never sees these as HTML5 drops with paths, so Tauri's
 * drag-drop event is the only source: it reports physical-pixel
 * positions and absolute paths. The pane under the cursor (an element
 * carrying `data-os-drop-dir`) is highlighted while hovering and receives
 * a copy on drop — copy, not move, as the OS does across locations.
 */
import { useEffect } from "react";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { reportOperationFailure } from "@/lib/op-error-toast";
import { isRemotePath, parentLocation } from "@/lib/location";
import { resolveItems, statLocation, transferItems } from "@/lib/location-provider";

export const OS_DROP_ATTR = "data-os-drop-dir";
export const OS_DROP_HOVER_ATTR = "data-os-drop-hover";

type Position = { x: number; y: number };

/** The drop target under a physical-pixel position, if any. */
export function dropTargetAt(pos: Position, scale: number, doc: Document = document): HTMLElement | null {
  const el = doc.elementFromPoint(pos.x / (scale || 1), pos.y / (scale || 1));
  return (el?.closest(`[${OS_DROP_ATTR}]`) as HTMLElement | null) ?? null;
}

function setHover(target: HTMLElement | null, doc: Document = document) {
  doc.querySelectorAll(`[${OS_DROP_HOVER_ATTR}]`).forEach((el) => {
    if (el !== target) el.removeAttribute(OS_DROP_HOVER_ATTR);
  });
  target?.setAttribute(OS_DROP_HOVER_ATTR, "");
}

export type OsDropEvent =
  | { type: "enter"; paths: string[]; position: Position }
  | { type: "over"; position: Position }
  | { type: "drop"; paths: string[]; position: Position }
  | { type: "leave" };

/** Handle one drag-drop event; returns the copy it started, if any. */
export async function handleOsDrop(event: OsDropEvent, scale: number): Promise<{ dest: string; paths: string[] } | null> {
  if (event.type === "leave") {
    setHover(null);
    return null;
  }
  const target = dropTargetAt(event.position, scale);
  if (event.type !== "drop") {
    setHover(target);
    return null;
  }
  setHover(null);
  const dest = target?.getAttribute(OS_DROP_ATTR);
  // Dropping a pane's own files back onto it would copy them onto themselves.
  const paths = event.paths.filter((p) => dest && parentLocation(p) !== dest);
  if (!dest || paths.length === 0) return null;
  try {
    if (isRemotePath(dest)) {
      // Dropped on a connection folder: upload through the transfer queue.
      const items = await Promise.all(paths.map(async (p) => (await statLocation(p)) ?? (await resolveItems([p]))[0]));
      await transferItems(items, dest, "copy");
    } else {
      await tauriInvoke("copy_files", { sourcePaths: paths, destDir: dest });
    }
    dispatchRefresh([dest]);
  } catch (err) {
    reportOperationFailure("Copy dropped files", err);
  }
  return { dest, paths };
}

/** Mount once: routes OS file drops to the pane under the cursor. */
export function useOsFileDrop() {
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((e) => void handleOsDrop(e.payload as OsDropEvent, window.devicePixelRatio)),
      )
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((e) => console.warn("OS file drop unavailable:", e));
    return () => {
      cancelled = true;
      unlisten?.();
      setHover(null);
    };
  }, []);
}
