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

/**
 * All available guided flow IDs.
 * These can be triggered from anywhere in the app.
 */
export const GUIDED_FLOW_IDS = [
  "connect-onedrive",
  "connect-gdrive",
  "connect-dropbox",
  "connect-gcs",
  "connect-local-sync",
  "connect-smb",
  "connect-sftp",
  "start-transfer",
  "create-sync",
  "resolve-interruption",
  "handle-compat-warning",
  "restore-names",
  "export-report",
  "transfer-to-computer",
  "copy-to-external",
  "old-to-new-drive",
  "backup-folder",
  "migrate-computers",
] as const;

export type GuidedFlowId = (typeof GUIDED_FLOW_IDS)[number];

/**
 * Metadata for each guided flow (for menus, command palette, etc.)
 */
export const GUIDED_FLOW_META: Record<
  GuidedFlowId,
  { label: string; description: string; category: string }
> = {
  "connect-onedrive": {
    label: "Connect OneDrive",
    description: "Link your Microsoft OneDrive account",
    category: "Cloud",
  },
  "connect-gdrive": {
    label: "Connect Google Drive",
    description: "Link your Google Drive account",
    category: "Cloud",
  },
  "connect-dropbox": {
    label: "Connect Dropbox",
    description: "Link your Dropbox account",
    category: "Cloud",
  },
  "connect-gcs": {
    label: "Connect Google Cloud Storage",
    description: "Link GCS via S3-compatible HMAC keys",
    category: "Cloud",
  },
  "connect-local-sync": {
    label: "Set Up Local Sync",
    description: "Keep two local folders in sync",
    category: "Sync",
  },
  "connect-smb": {
    label: "Connect to Windows Share",
    description: "Access files on another computer or NAS",
    category: "Servers",
  },
  "connect-sftp": {
    label: "Connect via SFTP",
    description: "Securely access files on a remote server",
    category: "Servers",
  },
  "start-transfer": {
    label: "Transfer Files",
    description: "Copy or move files between locations",
    category: "Transfer",
  },
  "create-sync": {
    label: "Create Sync Pair",
    description: "Set up automatic folder synchronization",
    category: "Sync",
  },
  "resolve-interruption": {
    label: "Resolve Interrupted Operation",
    description: "Resume or fix a stopped transfer",
    category: "Recovery",
  },
  "handle-compat-warning": {
    label: "Fix File Name Issues",
    description: "Handle cross-platform compatibility warnings",
    category: "Compatibility",
  },
  "restore-names": {
    label: "Restore Original Names",
    description: "Undo automatic file renames",
    category: "Compatibility",
  },
  "export-report": {
    label: "Export Report",
    description: "Save a report of file operations",
    category: "Reports",
  },
  "transfer-to-computer": {
    label: "Send to Another Computer",
    description: "Transfer files to a computer on your network",
    category: "Transfer",
  },
  "copy-to-external": {
    label: "Copy to External Drive",
    description: "Copy files to USB, SD card, or external drive",
    category: "Transfer",
  },
  "old-to-new-drive": {
    label: "Migrate Drive",
    description: "Move all files from an old drive to a new one",
    category: "Migration",
  },
  "backup-folder": {
    label: "Back Up Folder",
    description: "Create a safe copy of important files",
    category: "Backup",
  },
  "migrate-computers": {
    label: "Migrate from Computer",
    description: "Transfer files and settings from your old computer",
    category: "Migration",
  },
};

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
