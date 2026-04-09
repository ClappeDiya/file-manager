/**
 * Operation Guided Flows (T-056)
 *
 * Wizards 7-10:
 * 7. Start Transfer
 * 8. Create Sync
 * 9. Resolve Interruption
 * 10. Handle Compatibility Warning
 *
 * Each wizard provides step-by-step guidance with validation
 * and contextual help.
 */
import { useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { pickFolder } from "@/lib/folder-picker";
import {
  ArrowUpDown,
  FolderOpen,
  Check,
  AlertTriangle,
  RefreshCw,
  Shield,
  RotateCcw,
} from "lucide-react";

// ──────────────────────────────────────────────
// Wizard 7: Start Transfer
// ──────────────────────────────────────────────

export function StartTransferWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [destPath, setDestPath] = useState("");
  const [transferMode, setTransferMode] = useState<"copy" | "move">("copy");
  const [verifyAfter, setVerifyAfter] = useState(false);

  const steps: WizardStepDef[] = [
    {
      id: "source",
      label: "What to Send",
      helpText: "Select the files or folder you want to transfer. You can also drag files here from the file manager.",
      validate: () => (sourcePath ? null : "Please select files to transfer."),
    },
    {
      id: "destination",
      label: "Where to Send",
      helpText: "Choose the destination folder. This can be a local folder, external drive, or connected server.",
      validate: () => (destPath ? null : "Please choose a destination."),
    },
    {
      id: "options",
      label: "Options",
      helpText: "Choose whether to copy (keep originals) or move (remove originals after transfer).",
    },
    {
      id: "confirm",
      label: "Start",
      helpText: "Review your transfer and start.",
    },
  ];

  return (
    <WizardShell
      title="Transfer Files"
      subtitle="Copy or move files from one place to another"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "transfer",
          summary: `Started ${transferMode === "copy" ? "copying" : "moving"} files from "${sourcePath}" to "${destPath}".`,
          paths: [sourcePath, destPath],
          undoable: transferMode === "move",
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Transfer"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            What would you like to transfer?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Enter the path to the files or folder you want to send.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Source</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={sourcePath}
                onChange={(e) => setSourcePath(e.target.value)}
                placeholder="/Users/you/Documents/project"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
              <button
                type="button"
                className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                onClick={async (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  const path = await pickFolder("Select source folder");
                  if (path) setSourcePath(path);
                }}
              >
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Where should the files go?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Choose the destination. This can be any connected location.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Destination</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={destPath}
                onChange={(e) => setDestPath(e.target.value)}
                placeholder="/Volumes/External/backup"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
              <button
                type="button"
                className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                onClick={async (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  const path = await pickFolder("Select destination folder");
                  if (path) setDestPath(path);
                }}
              >
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </label>
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            Transfer options
          </h3>
          <div className="space-y-2 mb-4">
            {[
              { id: "copy" as const, label: "Copy files", desc: "Keep the originals in place and create copies at the destination." },
              { id: "move" as const, label: "Move files", desc: "Transfer files to the destination and remove the originals." },
            ].map((m) => (
              <button
                key={m.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  transferMode === m.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setTransferMode(m.id)}
                aria-pressed={transferMode === m.id}
              >
                <ArrowUpDown className={cn("h-5 w-5 shrink-0 mt-0.5", transferMode === m.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{m.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{m.desc}</span>
                </div>
              </button>
            ))}
          </div>

          {transferMode === "move" && (
            <div className="p-3 rounded-lg bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/20 mb-4">
              <div className="flex items-start gap-2">
                <AlertTriangle className="h-4 w-4 text-[color:var(--color-warning)] shrink-0 mt-0.5" aria-hidden="true" />
                <p className="text-xs text-[color:var(--color-text-secondary)]">
                  Move will remove the original files after they're transferred.
                  You can undo this from the Activity feed.
                </p>
              </div>
            </div>
          )}

          <label className="flex items-center gap-3 p-3 rounded-lg bg-[var(--color-bg-secondary)] cursor-pointer">
            <input
              type="checkbox"
              checked={verifyAfter}
              onChange={(e) => setVerifyAfter(e.target.checked)}
              className="h-4 w-4 rounded border-[var(--color-border)]"
            />
            <div>
              <span className="text-sm text-[color:var(--color-text)]">Verify files after transfer</span>
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-0.5">
                Checks that every file was copied correctly. Takes longer but ensures accuracy.
              </p>
            </div>
          </label>
        </div>
      )}

      {step === 3 && (
        <div className="space-y-4">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Ready to transfer
          </h3>
          <div className="space-y-3">
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">From</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono">{sourcePath}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">To</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono">{destPath}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Action</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 capitalize">{transferMode}</p>
              {verifyAfter && <p className="text-xs text-[color:var(--color-primary)] mt-0.5">With verification</p>}
            </div>
          </div>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 8: Create Sync
// ──────────────────────────────────────────────

export function CreateSyncWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [name, setName] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [destPath, setDestPath] = useState("");
  const [schedule, setSchedule] = useState<"manual" | "hourly" | "daily" | "realtime">("manual");

  const steps: WizardStepDef[] = [
    {
      id: "name",
      label: "Name",
      helpText: "Give your sync pair a name so you can identify it later.",
      validate: () => (name ? null : "Please enter a name for this sync pair."),
    },
    {
      id: "folders",
      label: "Folders",
      helpText: "Choose the two folders to keep in sync.",
      validate: () => {
        if (!sourcePath || !destPath) return "Please choose both folders.";
        if (sourcePath === destPath) return "The two folders must be different.";
        return null;
      },
    },
    {
      id: "schedule",
      label: "Schedule",
      helpText: "Choose how often the sync runs. Real-time watches for changes as they happen.",
    },
    {
      id: "confirm",
      label: "Create",
    },
  ];

  return (
    <WizardShell
      title="Create Sync Pair"
      subtitle="Keep two folders automatically in sync"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "sync",
          summary: `Created sync pair "${name}" between "${sourcePath}" and "${destPath}" (${schedule}).`,
          paths: [sourcePath, destPath],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Create Sync"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Name your sync pair
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Choose a descriptive name so you can identify this sync pair later.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Sync name</span>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g., Documents Backup, Work Files Sync"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 1 && (
        <div className="space-y-4">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Choose the two folders
          </h3>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">First folder</span>
            <input
              type="text"
              value={sourcePath}
              onChange={(e) => setSourcePath(e.target.value)}
              placeholder="/Users/you/Documents"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Second folder</span>
            <input
              type="text"
              value={destPath}
              onChange={(e) => setDestPath(e.target.value)}
              placeholder="/Volumes/Backup/Documents"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            When should sync run?
          </h3>
          <div className="space-y-2">
            {[
              { id: "manual" as const, label: "Manual", desc: "Only sync when you click the Sync button" },
              { id: "hourly" as const, label: "Every hour", desc: "Check for changes every hour" },
              { id: "daily" as const, label: "Once a day", desc: "Sync once daily at a scheduled time" },
              { id: "realtime" as const, label: "Real-time", desc: "Sync changes as they happen (uses more resources)" },
            ].map((s) => (
              <button
                key={s.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  schedule === s.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setSchedule(s.id)}
              >
                <RefreshCw className={cn("h-5 w-5 shrink-0 mt-0.5", schedule === s.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{s.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{s.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === 3 && (
        <div className="space-y-4">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Review your sync pair
          </h3>
          <div className="space-y-3">
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Name</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5">{name}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Folders</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono">{sourcePath}</p>
              <p className="text-sm text-[color:var(--color-text)] font-mono">{destPath}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Schedule</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 capitalize">{schedule}</p>
            </div>
          </div>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 9: Resolve Interruption
// ──────────────────────────────────────────────

export function ResolveInterruptionWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [resolution, setResolution] = useState<"resume" | "restart" | "skip" | "cancel">("resume");

  const steps: WizardStepDef[] = [
    {
      id: "explain",
      label: "What Happened",
      helpText: "This wizard helps you recover from an interrupted transfer or sync operation.",
    },
    {
      id: "choose",
      label: "Choose Action",
      helpText: "Pick how to handle the interrupted operation.",
    },
    {
      id: "confirm",
      label: "Confirm",
    },
  ];

  return (
    <WizardShell
      title="Interrupted Operation"
      subtitle="A file operation was interrupted and needs your attention"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: resolution === "cancel" ? "error" : "transfer",
          summary: `Resolved interrupted operation: chose to ${resolution}.`,
          paths: [],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Apply"
    >
      {step === 0 && (
        <div>
          <div className="flex items-start gap-3 p-4 rounded-lg bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/20 mb-4">
            <AlertTriangle className="h-5 w-5 text-[color:var(--color-warning)] shrink-0 mt-0.5" aria-hidden="true" />
            <div>
              <p className="text-sm font-medium text-[color:var(--color-text)]">
                A transfer was interrupted
              </p>
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
                <strong>What happened:</strong> The operation stopped before all
                files were transferred. This can happen if the computer went to
                sleep, the network disconnected, or a drive was removed.
              </p>
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
                <strong>What we did:</strong> The app saved your progress. Files
                that were already transferred are safe. No partial files were left
                behind.
              </p>
              <p className="text-xs text-[color:var(--color-primary)] mt-1">
                <strong>What you can do:</strong> Choose how to proceed on the
                next step.
              </p>
            </div>
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            How would you like to proceed?
          </h3>
          <div className="space-y-2">
            {[
              { id: "resume" as const, label: "Resume where it stopped", desc: "Continue transferring the remaining files. Already-transferred files are skipped.", icon: <RotateCcw className="h-5 w-5" /> },
              { id: "restart" as const, label: "Start over", desc: "Re-transfer everything from the beginning. Existing files will be overwritten.", icon: <RefreshCw className="h-5 w-5" /> },
              { id: "skip" as const, label: "Skip failed files", desc: "Mark the remaining files as skipped and consider the operation complete.", icon: <Check className="h-5 w-5" /> },
              { id: "cancel" as const, label: "Cancel entirely", desc: "Abandon the operation. Already-transferred files will remain at the destination.", icon: <AlertTriangle className="h-5 w-5" /> },
            ].map((r) => (
              <button
                key={r.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  resolution === r.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setResolution(r.id)}
              >
                <div className={cn("shrink-0 mt-0.5", resolution === r.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")}>
                  {r.icon}
                </div>
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{r.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{r.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Confirm your choice
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {resolution === "resume" && "The operation will resume from where it stopped. Files already transferred will be skipped."}
            {resolution === "restart" && "The operation will start over from the beginning. This may take longer."}
            {resolution === "skip" && "Remaining files will be skipped. The operation will be marked as complete with some files missing."}
            {resolution === "cancel" && "The operation will be cancelled. Files already at the destination will remain there."}
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 10: Handle Compatibility Warning
// ──────────────────────────────────────────────

export function HandleCompatWarningWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [action, setAction] = useState<"rename" | "skip" | "proceed">("rename");

  const sampleIssues = [
    { name: "project:final.doc", issue: 'Contains ":" which is not allowed on Windows' },
    { name: "CON.txt", issue: '"CON" is a reserved name on Windows' },
    { name: "file with trailing space .pdf", issue: "Has a trailing space before the extension" },
  ];

  const steps: WizardStepDef[] = [
    {
      id: "explain",
      label: "Issues Found",
      helpText: "Some filenames in this transfer contain characters or patterns that might cause problems on the destination system.",
    },
    {
      id: "choose",
      label: "Choose Fix",
      helpText: "Decide how to handle the problematic filenames.",
    },
    {
      id: "confirm",
      label: "Apply",
    },
  ];

  return (
    <WizardShell
      title="File Name Compatibility"
      subtitle="Some file names may not work on the destination system"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "rename",
          summary: `Resolved ${sampleIssues.length} compatibility issues: chose to ${action} affected files.`,
          paths: sampleIssues.map((i) => i.name),
          undoable: action === "rename",
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Apply Fix"
    >
      {step === 0 && (
        <div>
          <div className="p-4 rounded-lg bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/20 mb-4">
            <div className="flex items-start gap-3">
              <Shield className="h-5 w-5 text-[color:var(--color-warning)] shrink-0 mt-0.5" aria-hidden="true" />
              <div>
                <p className="text-sm font-medium text-[color:var(--color-text)]">
                  {sampleIssues.length} file names may cause problems
                </p>
                <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
                  The destination uses a different file system that has stricter
                  naming rules. The following files need attention.
                </p>
              </div>
            </div>
          </div>

          <div className="space-y-2">
            {sampleIssues.map((issue, i) => (
              <div key={i} className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
                <p className="text-sm font-mono text-[color:var(--color-text)]">{issue.name}</p>
                <p className="text-xs text-[color:var(--color-text-secondary)] mt-0.5">{issue.issue}</p>
              </div>
            ))}
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            How should we handle these files?
          </h3>
          <div className="space-y-2">
            {[
              { id: "rename" as const, label: "Automatically fix names", desc: "The app will rename files to be compatible. Original names are saved so you can restore them later." },
              { id: "skip" as const, label: "Skip these files", desc: "Transfer everything else and leave these files behind." },
              { id: "proceed" as const, label: "Transfer anyway", desc: "Copy the files as-is. They may not open correctly on the destination system." },
            ].map((a) => (
              <button
                key={a.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  action === a.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setAction(a.id)}
              >
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{a.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{a.desc}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Ready to apply
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {action === "rename" && "Files will be renamed to be compatible. You can restore original names anytime from the Activity feed."}
            {action === "skip" && `${sampleIssues.length} files will be skipped. Everything else will be transferred normally.`}
            {action === "proceed" && "Files will be transferred with their original names. They may not work correctly on the destination."}
          </p>
        </div>
      )}
    </WizardShell>
  );
}
