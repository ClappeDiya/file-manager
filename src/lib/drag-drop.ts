/**
 * Drag-and-drop requests waiting for the inline confirmation
 * (`DragDropConfirm`) and their Safety Interlock intent.
 */
import type { OperationIntent } from "@/lib/safety";
import type { TransferMode } from "@/lib/location-provider";

export interface DropRequest {
  /** Pane paths being dropped. */
  sources: string[];
  /** Folder they are dropped on. */
  destDir: string;
  mode: TransferMode;
  /** Shown as "to <destLabel>". */
  destLabel: string;
  /** Total size when known (0 = unknown). */
  totalBytes: number;
  /** Files affected, folders counted through (default: the item count). */
  affectedFiles?: number;
  /** Still being measured: not assessed or confirmable yet. */
  measuring?: boolean;
  /** Measuring failed: size unknown, and the counts are the top-level items only. */
  unmeasured?: boolean;
}

/** `req` once measured: `m` is what it affects, or null when measuring failed. */
export function measuredDrop(req: DropRequest, m: { files: number; bytes: number } | null): DropRequest {
  return m ? { ...req, measuring: false, unmeasured: false, affectedFiles: m.files, totalBytes: m.bytes } : { ...req, measuring: false, unmeasured: true };
}

/**
 * Whether confirming the drop counts as the Safety Interlock's
 * confirmation: only when it was assessed on what the drop really affects.
 * An unmeasured drop (possibly understated) goes through the interlock again.
 */
export function dropPreconfirmed(req: DropRequest, assessment: unknown): boolean {
  return assessment !== null && assessment !== undefined && !req.unmeasured;
}

/** The interlock intent of a drop. */
export function dropIntent(req: DropRequest): OperationIntent {
  const n = req.sources.length;
  return {
    engine: "fs",
    kind: req.mode === "move" ? "move" : "copy",
    affected_files: req.affectedFiles ?? n,
    total_bytes: req.totalBytes,
    subject_path: req.destDir,
    summary: `${req.mode === "move" ? "Move" : "Copy"} ${n} item${n === 1 ? "" : "s"} → ${req.destLabel}`,
  };
}
