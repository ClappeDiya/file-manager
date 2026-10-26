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
import { useEffect, useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { pickFolder } from "@/lib/folder-picker";
import { errorText } from "@/lib/error-text";
import { createSyncPair, fixNames, listInterrupted, resolveInterrupted, scanNames, startTransfer, compatProfiles, type CompatIssue, type CompatScan, type Resolution, type TransferJobLite } from "@/lib/guided-actions";
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
      onComplete={async () => {
        const outcome = await startTransfer(sourcePath, destPath, transferMode, verifyAfter);
        addActivity({
          type: "transfer",
          summary: `${transferMode === "copy" ? "Copy" : "Move"} of "${sourcePath}" to "${destPath}": ${outcome}`,
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

          {transferMode === "copy" && (
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
                Compares every copied file with its original by checksum. Takes longer but proves the copy is exact.
              </p>
            </div>
          </label>
          )}
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
              {verifyAfter && transferMode === "copy" && <p className="text-xs text-[color:var(--color-primary)] mt-0.5">With verification</p>}
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
      onComplete={async () => {
        await createSyncPair({ name, source: sourcePath, dest: destPath, mode: "one-way", schedule });
        addActivity({
          type: "sync",
          summary: `Created sync pair "${name}" between "${sourcePath}" and "${destPath}" (${schedule}). It is in the Sync panel.`,
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
  const [resolution, setResolution] = useState<Resolution>("resume");
  const [jobs, setJobs] = useState<TransferJobLite[] | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());

  useEffect(() => {
    let live = true;
    void listInterrupted()
      .then((list) => {
        if (!live) return;
        setJobs(list);
        setChosen(new Set(list.map((j) => j.id)));
      })
      .catch(() => live && setJobs([]));
    return () => {
      live = false;
    };
  }, []);

  const picked = (jobs ?? []).filter((j) => chosen.has(j.id));
  const steps: WizardStepDef[] = [
    {
      id: "explain",
      label: "What Happened",
      helpText: "Transfers that were paused or stopped with an error are listed here. Choose the ones to deal with.",
      validate: () => (picked.length > 0 ? null : jobs && jobs.length === 0 ? "Nothing is waiting: every transfer finished." : "Choose at least one transfer."),
    },
    {
      id: "choose",
      label: "Choose Action",
      helpText: "Pick how to handle the interrupted transfers.",
    },
    {
      id: "confirm",
      label: "Confirm",
    },
  ];

  return (
    <WizardShell
      title="Interrupted Operation"
      subtitle="Transfers that stopped before the end"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const failed: string[] = [];
        for (const job of picked) {
          try {
            await resolveInterrupted(job, resolution);
          } catch (e) {
            failed.push(`${baseName(job.source_path)}: ${errorText(e)}`);
          }
        }
        if (failed.length === picked.length) throw new Error(failed.join("; "));
        addActivity({
          type: resolution === "cancel" ? "error" : "transfer",
          summary: `${resolution === "cancel" ? "Cancelled" : "Resumed"} ${picked.length - failed.length} interrupted transfer(s)${failed.length ? `; ${failed.length} could not be: ${failed.join("; ")}` : ""}.`,
          paths: picked.map((j) => j.source_path),
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
            <p className="text-xs text-[color:var(--color-text-secondary)]">
              <strong>What happened:</strong> these transfers stopped before all of the data arrived: they were paused, or the computer slept, the
              network dropped or a drive went away. <strong>What we did:</strong> the progress was saved, so a resumed transfer continues from where it
              stopped and is checked when it completes.
            </p>
          </div>
          {jobs === null && <p className="text-sm text-[color:var(--color-text-secondary)]">Looking for interrupted transfers…</p>}
          {jobs?.length === 0 && <p className="text-sm text-[color:var(--color-text-secondary)]" data-testid="wizard-no-interrupted">Nothing is waiting: every transfer finished.</p>}
          <div className="space-y-1">
            {(jobs ?? []).map((j) => (
              <label key={j.id} className="flex items-start gap-2 p-2 rounded border border-[var(--color-border)] text-xs" data-testid="wizard-interrupted-job">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  checked={chosen.has(j.id)}
                  onChange={(e) =>
                    setChosen((prev) => {
                      const next = new Set(prev);
                      if (e.target.checked) next.add(j.id);
                      else next.delete(j.id);
                      return next;
                    })
                  }
                />
                <span className="min-w-0">
                  <span className="block font-mono truncate text-[color:var(--color-text)]">{j.source_path}</span>
                  <span className="block text-[color:var(--color-text-secondary)]">
                    {j.status === "paused" ? "Paused" : "Failed"}
                    {j.total_bytes > 0 ? ` at ${Math.floor((j.transferred_bytes / j.total_bytes) * 100)}%` : ""}
                    {j.error_message ? ` — ${j.error_message}` : ""}
                  </span>
                </span>
              </label>
            ))}
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
              { id: "resume" as const, label: "Resume where it stopped", desc: "Paused transfers continue; failed ones are tried again, continuing from the saved progress.", icon: <RotateCcw className="h-5 w-5" /> },
              { id: "cancel" as const, label: "Cancel", desc: "Stop for good. Files that already arrived stay at the destination; a partly written file is removed.", icon: <AlertTriangle className="h-5 w-5" /> },
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
                aria-pressed={resolution === r.id}
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
            {resolution === "resume"
              ? `${picked.length} transfer(s) will continue from where they stopped.`
              : `${picked.length} transfer(s) will be cancelled. Files already at the destination stay there.`}
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
  const [folder, setFolder] = useState("");
  const [profiles, setProfiles] = useState<{ id: string; name: string; description: string }[]>([]);
  const [profileId, setProfileId] = useState("windows-ntfs");
  const [scan, setScan] = useState<CompatScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [action, setAction] = useState<"rename" | "skip">("rename");

  useEffect(() => {
    void compatProfiles().then(setProfiles).catch(() => setProfiles([]));
  }, []);

  const runScan = async () => {
    setScanning(true);
    setScanError(null);
    try {
      setScan(await scanNames(folder, profileId));
    } catch (e) {
      setScan(null);
      setScanError(errorText(e));
    } finally {
      setScanning(false);
    }
  };

  const fixable: CompatIssue[] = (scan?.issues ?? []).filter((i) => i.fixable);
  const steps: WizardStepDef[] = [
    {
      id: "where",
      label: "Folder",
      helpText: "Choose the folder you are about to copy, and where it is going. Names that place cannot store are found before the copy fails on them.",
      validate: () => (folder ? null : "Choose a folder."),
    },
    {
      id: "scan",
      label: "What We Found",
      helpText: "Names with characters, lengths or reserved words the destination does not allow.",
      validate: () => (scan ? null : scanning ? "Still checking…" : "Check the folder first."),
    },
    {
      id: "action",
      label: "Choose Fix",
      helpText: "Rename to the suggested names now (you can restore the original names later), or leave the files as they are.",
    },
  ];

  return (
    <WizardShell
      title="Fix File Name Issues"
      subtitle="Find names another system cannot store, and fix them before copying"
      steps={steps}
      currentStep={step}
      onStepChange={(i) => {
        setStep(i);
        if (i === 1 && !scan && !scanning) void runScan();
      }}
      onComplete={async () => {
        if (action === "rename" && fixable.length > 0) {
          const report = await fixNames(fixable.map((i) => ({ path: i.path, new_name: i.suggested_name })), profileId);
          if (report.renamed.length === 0 && report.failed.length > 0) throw new Error(report.failed.map((f) => `${baseName(f.path)}: ${f.reason}`).join("; "));
          addActivity({
            type: "rename",
            summary: `Renamed ${report.renamed.length} item(s) in "${folder}" for ${profiles.find((p) => p.id === profileId)?.name ?? profileId}${report.failed.length ? `; ${report.failed.length} could not be renamed` : ""}. Restore Original Names undoes it.`,
            paths: report.renamed.map(([, to]) => to),
            undoable: true,
          });
        }
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel={action === "rename" && fixable.length > 0 ? "Rename Files" : "Done"}
    >
      {step === 0 && (
        <div className="space-y-4">
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Folder to check</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={folder}
                onChange={(e) => {
                  setFolder(e.target.value);
                  setScan(null);
                }}
                placeholder="/Users/you/Documents"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)]"
                data-testid="wizard-compat-folder"
              />
              <button
                type="button"
                className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                onClick={async () => {
                  const p = await pickFolder("Folder to check").catch(() => null);
                  if (p) {
                    setFolder(p);
                    setScan(null);
                  }
                }}
                title="Browse for folder"
              >
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </label>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">It is going to</span>
            <select
              value={profileId}
              onChange={(e) => {
                setProfileId(e.target.value);
                setScan(null);
              }}
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)]"
              data-testid="wizard-compat-profile"
            >
              {(profiles.length ? profiles : [{ id: "windows-ntfs", name: "Windows NTFS", description: "" }]).map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          {scanning && <p className="text-sm text-[color:var(--color-text-secondary)]">Checking the names in "{folder}"…</p>}
          {scanError && (
            <p role="alert" className="text-sm text-[color:var(--color-error)]">
              {scanError}
            </p>
          )}
          {scan && scan.issues.length === 0 && (
            <div className="flex items-center gap-2 text-sm text-[color:var(--color-text)]" data-testid="wizard-compat-clean">
              <Check className="h-4 w-4 text-[color:var(--color-success)]" aria-hidden="true" />
              All {scan.scanned} names can be stored there.
            </div>
          )}
          {scan && scan.issues.length > 0 && (
            <div className="space-y-2">
              <p className="text-sm text-[color:var(--color-text)]">
                {scan.issues.length} name(s) need attention{scan.truncated ? " (the list was cut short; check again after fixing these)" : ""}; {fixable.length} can be renamed automatically.
              </p>
              <ul className="max-h-56 overflow-y-auto space-y-1">
                {scan.issues.slice(0, 200).map((i) => (
                  <li key={i.path} className="p-2 rounded border border-[var(--color-border)] text-xs" data-testid="wizard-compat-issue">
                    <span className="font-mono text-[color:var(--color-text)]">{i.name}</span>
                    {i.fixable ? (
                      <span className="text-[color:var(--color-text-secondary)]"> → <span className="font-mono">{i.suggested_name}</span></span>
                    ) : (
                      <span className="text-[color:var(--color-warning)]"> — needs a manual rename</span>
                    )}
                    {i.rules.length > 0 && <span className="block text-[color:var(--color-text-tertiary)]">{i.rules.join("; ")}</span>}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}

      {step === 2 && (
        <div className="space-y-2">
          {[
            { id: "rename" as const, label: `Rename ${fixable.length} item(s) to the suggested names`, desc: "Recorded, so Restore Original Names can put the old names back." },
            { id: "skip" as const, label: "Leave them as they are", desc: "Nothing is changed. Copying may fail for these files." },
          ].map((a) => (
            <button
              key={a.id}
              className={cn(
                "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                action === a.id ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
              )}
              onClick={() => setAction(a.id)}
              aria-pressed={action === a.id}
              disabled={a.id === "rename" && fixable.length === 0}
            >
              <Shield className="h-5 w-5 shrink-0 mt-0.5 text-[color:var(--color-text-secondary)]" aria-hidden="true" />
              <div>
                <span className="text-sm font-medium text-[color:var(--color-text)] block">{a.label}</span>
                <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{a.desc}</span>
              </div>
            </button>
          ))}
        </div>
      )}
    </WizardShell>
  );
}

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
