/**
 * Utility Guided Flows (T-056)
 *
 * Wizards 11-15:
 * 11. Restore Names
 * 12. Export Report
 * 13. Transfer to Computer
 * 14. Copy to External
 * 15. Old-to-New Drive Migration
 *
 * Plus bonus flows:
 * - Backup Folder
 * - Migrate Computers
 */
import { useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import {
  RotateCcw,
  FileText,
  Monitor,
  HardDrive,
  ArrowRight,
  Check,
  Download,
  Shield,
  FolderOpen,
  Copy,
} from "lucide-react";

// ──────────────────────────────────────────────
// Wizard 11: Restore Names
// ──────────────────────────────────────────────

export function RestoreNamesWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);

  const renamedFiles = [
    { current: "project_final.doc", original: "project:final.doc" },
    { current: "_CON.txt", original: "CON.txt" },
    { current: "file_with_trailing_space.pdf", original: "file with trailing space .pdf" },
  ];

  const steps: WizardStepDef[] = [
    { id: "select", label: "Select Files", helpText: "Choose which files to restore to their original names." },
    { id: "preview", label: "Preview", helpText: "Review the name changes before applying." },
    { id: "apply", label: "Restore" },
  ];

  return (
    <WizardShell
      title="Restore Original Names"
      subtitle="Undo automatic file renames"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "rename",
          summary: `Restored original names for ${selectedFiles.length} file${selectedFiles.length !== 1 ? "s" : ""}.`,
          paths: selectedFiles,
          undoable: true,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Restore Names"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Files with renamed names
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            These files were automatically renamed for compatibility. Select the
            ones you want to restore to their original names.
          </p>
          <div className="space-y-2">
            {renamedFiles.map((f) => {
              const selected = selectedFiles.includes(f.current);
              return (
                <button
                  key={f.current}
                  className={cn(
                    "flex items-center gap-3 w-full p-3 rounded-lg border transition-all text-left",
                    selected ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)]",
                  )}
                  onClick={() =>
                    setSelectedFiles((prev) =>
                      prev.includes(f.current) ? prev.filter((p) => p !== f.current) : [...prev, f.current],
                    )
                  }
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-mono text-[color:var(--color-text)]">{f.current}</p>
                    <p className="text-xs text-[color:var(--color-text-tertiary)] mt-0.5">Original: {f.original}</p>
                  </div>
                  {selected && <Check className="h-4 w-4 text-[color:var(--color-primary)]" aria-hidden="true" />}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            Preview name changes
          </h3>
          <div className="space-y-2">
            {renamedFiles
              .filter((f) => selectedFiles.includes(f.current))
              .map((f) => (
                <div key={f.current} className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
                  <div className="flex items-center gap-2 text-sm">
                    <span className="font-mono text-[color:var(--color-text-secondary)] line-through">{f.current}</span>
                    <ArrowRight className="h-3 w-3 text-[color:var(--color-text-tertiary)]" aria-hidden="true" />
                    <span className="font-mono text-[color:var(--color-text)]">{f.original}</span>
                  </div>
                </div>
              ))}
          </div>
          {selectedFiles.length === 0 && (
            <p className="text-sm text-[color:var(--color-text-secondary)] text-center py-4">
              No files selected. Go back to select files to restore.
            </p>
          )}
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <RotateCcw className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to restore
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {selectedFiles.length} file{selectedFiles.length !== 1 ? "s" : ""} will
            be renamed back to {selectedFiles.length !== 1 ? "their" : "its"} original
            name{selectedFiles.length !== 1 ? "s" : ""}. This can be undone.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 12: Export Report
// ──────────────────────────────────────────────

export function ExportReportWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [reportType, setReportType] = useState<"transfer" | "sync" | "compat" | "activity">("transfer");
  const [format, setFormat] = useState<"csv" | "json" | "pdf">("csv");

  const steps: WizardStepDef[] = [
    { id: "type", label: "Report Type", helpText: "Choose which type of report to export." },
    { id: "format", label: "Format", helpText: "Choose the file format for your report." },
    { id: "export", label: "Export" },
  ];

  return (
    <WizardShell
      title="Export Report"
      subtitle="Save a report of your file operations"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "transfer",
          summary: `Exported ${reportType} report as ${format.toUpperCase()}.`,
          paths: [],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Export"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            What kind of report?
          </h3>
          <div className="space-y-2">
            {[
              { id: "transfer" as const, label: "Transfer History", desc: "All file transfers, including status, speed, and errors" },
              { id: "sync" as const, label: "Sync Reports", desc: "Sync pair activity, conflicts resolved, and changes made" },
              { id: "compat" as const, label: "Compatibility Report", desc: "File name issues detected and fixes applied" },
              { id: "activity" as const, label: "Activity Log", desc: "Full history of all file operations" },
            ].map((r) => (
              <button
                key={r.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  reportType === r.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setReportType(r.id)}
              >
                <FileText className={cn("h-5 w-5 shrink-0 mt-0.5", reportType === r.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{r.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{r.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            Choose format
          </h3>
          <div className="space-y-2">
            {[
              { id: "csv" as const, label: "CSV (Spreadsheet)", desc: "Open in Excel, Google Sheets, or any spreadsheet app" },
              { id: "json" as const, label: "JSON (Data)", desc: "Machine-readable format for developers and automation" },
              { id: "pdf" as const, label: "PDF (Document)", desc: "Printable document with formatted tables" },
            ].map((f) => (
              <button
                key={f.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  format === f.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setFormat(f.id)}
              >
                <Download className={cn("h-5 w-5 shrink-0 mt-0.5", format === f.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{f.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{f.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <Download className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to export
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            Your {reportType} report will be saved as a {format.toUpperCase()} file.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 13: Transfer to Computer
// ──────────────────────────────────────────────

export function TransferToComputerWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [targetComputer, setTargetComputer] = useState("");
  const [selectedFolders, setSelectedFolders] = useState<string[]>([]);

  const steps: WizardStepDef[] = [
    { id: "find", label: "Find Computer", helpText: "Select the computer you want to transfer files to. Both computers must be on the same network.", validate: () => (targetComputer ? null : "Please select or enter a computer.") },
    { id: "select", label: "Select Files", helpText: "Choose which files or folders to send." },
    { id: "transfer", label: "Transfer" },
  ];

  return (
    <WizardShell
      title="Transfer to Another Computer"
      subtitle="Send files directly to another computer on your network"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "transfer",
          summary: `Started transfer of ${selectedFolders.length} item${selectedFolders.length !== 1 ? "s" : ""} to "${targetComputer}".`,
          paths: selectedFolders,
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Transfer"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Which computer?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Make sure both computers are on the same network and running File Manager.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Computer name or IP</span>
            <input
              type="text"
              value={targetComputer}
              onChange={(e) => setTargetComputer(e.target.value)}
              placeholder="e.g., Johns-MacBook or 192.168.1.50"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
          <div className="mt-4 p-3 rounded-lg bg-[var(--color-bg-secondary)]">
            <p className="text-xs text-[color:var(--color-text-secondary)]">
              <Monitor className="h-3 w-3 inline mr-1" aria-hidden="true" />
              Nearby computers will appear here when detected.
            </p>
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            What would you like to send?
          </h3>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Files or folders</span>
            <input
              type="text"
              placeholder="Enter paths separated by commas"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              onChange={(e) => setSelectedFolders(e.target.value.split(",").map((s) => s.trim()).filter(Boolean))}
            />
          </label>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <Monitor className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to send
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {selectedFolders.length} item{selectedFolders.length !== 1 ? "s" : ""} will
            be sent to "{targetComputer}". The transfer uses an encrypted connection.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 14: Copy to External Drive
// ──────────────────────────────────────────────

export function CopyToExternalWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [externalDrive, setExternalDrive] = useState("");

  const steps: WizardStepDef[] = [
    { id: "source", label: "Choose Files", validate: () => (sourcePath ? null : "Please choose files to copy.") },
    { id: "drive", label: "Choose Drive", validate: () => (externalDrive ? null : "Please select an external drive.") },
    { id: "copy", label: "Copy" },
  ];

  return (
    <WizardShell
      title="Copy to External Drive"
      subtitle="Copy files to a USB drive, SD card, or other external storage"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "copy",
          summary: `Copied files from "${sourcePath}" to external drive "${externalDrive}".`,
          paths: [sourcePath, externalDrive],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Copy"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            What do you want to copy?
          </h3>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Source folder or files</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={sourcePath}
                onChange={(e) => setSourcePath(e.target.value)}
                placeholder="/Users/you/Documents"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
              <button className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)]" onClick={() => setSourcePath("/Users/me/Documents")}>
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            Which external drive?
          </h3>
          <div className="space-y-2">
            {["USB Drive (E:)", "SD Card (F:)", "External HDD (G:)"].map((drive) => (
              <button
                key={drive}
                className={cn(
                  "flex items-center gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  externalDrive === drive ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)]",
                )}
                onClick={() => setExternalDrive(drive)}
              >
                <HardDrive className={cn("h-5 w-5", externalDrive === drive ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <span className="text-sm text-[color:var(--color-text)]">{drive}</span>
                {externalDrive === drive && <Check className="h-4 w-4 text-[color:var(--color-primary)] ml-auto" aria-hidden="true" />}
              </button>
            ))}
          </div>
          <label className="block mt-4">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Or enter path manually</span>
            <input
              type="text"
              value={externalDrive}
              onChange={(e) => setExternalDrive(e.target.value)}
              placeholder="/Volumes/MyDrive"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <Copy className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to copy
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            Files from "{sourcePath}" will be copied to "{externalDrive}".
            Your originals will not be modified.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 15: Old-to-New Drive Migration
// ──────────────────────────────────────────────

export function OldToNewDriveWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [oldDrive, setOldDrive] = useState("");
  const [newDrive, setNewDrive] = useState("");
  const [verifyEnabled, setVerifyEnabled] = useState(true);

  const steps: WizardStepDef[] = [
    { id: "old", label: "Old Drive", validate: () => (oldDrive ? null : "Please select the old drive.") },
    { id: "new", label: "New Drive", validate: () => {
      if (!newDrive) return "Please select the new drive.";
      if (newDrive === oldDrive) return "The new drive must be different from the old drive.";
      return null;
    }},
    { id: "options", label: "Options" },
    { id: "migrate", label: "Migrate" },
  ];

  return (
    <WizardShell
      title="Drive Migration"
      subtitle="Move all your files from an old drive to a new one"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "move",
          summary: `Started drive migration from "${oldDrive}" to "${newDrive}".`,
          paths: [oldDrive, newDrive],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Migration"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Select the old drive
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            This is the drive you're moving files FROM. All files will be copied to the new drive.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Old drive path</span>
            <input type="text" value={oldDrive} onChange={(e) => setOldDrive(e.target.value)} placeholder="/Volumes/OldDrive" className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]" />
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Select the new drive
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            This is the drive you're moving files TO. Make sure it has enough space.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">New drive path</span>
            <input type="text" value={newDrive} onChange={(e) => setNewDrive(e.target.value)} placeholder="/Volumes/NewDrive" className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]" />
          </label>
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            Migration options
          </h3>
          <label className="flex items-center gap-3 p-4 rounded-lg bg-[var(--color-bg-secondary)] cursor-pointer">
            <input type="checkbox" checked={verifyEnabled} onChange={(e) => setVerifyEnabled(e.target.checked)} className="h-4 w-4 rounded" />
            <div>
              <span className="text-sm text-[color:var(--color-text)]">Verify every file after copying</span>
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-0.5">
                Compares each file to ensure no data was lost. Takes longer but guarantees accuracy.
              </p>
            </div>
          </label>
          <div className="mt-4 p-3 rounded-lg bg-[var(--color-primary)]/5 border border-[var(--color-primary)]/20">
            <p className="text-xs text-[color:var(--color-text-secondary)]">
              <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
              Your files on the old drive will NOT be deleted during migration. You can delete
              them manually after confirming everything copied correctly.
            </p>
          </div>
        </div>
      )}

      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <HardDrive className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to migrate
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            All files from "{oldDrive}" will be copied to "{newDrive}".
            {verifyEnabled ? " Each file will be verified after copying." : ""}
            {" "}Original files will NOT be deleted.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Bonus: Backup Folder Wizard
// ──────────────────────────────────────────────

export function BackupFolderWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [backupDest, setBackupDest] = useState("");
  const [schedule, setSchedule] = useState<"once" | "daily" | "weekly">("once");

  const steps: WizardStepDef[] = [
    { id: "source", label: "Source", validate: () => (sourcePath ? null : "Please choose a folder to back up.") },
    { id: "dest", label: "Backup Location", validate: () => (backupDest ? null : "Please choose a backup destination.") },
    { id: "schedule", label: "Schedule" },
    { id: "start", label: "Start" },
  ];

  return (
    <WizardShell
      title="Back Up a Folder"
      subtitle="Create a safe copy of your important files"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "copy",
          summary: `Started backup of "${sourcePath}" to "${backupDest}" (${schedule}).`,
          paths: [sourcePath, backupDest],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Backup"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">Which folder do you want to back up?</h3>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Folder</span>
            <input type="text" value={sourcePath} onChange={(e) => setSourcePath(e.target.value)} placeholder="/Users/you/Documents" className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]" />
          </label>
        </div>
      )}
      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">Where should the backup go?</h3>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Backup destination</span>
            <input type="text" value={backupDest} onChange={(e) => setBackupDest(e.target.value)} placeholder="/Volumes/External/Backups" className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]" />
          </label>
        </div>
      )}
      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">How often?</h3>
          <div className="space-y-2">
            {[
              { id: "once" as const, label: "Just once", desc: "Create a single backup now" },
              { id: "daily" as const, label: "Daily", desc: "Back up every day" },
              { id: "weekly" as const, label: "Weekly", desc: "Back up once a week" },
            ].map((s) => (
              <button key={s.id} className={cn("flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left", schedule === s.id ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)]")} onClick={() => setSchedule(s.id)}>
                <div><span className="text-sm font-medium text-[color:var(--color-text)] block">{s.label}</span><span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{s.desc}</span></div>
              </button>
            ))}
          </div>
        </div>
      )}
      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <Shield className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">Ready to back up</h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            "{sourcePath}" will be backed up to "{backupDest}". Schedule: {schedule}. Your original files will not be modified.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Bonus: Migrate Computers Wizard
// ──────────────────────────────────────────────

export function MigrateComputersWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourceComputer, setSourceComputer] = useState("");
  const [selectedCategories, setSelectedCategories] = useState<string[]>(["Documents", "Desktop", "Pictures"]);

  const categories = ["Documents", "Desktop", "Pictures", "Music", "Videos", "Downloads", "Application Settings"];

  const steps: WizardStepDef[] = [
    { id: "source", label: "Old Computer", validate: () => (sourceComputer ? null : "Please enter the old computer's name or IP.") },
    { id: "what", label: "What to Migrate" },
    { id: "review", label: "Review" },
    { id: "start", label: "Migrate" },
  ];

  return (
    <WizardShell
      title="Migrate from Another Computer"
      subtitle="Transfer your files and settings from your old computer"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "transfer",
          summary: `Started computer migration from "${sourceComputer}". Migrating: ${selectedCategories.join(", ")}.`,
          paths: selectedCategories,
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Migration"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">Connect to your old computer</h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">Both computers must be on the same network and running File Manager.</p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Computer name or IP</span>
            <input type="text" value={sourceComputer} onChange={(e) => setSourceComputer(e.target.value)} placeholder="Old-MacBook or 192.168.1.25" className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]" />
          </label>
        </div>
      )}
      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">What would you like to bring over?</h3>
          <div className="space-y-2">
            {categories.map((cat) => {
              const selected = selectedCategories.includes(cat);
              return (
                <button key={cat} className={cn("flex items-center gap-3 w-full p-3 rounded-lg border transition-all text-left", selected ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)]")} onClick={() => setSelectedCategories((prev) => prev.includes(cat) ? prev.filter((c) => c !== cat) : [...prev, cat])}>
                  <FolderOpen className={cn("h-5 w-5", selected ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                  <span className="text-sm text-[color:var(--color-text)] flex-1">{cat}</span>
                  {selected && <Check className="h-4 w-4 text-[color:var(--color-primary)]" aria-hidden="true" />}
                </button>
              );
            })}
          </div>
        </div>
      )}
      {step === 2 && (
        <div className="space-y-3">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">Review migration</h3>
          <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
            <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">From</span>
            <p className="text-sm text-[color:var(--color-text)] mt-0.5">{sourceComputer}</p>
          </div>
          <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
            <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Categories</span>
            <p className="text-sm text-[color:var(--color-text)] mt-0.5">{selectedCategories.join(", ")}</p>
          </div>
          <div className="p-3 rounded-lg bg-[var(--color-primary)]/5 border border-[var(--color-primary)]/20">
            <p className="text-xs text-[color:var(--color-text-secondary)]">
              Files will be copied to the corresponding folders on this computer. Nothing on the old computer will be deleted.
            </p>
          </div>
        </div>
      )}
      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <Monitor className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">Ready to migrate</h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {selectedCategories.length} categories will be transferred from "{sourceComputer}" to this computer.
          </p>
        </div>
      )}
    </WizardShell>
  );
}
