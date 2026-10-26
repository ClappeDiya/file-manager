/**
 * Renders the mount screens that the toolbar, the command palette, the
 * connection manager and the tray open (`useMountsUi`): the Mounts list as a
 * modal and the mount dialog on top of it.
 */
import { X } from "lucide-react";
import MountDriveDialog from "@/components/mount-drive-dialog";
import { MountsPanel } from "@/components/mounts-panel";
import { useMountsUi } from "@/stores/mounts-ui-store";

export function MountsHost() {
  const panelOpen = useMountsUi((s) => s.panelOpen);
  const dialogOpen = useMountsUi((s) => s.dialogOpen);
  const connectionId = useMountsUi((s) => s.dialogConnectionId);
  const editing = useMountsUi((s) => s.editing);
  const closePanel = useMountsUi((s) => s.closePanel);
  const closeDialog = useMountsUi((s) => s.closeDialog);
  const bump = useMountsUi((s) => s.bump);

  return (
    <>
      {panelOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-label="Mounts"
          data-testid="mounts-backdrop"
          onClick={closePanel}
        >
          <div
            className="relative w-[min(760px,95vw)] h-[min(620px,90vh)] rounded-lg shadow-2xl overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)]"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              className="absolute top-3 right-4 z-10 h-7 w-7 flex items-center justify-center rounded text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover)]"
              onClick={closePanel}
              aria-label="Close Mounts"
            >
              <X className="w-4 h-4" />
            </button>
            <MountsPanel />
          </div>
        </div>
      )}
      <MountDriveDialog
        open={dialogOpen}
        onClose={closeDialog}
        connectionId={connectionId}
        editing={editing}
        onChanged={bump}
      />
    </>
  );
}
