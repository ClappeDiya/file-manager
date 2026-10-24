/** The log viewer as a dialog, opened from the toolbar, the command palette and Settings → Logging (Advanced only). */
import { X } from "lucide-react";
import { LogViewer } from "@/components/log-viewer";
import { useLogViewer } from "@/stores/log-viewer-store";
import { useUIStore } from "@/stores/ui-store";

export function LogViewerDialog() {
  const open = useLogViewer((s) => s.open);
  const setOpen = useLogViewer((s) => s.setOpen);
  const advanced = useUIStore((s) => s.appMode === "advanced");
  if (!open || !advanced) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Log viewer"
      data-testid="log-viewer-backdrop"
      onClick={() => setOpen(false)}
    >
      <div className="w-[min(1100px,95vw)] max-h-[92vh] overflow-auto rounded-lg shadow-2xl border border-[var(--color-border)] bg-[var(--color-surface)] relative" onClick={(e) => e.stopPropagation()}>
        <button
          className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
          onClick={() => setOpen(false)}
          aria-label="Close log viewer"
        >
          <X className="w-4 h-4" />
        </button>
        <LogViewer />
      </div>
    </div>
  );
}
