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
 *
 * Each does the real work through `@/lib/guided-actions` (the same backend
 * calls as the full screens) and reports what happened in the Activity feed.
 */
import { useEffect, useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { pickFolder } from "@/lib/folder-picker";
import { errorText } from "@/lib/error-text";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import {
  copyWithSpaceCheck,
  createBackup,
  listDrives,
  listPeers,
  listRenames,
  restoreNames,
  saveReport,
  sendToPeer,
  userFolders,
  verifyCopy,
  type BackupScheduleChoice,
  type CompatRenameRecord,
  type DriveLite,
  type PeerLite,
  type ReportFormat,
  type ReportKind,
} from "@/lib/guided-actions";
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

const heading = "text-base font-semibold text-[color:var(--color-text)] mb-2";
const lead = "text-sm text-[color:var(--color-text-secondary)] mb-4";
const fieldClass =
  "flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]";

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

/** A path field with a Browse button. */
function FolderField({ value, onChange, title, placeholder, testId }: { value: string; onChange: (v: string) => void; title: string; placeholder: string; testId?: string }) {
  return (
    <div className="flex gap-2 mt-1">
      <input type="text" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} className={fieldClass} data-testid={testId} />
      <button
        type="button"
        className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
        onClick={async () => {
          const p = await pickFolder(title).catch(() => null);
          if (p) onChange(p);
        }}
        title="Browse for folder"
      >
        <FolderOpen className="h-4 w-4" aria-hidden="true" />
      </button>
    </div>
  );
}

/** A selectable card. */
function Choice({ selected, onClick, icon, label, desc, testId, disabled }: { selected: boolean; onClick: () => void; icon?: React.ReactNode; label: string; desc?: string; testId?: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      className={cn(
        "flex items-start gap-3 w-full p-3 rounded-lg border transition-all text-left disabled:opacity-50",
        selected ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5" : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
      )}
      onClick={onClick}
      aria-pressed={selected}
      data-testid={testId}
      disabled={disabled}
    >
      {icon && <span className={cn("shrink-0 mt-0.5", selected ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")}>{icon}</span>}
      <span className="flex-1 min-w-0">
        <span className="text-sm font-medium text-[color:var(--color-text)] block truncate">{label}</span>
        {desc && <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{desc}</span>}
      </span>
      {selected && <Check className="h-4 w-4 text-[color:var(--color-primary)] shrink-0" aria-hidden="true" />}
    </button>
  );
}

function Loading({ text }: { text: string }) {
  return <p className="text-sm text-[color:var(--color-text-secondary)]">{text}</p>;
}

function Summary({ rows }: { rows: [string, string][] }) {
  return (
    <div className="space-y-3">
      {rows.map(([k, v]) => (
        <div key={k} className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
          <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">{k}</span>
          <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono break-all">{v}</p>
        </div>
      ))}
    </div>
  );
}

// ──────────────────────────────────────────────
// Wizard 11: Restore Names
// ──────────────────────────────────────────────

export function RestoreNamesWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [records, setRecords] = useState<CompatRenameRecord[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);

  useEffect(() => {
    void listRenames()
      .then(setRecords)
      .catch(() => setRecords([]));
  }, []);

  const chosen = (records ?? []).filter((r) => selected.includes(r.id));
  const steps: WizardStepDef[] = [
    {
      id: "select",
      label: "Select Files",
      helpText: "Files renamed by Fix File Name Issues. Choose which ones get their original names back.",
      validate: () => (chosen.length > 0 ? null : "Choose at least one file."),
    },
    { id: "preview", label: "Preview", helpText: "Review the name changes before applying." },
    { id: "apply", label: "Restore" },
  ];

  return (
    <WizardShell
      title="Restore Original Names"
      subtitle="Undo renames made for compatibility"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const report = await restoreNames(chosen.map((r) => r.id));
        if (report.renamed.length === 0 && report.failed.length > 0) throw new Error(report.failed.map((f) => `${baseName(f.path)}: ${f.reason}`).join("; "));
        addActivity({
          type: "rename",
          summary: `Restored original names for ${report.renamed.length} file(s)${report.failed.length ? `; ${report.failed.length} could not be restored (${report.failed.map((f) => f.reason).join("; ")})` : ""}.`,
          paths: report.renamed.map(([, to]) => to),
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Restore Names"
    >
      {step === 0 && (
        <div>
          <h3 className={heading}>Files with changed names</h3>
          <p className={lead}>These were renamed so another system could store them. Choose the ones to give their original names back.</p>
          {records === null && <Loading text="Looking for renamed files…" />}
          {records?.length === 0 && (
            <p className="text-sm text-[color:var(--color-text-secondary)]" data-testid="wizard-no-renames">
              No renamed files are recorded. Files renamed with Fix File Name Issues appear here.
            </p>
          )}
          <div className="space-y-2">
            {(records ?? []).map((r) => (
              <Choice
                key={r.id}
                selected={selected.includes(r.id)}
                disabled={!r.restorable}
                onClick={() => setSelected((prev) => (prev.includes(r.id) ? prev.filter((p) => p !== r.id) : [...prev, r.id]))}
                label={r.translated_name}
                desc={`Original: ${r.original_name} — in ${r.folder}${r.exists ? (r.restorable ? "" : " (something else has the original name now)") : " (no longer there)"}`}
                testId="wizard-rename-record"
              />
            ))}
          </div>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className={heading}>Preview name changes</h3>
          <div className="space-y-2">
            {chosen.map((r) => (
              <div key={r.id} className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
                <div className="flex items-center gap-2 text-sm">
                  <span className="font-mono text-[color:var(--color-text-secondary)] line-through">{r.translated_name}</span>
                  <ArrowRight className="h-3 w-3 text-[color:var(--color-text-tertiary)]" aria-hidden="true" />
                  <span className="font-mono text-[color:var(--color-text)]">{r.original_name}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <RotateCcw className="h-12 w-12 text-[color:var(--color-primary)] mb-4" aria-hidden="true" />
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">Ready to restore</h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
            {chosen.length} file(s) will get their original names back. A name this computer does not allow is reported and left as it is.
          </p>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 12: Export Report
// ──────────────────────────────────────────────

const REPORTS: { id: ReportKind; label: string; desc: string }[] = [
  { id: "transfer", label: "Transfer history", desc: "Every transfer with its result, size, time and checksum." },
  { id: "activity", label: "Activity log", desc: "File operations, syncs, automations and transfers, newest first." },
  { id: "audit", label: "Security audit log", desc: "Sign-ins, locks, setting changes; signed, as JSON lines." },
];

export function ExportReportWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [reportType, setReportType] = useState<ReportKind>("transfer");
  const [format, setFormat] = useState<ReportFormat>("csv");

  const steps: WizardStepDef[] = [
    { id: "type", label: "Report", helpText: "Choose what the report lists." },
    { id: "format", label: "Format", helpText: "CSV opens in a spreadsheet; JSON is for other programs." },
    { id: "export", label: "Save", helpText: "You choose where the file is saved." },
  ];

  return (
    <WizardShell
      title="Export a Report"
      subtitle="Save what UFOP did as a file"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const path = await saveReport(reportType, format);
        if (!path) return; // cancelled: stay on this step
        addActivity({ type: "transfer", summary: `Saved the ${REPORTS.find((r) => r.id === reportType)!.label.toLowerCase()} to "${path}".`, paths: [path], undoable: false });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Save Report…"
    >
      {step === 0 && (
        <div className="space-y-2">
          <h3 className={heading}>Which report?</h3>
          {REPORTS.map((r) => (
            <Choice key={r.id} selected={reportType === r.id} onClick={() => setReportType(r.id)} icon={<FileText className="h-5 w-5" />} label={r.label} desc={r.desc} testId={`wizard-report-${r.id}`} />
          ))}
        </div>
      )}
      {step === 1 && (
        <div className="space-y-2">
          <h3 className={heading}>File format</h3>
          {reportType === "audit" ? (
            <p className={lead}>The security audit log is saved as JSON lines (one signed entry per line), so it can be checked later.</p>
          ) : (
            (["csv", "json"] as const).map((f) => (
              <Choice key={f} selected={format === f} onClick={() => setFormat(f)} icon={<Download className="h-5 w-5" />} label={f.toUpperCase()} desc={f === "csv" ? "For Excel, Numbers or Google Sheets" : "For scripts and other programs"} />
            ))
          )}
        </div>
      )}
      {step === 2 && (
        <Summary
          rows={[
            ["Report", REPORTS.find((r) => r.id === reportType)!.label],
            ["Format", reportType === "audit" ? "JSON lines" : format.toUpperCase()],
          ]}
        />
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Other computers: shared by Transfer to Computer and Migrate Computers
// ──────────────────────────────────────────────

function usePeers() {
  const [peers, setPeers] = useState<PeerLite[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const load = () =>
      listPeers()
        .then((p) => live && setPeers(p))
        .catch((e) => {
          if (!live) return;
          setPeers([]);
          setError(errorText(e));
        });
    void load();
    // Discovery takes a few seconds: look again while the step is open.
    const t = setInterval(() => void load(), 3000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, []);
  return { peers, error };
}

function PeerPicker({ value, onChange }: { value: string; onChange: (id: string) => void }) {
  const { peers, error } = usePeers();
  return (
    <div className="space-y-2">
      {peers === null && <Loading text="Looking for computers on this network…" />}
      {peers?.length === 0 && (
        <p className="text-sm text-[color:var(--color-text-secondary)]" data-testid="wizard-no-peers">
          No other computer with UFOP is visible yet. Open UFOP on it, on the same network, and keep this step open.
          {error ? ` (${error})` : ""}
        </p>
      )}
      {(peers ?? []).map((p) => (
        <Choice key={p.id} selected={value === p.id} onClick={() => onChange(p.id)} icon={<Monitor className="h-5 w-5" />} label={p.display_name || p.hostname} desc={p.hostname} testId="wizard-peer" />
      ))}
    </div>
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
    { id: "computer", label: "Computer", helpText: "Computers on this network running UFOP. The other side approves the transfer.", validate: () => (targetComputer ? null : "Choose a computer.") },
    { id: "folders", label: "What to Send", helpText: "Add the folders to send.", validate: () => (selectedFolders.length > 0 ? null : "Add at least one folder.") },
    { id: "confirm", label: "Send" },
  ];

  return (
    <WizardShell
      title="Send to Another Computer"
      subtitle="Directly over your network, encrypted, every file checked"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        await sendToPeer(targetComputer, selectedFolders);
        addActivity({ type: "transfer", summary: `Offered ${selectedFolders.length} folder(s) to the other computer; it starts once that computer accepts.`, paths: selectedFolders, undoable: false });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Send"
    >
      {step === 0 && (
        <div>
          <h3 className={heading}>Which computer?</h3>
          <PeerPicker value={targetComputer} onChange={setTargetComputer} />
        </div>
      )}
      {step === 1 && (
        <div className="space-y-2">
          <h3 className={heading}>Folders to send</h3>
          {selectedFolders.map((f) => (
            <div key={f} className="flex items-center gap-2 p-2 rounded border border-[var(--color-border)] text-xs font-mono">
              <span className="flex-1 truncate">{f}</span>
              <button type="button" className="text-[color:var(--color-text-secondary)] hover:underline" onClick={() => setSelectedFolders((p) => p.filter((x) => x !== f))}>
                Remove
              </button>
            </div>
          ))}
          <button
            type="button"
            className="px-3 py-2 rounded-lg border border-[var(--color-border)] text-sm hover:bg-[var(--color-hover-bg)]"
            onClick={async () => {
              const p = await pickFolder("Folder to send").catch(() => null);
              if (p) setSelectedFolders((prev) => (prev.includes(p) ? prev : [...prev, p]));
            }}
            data-testid="wizard-add-folder"
          >
            Add Folder…
          </button>
        </div>
      )}
      {step === 2 && <Summary rows={[["Folders", selectedFolders.join("\n")]]} />}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Drives: shared by Copy to External and Old-to-New Drive
// ──────────────────────────────────────────────

function DrivePicker({ value, onChange, exclude, onlyExternal = false }: { value: string; onChange: (mountPoint: string) => void; exclude?: string; onlyExternal?: boolean }) {
  const [drives, setDrives] = useState<DriveLite[] | null>(null);
  useEffect(() => {
    void listDrives()
      .then(setDrives)
      .catch(() => setDrives([]));
  }, []);
  const shown = (drives ?? []).filter((d) => d.mount_point !== exclude && (!onlyExternal || d.drive_type === "external"));
  return (
    <div className="space-y-2">
      {drives === null && <Loading text="Looking for drives…" />}
      {drives !== null && shown.length === 0 && (
        <p className="text-sm text-[color:var(--color-text-secondary)]" data-testid="wizard-no-drives">
          {onlyExternal ? "No external drive is connected. Plug one in, then go back and forth to look again." : "No other drive was found."}
        </p>
      )}
      {shown.map((d) => (
        <Choice
          key={d.mount_point}
          selected={value === d.mount_point}
          onClick={() => onChange(d.mount_point)}
          icon={<HardDrive className="h-5 w-5" />}
          label={d.name || d.mount_point}
          desc={`${d.mount_point} — ${formatBytes(d.free_bytes)} free of ${formatBytes(d.total_bytes)}`}
          testId="wizard-drive"
        />
      ))}
    </div>
  );
}

const joinPath = (dir: string, name: string) => (dir.endsWith("/") || dir.endsWith("\\") ? `${dir}${name}` : `${dir}${dir.includes("\\") && !dir.includes("/") ? "\\" : "/"}${name}`);

// ──────────────────────────────────────────────
// Wizard 14: Copy to External
// ──────────────────────────────────────────────

export function CopyToExternalWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [externalDrive, setExternalDrive] = useState("");

  const steps: WizardStepDef[] = [
    { id: "source", label: "What to Copy", helpText: "The folder to copy.", validate: () => (sourcePath ? null : "Choose a folder.") },
    { id: "drive", label: "Drive", helpText: "External drives connected to this computer.", validate: () => (externalDrive ? null : "Choose a drive.") },
    { id: "confirm", label: "Copy", helpText: "We check there is room first. Existing files with the same name are kept as copies." },
  ];

  return (
    <WizardShell
      title="Copy to an External Drive"
      subtitle="Copy a folder onto a USB or external drive"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const outcome = await copyWithSpaceCheck([sourcePath], externalDrive);
        addActivity({ type: "copy", summary: `Copied "${sourcePath}" to ${externalDrive}: ${outcome}`, paths: [sourcePath, joinPath(externalDrive, baseName(sourcePath))], undoable: false });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Copy"
    >
      {step === 0 && (
        <div>
          <h3 className={heading}>What would you like to copy?</h3>
          <FolderField value={sourcePath} onChange={setSourcePath} title="Folder to copy" placeholder="/Users/you/Documents" testId="wizard-source" />
        </div>
      )}
      {step === 1 && (
        <div>
          <h3 className={heading}>Which drive?</h3>
          <DrivePicker value={externalDrive} onChange={setExternalDrive} onlyExternal />
        </div>
      )}
      {step === 2 && (
        <Summary
          rows={[
            ["From", sourcePath],
            ["To", joinPath(externalDrive, baseName(sourcePath))],
          ]}
        />
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 15: Old-to-New Drive
// ──────────────────────────────────────────────

export function OldToNewDriveWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [oldDrive, setOldDrive] = useState("");
  const [newDrive, setNewDrive] = useState("");
  const [verifyEnabled, setVerifyEnabled] = useState(true);
  const folderName = `From ${baseName(oldDrive) || "old drive"}`;

  const steps: WizardStepDef[] = [
    { id: "old", label: "Old Drive", helpText: "The drive to copy everything from.", validate: () => (oldDrive ? null : "Choose the old drive.") },
    { id: "new", label: "New Drive", helpText: "The drive to copy to. Nothing on it is deleted.", validate: () => (newDrive ? null : "Choose the new drive.") },
    { id: "options", label: "Options" },
    { id: "confirm", label: "Copy" },
  ];

  return (
    <WizardShell
      title="Move to a New Drive"
      subtitle="Copy everything from an old drive to a new one"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        // Into a folder of its own on the new drive, so nothing there is mixed up or replaced.
        const target = joinPath(newDrive, folderName);
        await tauriInvoke("ensure_directory", { path: target });
        const outcome = await copyWithSpaceCheck([oldDrive], target);
        let verified = "";
        if (verifyEnabled) {
          const problem = await verifyCopy(oldDrive, joinPath(target, baseName(oldDrive)), true);
          if (problem) throw new Error(`Copied, but the check found a problem: ${problem}`);
          verified = " Every file was checked against the original.";
        }
        addActivity({ type: "copy", summary: `Copied ${oldDrive} to "${target}": ${outcome}${verified}`, paths: [oldDrive, target], undoable: false });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Start Copy"
    >
      {step === 0 && (
        <div>
          <h3 className={heading}>Old drive</h3>
          <DrivePicker value={oldDrive} onChange={setOldDrive} />
        </div>
      )}
      {step === 1 && (
        <div>
          <h3 className={heading}>New drive</h3>
          <DrivePicker value={newDrive} onChange={setNewDrive} exclude={oldDrive} />
        </div>
      )}
      {step === 2 && (
        <label className="flex items-center gap-3 p-3 rounded-lg bg-[var(--color-bg-secondary)] cursor-pointer">
          <input type="checkbox" checked={verifyEnabled} onChange={(e) => setVerifyEnabled(e.target.checked)} className="h-4 w-4" />
          <span>
            <span className="text-sm text-[color:var(--color-text)] flex items-center gap-1">
              <Shield className="h-4 w-4" aria-hidden="true" /> Check every file after copying
            </span>
            <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">Compares each copy with its original by checksum. Takes longer.</span>
          </span>
        </label>
      )}
      {step === 3 && (
        <Summary
          rows={[
            ["From", oldDrive],
            ["To", joinPath(newDrive, folderName)],
            ["Check", verifyEnabled ? "Every file, by checksum" : "No"],
          ]}
        />
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Bonus: Backup Folder
// ──────────────────────────────────────────────

export function BackupFolderWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [backupDest, setBackupDest] = useState("");
  const [schedule, setSchedule] = useState<BackupScheduleChoice>("once");
  const [encrypt, setEncrypt] = useState(true);
  const [passphrase, setPassphrase] = useState("");
  const [repeat, setRepeat] = useState("");

  const steps: WizardStepDef[] = [
    { id: "source", label: "Folder", helpText: "The folder to back up.", validate: () => (sourcePath ? null : "Choose a folder.") },
    { id: "dest", label: "Where", helpText: "A folder on another drive is best: a backup on the same drive is lost with it.", validate: () => (!backupDest ? "Choose where to keep the backup." : backupDest === sourcePath ? "Keep the backup outside the folder." : null) },
    {
      id: "protect",
      label: "Protect",
      helpText: "Encrypted backups can only be read with the passphrase. Keep it somewhere safe: it cannot be recovered.",
      validate: () => (!encrypt ? null : passphrase.length < 8 ? "Use a passphrase of at least 8 characters." : passphrase !== repeat ? "The two passphrases are different." : null),
    },
    { id: "schedule", label: "When" },
  ];

  return (
    <WizardShell
      title="Back Up a Folder"
      subtitle="Versioned, deduplicated backups you can restore from"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const job = await createBackup({ source: sourcePath, dest: backupDest, schedule, passphrase: encrypt ? passphrase : null });
        addActivity({
          type: "transfer",
          summary: `Created the backup "${job.name}" to "${backupDest}"${schedule === "once" ? "" : `, running ${schedule}`}; the first backup is running (see Backups).`,
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
          <h3 className={heading}>Which folder?</h3>
          <FolderField value={sourcePath} onChange={setSourcePath} title="Folder to back up" placeholder="/Users/you/Documents" testId="wizard-backup-source" />
        </div>
      )}
      {step === 1 && (
        <div>
          <h3 className={heading}>Where should the backup be kept?</h3>
          <FolderField value={backupDest} onChange={setBackupDest} title="Backup location" placeholder="/Volumes/Backup/UFOP" testId="wizard-backup-dest" />
        </div>
      )}
      {step === 2 && (
        <div className="space-y-2">
          <Choice selected={encrypt} onClick={() => setEncrypt(true)} icon={<Shield className="h-5 w-5" />} label="Encrypt the backup (recommended)" desc="AES-256; needs the passphrase to restore." />
          <Choice selected={!encrypt} onClick={() => setEncrypt(false)} icon={<Copy className="h-5 w-5" />} label="Do not encrypt" desc="Anyone with the backup drive can read it." />
          {encrypt && (
            <div className="space-y-2 pt-2">
              <input type="password" autoComplete="new-password" placeholder="Passphrase" aria-label="Passphrase" value={passphrase} onChange={(e) => setPassphrase(e.target.value)} className={cn(fieldClass, "w-full")} data-testid="wizard-backup-passphrase" />
              <input type="password" autoComplete="new-password" placeholder="Repeat it" aria-label="Repeat the passphrase" value={repeat} onChange={(e) => setRepeat(e.target.value)} className={cn(fieldClass, "w-full")} data-testid="wizard-backup-repeat" />
            </div>
          )}
        </div>
      )}
      {step === 3 && (
        <div className="space-y-2">
          {(
            [
              ["once", "Now only", "Run it again from Backups whenever you like."],
              ["daily", "Every day", "Starts now, then once a day."],
              ["weekly", "Every week", "Starts now, then once a week."],
            ] as const
          ).map(([id, label, desc]) => (
            <Choice key={id} selected={schedule === id} onClick={() => setSchedule(id)} label={label} desc={desc} testId={`wizard-backup-${id}`} />
          ))}
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Bonus: Migrate Computers
// ──────────────────────────────────────────────

const CATEGORIES = ["Documents", "Desktop", "Pictures", "Music", "Videos", "Downloads"] as const;

export function MigrateComputersWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [targetComputer, setTargetComputer] = useState("");
  const [selectedCategories, setSelectedCategories] = useState<string[]>(["Documents", "Desktop", "Pictures"]);

  const steps: WizardStepDef[] = [
    { id: "computer", label: "New Computer", helpText: "Open UFOP on the new computer, on the same network. It approves the transfer.", validate: () => (targetComputer ? null : "Choose the new computer.") },
    { id: "what", label: "What to Move", helpText: "Your folders on this computer. They are copied; nothing here is deleted.", validate: () => (selectedCategories.length > 0 ? null : "Choose at least one folder.") },
    { id: "confirm", label: "Send" },
  ];

  return (
    <WizardShell
      title="Move to a New Computer"
      subtitle="Send your folders from this computer to another one"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={async () => {
        const paths = await userFolders(selectedCategories);
        await sendToPeer(targetComputer, paths);
        addActivity({ type: "transfer", summary: `Offered ${selectedCategories.join(", ")} to the new computer; it starts once that computer accepts.`, paths, undoable: false });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Send"
    >
      {step === 0 && (
        <div>
          <h3 className={heading}>Which is the new computer?</h3>
          <PeerPicker value={targetComputer} onChange={setTargetComputer} />
        </div>
      )}
      {step === 1 && (
        <div className="space-y-2">
          <h3 className={heading}>What should move?</h3>
          {CATEGORIES.map((c) => (
            <Choice
              key={c}
              selected={selectedCategories.includes(c)}
              onClick={() => setSelectedCategories((prev) => (prev.includes(c) ? prev.filter((x) => x !== c) : [...prev, c]))}
              icon={<FolderOpen className="h-5 w-5" />}
              label={c}
              testId={`wizard-category-${c}`}
            />
          ))}
        </div>
      )}
      {step === 2 && <Summary rows={[["Folders", selectedCategories.join(", ")]]} />}
    </WizardShell>
  );
}
