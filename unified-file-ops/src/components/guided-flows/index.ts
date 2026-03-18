/**
 * Guided Flows Index (T-056)
 *
 * All 15 guided flow wizards plus the shared shell.
 */

// Shared framework
export { WizardShell, type WizardStepDef } from "./wizard-shell";

// Cloud connections (Wizards 1-4)
export {
  ConnectOneDriveWizard,
  ConnectGDriveWizard,
  ConnectDropboxWizard,
  ConnectGCSWizard,
} from "./cloud-connect-flows";

// Server/local connections (Wizards 4-6)
export {
  ConnectLocalSyncWizard,
  ConnectSMBWizard,
  ConnectSFTPWizard,
} from "./connection-flows";

// Operations (Wizards 7-10)
export {
  StartTransferWizard,
  CreateSyncWizard,
  ResolveInterruptionWizard,
  HandleCompatWarningWizard,
} from "./operation-flows";

// Utilities (Wizards 11-15 + bonus)
export {
  RestoreNamesWizard,
  ExportReportWizard,
  TransferToComputerWizard,
  CopyToExternalWizard,
  OldToNewDriveWizard,
  BackupFolderWizard,
  MigrateComputersWizard,
} from "./utility-flows";
