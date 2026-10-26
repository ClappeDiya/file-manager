/**
 * Guided Flow Router (T-056)
 *
 * Maps guided flow IDs to their wizard components.
 * Accessible from menu, context menu, sidebar, command palette, and AI assistant.
 */
import { useUIStore } from "@/stores/ui-store";
import {
  ConnectOneDriveWizard,
  ConnectGDriveWizard,
  ConnectDropboxWizard,
  ConnectGCSWizard,
  ConnectLocalSyncWizard,
  ConnectSMBWizard,
  ConnectSFTPWizard,
  StartTransferWizard,
  CreateSyncWizard,
  ResolveInterruptionWizard,
  HandleCompatWarningWizard,
  RestoreNamesWizard,
  ExportReportWizard,
  TransferToComputerWizard,
  CopyToExternalWizard,
  OldToNewDriveWizard,
  BackupFolderWizard,
  MigrateComputersWizard,
} from "./index";

export { GUIDED_FLOW_IDS, GUIDED_FLOW_META, type GuidedFlowId } from "./flow-meta";

/**
 * Renders the active guided flow wizard, if any.
 */
export function GuidedFlowRouter() {
  const activeFlow = useUIStore((s) => s.activeGuidedFlow);

  if (!activeFlow) return null;

  switch (activeFlow) {
    case "connect-onedrive":
      return <ConnectOneDriveWizard />;
    case "connect-gdrive":
      return <ConnectGDriveWizard />;
    case "connect-dropbox":
      return <ConnectDropboxWizard />;
    case "connect-gcs":
      return <ConnectGCSWizard />;
    case "connect-local-sync":
      return <ConnectLocalSyncWizard />;
    case "connect-smb":
      return <ConnectSMBWizard />;
    case "connect-sftp":
      return <ConnectSFTPWizard />;
    case "start-transfer":
      return <StartTransferWizard />;
    case "create-sync":
      return <CreateSyncWizard />;
    case "resolve-interruption":
      return <ResolveInterruptionWizard />;
    case "handle-compat-warning":
      return <HandleCompatWarningWizard />;
    case "restore-names":
      return <RestoreNamesWizard />;
    case "export-report":
      return <ExportReportWizard />;
    case "transfer-to-computer":
      return <TransferToComputerWizard />;
    case "copy-to-external":
      return <CopyToExternalWizard />;
    case "old-to-new-drive":
      return <OldToNewDriveWizard />;
    case "backup-folder":
      return <BackupFolderWizard />;
    case "migrate-computers":
      return <MigrateComputersWizard />;
    default:
      return null;
  }
}
