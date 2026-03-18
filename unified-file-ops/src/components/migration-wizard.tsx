/**
 * Migration Wizard Component (T-032)
 *
 * Three guided wizard workflows:
 * 1. Computer Migration - Transfer files between computers (connects to T-031 peer system)
 * 2. Drive Migration - Migrate files between drives with filesystem compat checks
 * 3. External Backup - Back up to external drive with copy/mirror/sync/versioned modes
 *
 * All wizards are resumable, retry failed items, and provide post-run summaries.
 */

import { useState, useCallback, useMemo } from "react";
import { cn } from "@ufop/ui-components";
import {
  HardDrive,
  Monitor,
  Shield,
  ArrowRight,
  ArrowLeft,
  Check,
  X,
  AlertTriangle,
  Loader2,
  FolderOpen,
  RefreshCw,
  Database,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/** Which wizard is active. */
export type WizardType = "computer" | "drive" | "backup" | null;

/** Wizard step for all three flows. */
export type WizardStep =
  | "select-type"     // Choose which wizard
  | "connect-source"  // Connect to source (computer/drive)
  | "select-folders"  // Pick folders/files
  | "space-analysis"  // Analyze space + compatibility
  | "preview"         // Preview what will be transferred
  | "executing"       // Transfer in progress
  | "summary";        // Post-run summary

/** Backup mode for external backup wizard. */
export type BackupMode = "copy" | "mirror" | "sync" | "versioned";

/** Source device for migration. */
export interface MigrationSource {
  id: string;
  name: string;
  type: "computer" | "drive";
  os?: string;
  ip?: string;
  mountPoint?: string;
  totalBytes: number;
  freeBytes: number;
  filesystem?: string;
}

/** Destination device for migration. */
export interface MigrationDestination {
  id: string;
  name: string;
  mountPoint: string;
  totalBytes: number;
  freeBytes: number;
  filesystem?: string;
}

/** A folder selected for migration. */
export interface MigrationFolder {
  path: string;
  name: string;
  size: number;
  fileCount: number;
  selected: boolean;
}

/** Compatibility issue found during preflight. */
export interface CompatIssue {
  severity: "error" | "warning" | "info";
  code: string;
  message: string;
  detail?: string;
  autoFix?: boolean;
}

/** Space analysis result. */
export interface SpaceAnalysis {
  totalSelectedBytes: number;
  destFreeBytes: number;
  destTotalBytes: number;
  hasEnoughSpace: boolean;
  spaceAfterTransfer: number;
  duplicateBytes: number;
  duplicateCount: number;
  compatIssues: CompatIssue[];
}

/** Transfer summary for a completed migration. */
export interface MigrationSummary {
  totalFiles: number;
  totalBytes: number;
  filesTransferred: number;
  bytesTransferred: number;
  filesFailed: number;
  filesSkipped: number;
  durationMs: number;
  averageSpeedBps: number;
  errors: string[];
  warnings: string[];
  verificationPassed: boolean;
  resumed: boolean;
}

/** Props for the MigrationWizard component. */
interface MigrationWizardProps {
  /** Whether the wizard is currently visible. */
  open: boolean;
  /** Close the wizard. */
  onClose: () => void;
  /** Available peer computers (from T-031). */
  availableComputers?: MigrationSource[];
  /** Available drives (from T-025). */
  availableDrives?: MigrationSource[];
  /** Callback to scan folders on a source. */
  onScanFolders?: (sourceId: string) => Promise<MigrationFolder[]>;
  /** Callback to analyze space and compatibility. */
  onAnalyzeSpace?: (
    sourceId: string,
    destId: string,
    folders: string[],
  ) => Promise<SpaceAnalysis>;
  /** Callback to execute the migration. */
  onExecute?: (config: MigrationConfig) => Promise<void>;
  /** Callback to get progress updates. */
  onGetProgress?: () => Promise<MigrationProgress>;
  /** Callback to retry failed items. */
  onRetryFailed?: () => Promise<void>;
  /** Callback to get the final summary. */
  onGetSummary?: () => Promise<MigrationSummary>;
  className?: string;
}

/** Full migration configuration passed to execute. */
export interface MigrationConfig {
  type: WizardType;
  sourceId: string;
  destId: string;
  folders: string[];
  backupMode?: BackupMode;
  verifyAfterTransfer: boolean;
  skipDuplicates: boolean;
  preservePermissions: boolean;
}

/** Live progress during execution. */
export interface MigrationProgress {
  currentFile: string;
  filesCompleted: number;
  filesTotal: number;
  bytesTransferred: number;
  bytesTotal: number;
  speedBps: number;
  etaSeconds: number | null;
  failedCount: number;
  phase: "preparing" | "transferring" | "verifying" | "complete" | "failed";
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

function formatBytes(bytes: number): string {
  const TB = 1024 ** 4;
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  const KB = 1024;
  if (bytes >= TB) return `${(bytes / TB).toFixed(1)} TB`;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours > 0)
    return `${hours}h ${minutes % 60}m ${seconds % 60}s`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
  return `${seconds}s`;
}

function formatSpeed(bps: number): string {
  if (bps >= 1024 ** 3) return `${(bps / 1024 ** 3).toFixed(1)} GB/s`;
  if (bps >= 1024 ** 2) return `${(bps / 1024 ** 2).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${bps} B/s`;
}

const BACKUP_MODES: { id: BackupMode; label: string; description: string }[] = [
  {
    id: "copy",
    label: "Copy",
    description: "Copy all selected files to the destination, keeping existing files.",
  },
  {
    id: "mirror",
    label: "Mirror",
    description:
      "Make destination an exact copy of source. Files not in source will be deleted from destination.",
  },
  {
    id: "sync",
    label: "Sync",
    description:
      "Copy only new and changed files. Existing identical files are skipped.",
  },
  {
    id: "versioned",
    label: "Versioned Backup",
    description:
      "Keep multiple versions of files with timestamps. Nothing is overwritten or deleted.",
  },
];

// ──────────────────────────────────────────────
// Main Component
// ──────────────────────────────────────────────

export function MigrationWizard({
  open,
  onClose,
  availableComputers = [],
  availableDrives = [],
  onScanFolders,
  onAnalyzeSpace,
  onExecute,
  onGetProgress,
  onRetryFailed,
  onGetSummary,
  className,
}: MigrationWizardProps) {
  // Wizard state
  const [wizardType, setWizardType] = useState<WizardType>(null);
  const [step, setStep] = useState<WizardStep>("select-type");
  const [source, setSource] = useState<MigrationSource | null>(null);
  const [destination, setDestination] = useState<MigrationDestination | null>(null);
  const [folders, setFolders] = useState<MigrationFolder[]>([]);
  const [analysis, setAnalysis] = useState<SpaceAnalysis | null>(null);
  const [backupMode, setBackupMode] = useState<BackupMode>("copy");
  const [progress, setProgress] = useState<MigrationProgress | null>(null);
  const [summary, setSummary] = useState<MigrationSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Options
  const [verifyAfterTransfer, setVerifyAfterTransfer] = useState(true);
  const [skipDuplicates, setSkipDuplicates] = useState(true);
  const [preservePermissions, setPreservePermissions] = useState(true);

  // Selected folder paths
  const selectedFolders = useMemo(
    () => folders.filter((f) => f.selected).map((f) => f.path),
    [folders],
  );

  const totalSelectedSize = useMemo(
    () => folders.filter((f) => f.selected).reduce((sum, f) => sum + f.size, 0),
    [folders],
  );

  // Reset wizard
  const resetWizard = useCallback(() => {
    setWizardType(null);
    setStep("select-type");
    setSource(null);
    setDestination(null);
    setFolders([]);
    setAnalysis(null);
    setBackupMode("copy");
    setProgress(null);
    setSummary(null);
    setError(null);
  }, []);

  // Handle close
  const handleClose = useCallback(() => {
    if (step === "executing" && progress?.phase === "transferring") {
      // Warn about active transfer
      if (!confirm("A transfer is in progress. Are you sure you want to close?")) {
        return;
      }
    }
    resetWizard();
    onClose();
  }, [step, progress, resetWizard, onClose]);

  // Select wizard type
  const handleSelectType = useCallback((type: WizardType) => {
    setWizardType(type);
    setStep("connect-source");
  }, []);

  // Select source
  const handleSelectSource = useCallback(
    async (src: MigrationSource) => {
      setSource(src);
      setError(null);

      if (wizardType === "computer") {
        // For computer migration, go straight to folder selection
        setStep("select-folders");
        if (onScanFolders) {
          setLoading(true);
          try {
            const scanned = await onScanFolders(src.id);
            setFolders(scanned);
          } catch (err) {
            setError(
              err instanceof Error ? err.message : "Failed to scan folders",
            );
          } finally {
            setLoading(false);
          }
        }
      } else {
        // For drive migration and backup, need to select destination too
        setStep("select-folders");
      }
    },
    [wizardType, onScanFolders],
  );

  // Select destination
  const handleSelectDestination = useCallback(
    (dest: MigrationSource) => {
      setDestination({
        id: dest.id,
        name: dest.name,
        mountPoint: dest.mountPoint || "",
        totalBytes: dest.totalBytes,
        freeBytes: dest.freeBytes,
        filesystem: dest.filesystem,
      });
    },
    [],
  );

  // Toggle folder selection
  const toggleFolder = useCallback((path: string) => {
    setFolders((prev) =>
      prev.map((f) =>
        f.path === path ? { ...f, selected: !f.selected } : f,
      ),
    );
  }, []);

  // Select all folders
  const selectAllFolders = useCallback(() => {
    setFolders((prev) => prev.map((f) => ({ ...f, selected: true })));
  }, []);

  // Analyze space and compatibility
  const handleAnalyze = useCallback(async () => {
    if (!source || !destination || selectedFolders.length === 0) return;

    setLoading(true);
    setError(null);

    try {
      if (onAnalyzeSpace) {
        const result = await onAnalyzeSpace(
          source.id,
          destination.id,
          selectedFolders,
        );
        setAnalysis(result);
      } else {
        // Mock analysis
        setAnalysis({
          totalSelectedBytes: totalSelectedSize,
          destFreeBytes: destination.freeBytes,
          destTotalBytes: destination.totalBytes,
          hasEnoughSpace: destination.freeBytes > totalSelectedSize,
          spaceAfterTransfer: destination.freeBytes - totalSelectedSize,
          duplicateBytes: 0,
          duplicateCount: 0,
          compatIssues: [],
        });
      }
      setStep("space-analysis");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Analysis failed");
    } finally {
      setLoading(false);
    }
  }, [
    source,
    destination,
    selectedFolders,
    totalSelectedSize,
    onAnalyzeSpace,
  ]);

  // Execute migration
  const handleExecute = useCallback(async () => {
    if (!source || !destination) return;

    setStep("executing");
    setError(null);

    const config: MigrationConfig = {
      type: wizardType,
      sourceId: source.id,
      destId: destination.id,
      folders: selectedFolders,
      backupMode: wizardType === "backup" ? backupMode : undefined,
      verifyAfterTransfer,
      skipDuplicates,
      preservePermissions,
    };

    try {
      if (onExecute) {
        await onExecute(config);
      }

      // Poll for progress
      if (onGetProgress) {
        const pollProgress = async () => {
          try {
            const p = await onGetProgress();
            setProgress(p);
            if (
              p.phase !== "complete" &&
              p.phase !== "failed"
            ) {
              setTimeout(pollProgress, 500);
            } else if (p.phase === "complete") {
              if (onGetSummary) {
                const s = await onGetSummary();
                setSummary(s);
              }
              setStep("summary");
            }
          } catch {
            // Progress polling failed, keep trying
            setTimeout(pollProgress, 2000);
          }
        };
        pollProgress();
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Migration failed");
      setProgress((prev) =>
        prev ? { ...prev, phase: "failed" } : null,
      );
    }
  }, [
    source,
    destination,
    wizardType,
    selectedFolders,
    backupMode,
    verifyAfterTransfer,
    skipDuplicates,
    preservePermissions,
    onExecute,
    onGetProgress,
    onGetSummary,
  ]);

  // Navigation
  const canGoBack = step !== "select-type" && step !== "executing";
  const goBack = useCallback(() => {
    const stepOrder: WizardStep[] = [
      "select-type",
      "connect-source",
      "select-folders",
      "space-analysis",
      "preview",
      "executing",
      "summary",
    ];
    const idx = stepOrder.indexOf(step);
    if (idx > 0) {
      setStep(stepOrder[idx - 1]);
    }
  }, [step]);

  if (!open) return null;

  return (
    <div
      className={cn(
        "fixed inset-0 z-50 flex items-center justify-center",
        "bg-black/50 backdrop-blur-sm",
      )}
      data-testid="migration-wizard"
    >
      <div
        className={cn(
          "bg-[var(--color-panel-bg)] rounded-xl shadow-2xl",
          "border border-[var(--color-border)]",
          "w-[720px] max-h-[80vh] flex flex-col",
          className,
        )}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
          <div className="flex items-center gap-3">
            {wizardType === "computer" && (
              <Monitor className="h-5 w-5 text-[color:var(--color-primary)]" />
            )}
            {wizardType === "drive" && (
              <HardDrive className="h-5 w-5 text-[color:var(--color-primary)]" />
            )}
            {wizardType === "backup" && (
              <Shield className="h-5 w-5 text-[color:var(--color-primary)]" />
            )}
            <h2 className="text-[length:var(--font-size-base)] font-semibold text-[color:var(--color-text)]">
              {wizardType === "computer" && "Computer Migration"}
              {wizardType === "drive" && "Drive Migration"}
              {wizardType === "backup" && "External Backup"}
              {!wizardType && "Migration Wizard"}
            </h2>
          </div>
          <button
            onClick={handleClose}
            className="p-1 rounded-md hover:bg-[var(--color-hover-bg)] transition-theme"
            aria-label="Close wizard"
          >
            <X className="h-4 w-4 text-[color:var(--color-text-secondary)]" />
          </button>
        </div>

        {/* Step indicator */}
        {wizardType && (
          <StepIndicator currentStep={step} wizardType={wizardType} />
        )}

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {error && (
            <div className="mb-4 p-3 rounded-lg bg-[var(--color-error)]/10 text-[color:var(--color-error)] text-[length:var(--font-size-sm)] flex items-start gap-2">
              <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{error}</span>
            </div>
          )}

          {/* Step: Select wizard type */}
          {step === "select-type" && (
            <WizardTypeSelector onSelect={handleSelectType} />
          )}

          {/* Step: Connect source */}
          {step === "connect-source" && (
            <SourceSelector
              wizardType={wizardType!}
              computers={availableComputers}
              drives={availableDrives}
              selectedSource={source}
              selectedDest={destination}
              onSelectSource={handleSelectSource}
              onSelectDest={handleSelectDestination}
              backupMode={backupMode}
              onSetBackupMode={setBackupMode}
            />
          )}

          {/* Step: Select folders */}
          {step === "select-folders" && (
            <FolderSelector
              folders={folders}
              loading={loading}
              onToggle={toggleFolder}
              onSelectAll={selectAllFolders}
              totalSelected={totalSelectedSize}
            />
          )}

          {/* Step: Space analysis */}
          {step === "space-analysis" && analysis && (
            <SpaceAnalysisView analysis={analysis} />
          )}

          {/* Step: Preview */}
          {step === "preview" && (
            <PreviewStep
              source={source!}
              destination={destination!}
              selectedFolders={selectedFolders}
              totalSize={totalSelectedSize}
              wizardType={wizardType!}
              backupMode={backupMode}
              verifyAfterTransfer={verifyAfterTransfer}
              onSetVerify={setVerifyAfterTransfer}
              skipDuplicates={skipDuplicates}
              onSetSkipDuplicates={setSkipDuplicates}
              preservePermissions={preservePermissions}
              onSetPreservePermissions={setPreservePermissions}
            />
          )}

          {/* Step: Executing */}
          {step === "executing" && (
            <ExecutingStep
              progress={progress}
              onRetry={onRetryFailed}
            />
          )}

          {/* Step: Summary */}
          {step === "summary" && summary && (
            <SummaryStep summary={summary} />
          )}
        </div>

        {/* Footer with navigation */}
        <div className="flex items-center justify-between px-6 py-4 border-t border-[var(--color-border)]">
          <div>
            {canGoBack && (
              <button
                onClick={goBack}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)] transition-theme"
              >
                <ArrowLeft className="h-3.5 w-3.5" />
                Back
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            {step === "select-folders" && selectedFolders.length > 0 && destination && (
              <button
                onClick={handleAnalyze}
                disabled={loading}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-[length:var(--font-size-sm)] bg-[var(--color-primary)] text-white hover:opacity-90 transition-theme disabled:opacity-50"
              >
                {loading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <ArrowRight className="h-3.5 w-3.5" />
                )}
                Analyze
              </button>
            )}
            {step === "space-analysis" && analysis?.hasEnoughSpace && (
              <button
                onClick={() => setStep("preview")}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-[length:var(--font-size-sm)] bg-[var(--color-primary)] text-white hover:opacity-90 transition-theme"
              >
                <ArrowRight className="h-3.5 w-3.5" />
                Preview
              </button>
            )}
            {step === "preview" && (
              <button
                onClick={handleExecute}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-[length:var(--font-size-sm)] bg-[var(--color-success)] text-white hover:opacity-90 transition-theme"
              >
                <Check className="h-3.5 w-3.5" />
                Start Migration
              </button>
            )}
            {step === "summary" && (
              <button
                onClick={handleClose}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-[length:var(--font-size-sm)] bg-[var(--color-primary)] text-white hover:opacity-90 transition-theme"
              >
                Done
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step Indicator
// ──────────────────────────────────────────────

function StepIndicator({
  currentStep,
  wizardType: _wizardType,
}: {
  currentStep: WizardStep;
  wizardType: WizardType;
}) {
  const steps: { key: WizardStep; label: string }[] = [
    { key: "connect-source", label: "Source" },
    { key: "select-folders", label: "Select" },
    { key: "space-analysis", label: "Analyze" },
    { key: "preview", label: "Preview" },
    { key: "executing", label: "Execute" },
    { key: "summary", label: "Summary" },
  ];

  const currentIdx = steps.findIndex((s) => s.key === currentStep);

  return (
    <div className="px-6 py-3 border-b border-[var(--color-border)]">
      <div className="flex items-center gap-1">
        {steps.map((s, i) => (
          <div key={s.key} className="flex items-center gap-1 flex-1">
            <div
              className={cn(
                "flex items-center gap-1.5",
                i <= currentIdx
                  ? "text-[color:var(--color-primary)]"
                  : "text-[color:var(--color-text-tertiary)]",
              )}
            >
              <div
                className={cn(
                  "w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold",
                  i < currentIdx && "bg-[var(--color-primary)] text-white",
                  i === currentIdx &&
                    "border-2 border-[var(--color-primary)] text-[color:var(--color-primary)]",
                  i > currentIdx &&
                    "border border-[var(--color-text-tertiary)] text-[color:var(--color-text-tertiary)]",
                )}
              >
                {i < currentIdx ? (
                  <Check className="h-3 w-3" />
                ) : (
                  i + 1
                )}
              </div>
              <span className="text-[10px] font-medium hidden sm:inline">
                {s.label}
              </span>
            </div>
            {i < steps.length - 1 && (
              <div
                className={cn(
                  "flex-1 h-px mx-1",
                  i < currentIdx
                    ? "bg-[var(--color-primary)]"
                    : "bg-[var(--color-border)]",
                )}
              />
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Wizard Type Selector
// ──────────────────────────────────────────────

function WizardTypeSelector({
  onSelect,
}: {
  onSelect: (type: WizardType) => void;
}) {
  const types = [
    {
      id: "computer" as WizardType,
      icon: Monitor,
      title: "Computer Migration",
      description:
        "Transfer files from another computer on your network. Works across macOS, Windows, and Linux.",
    },
    {
      id: "drive" as WizardType,
      icon: HardDrive,
      title: "Drive Migration",
      description:
        "Migrate files between drives. Checks filesystem compatibility, detects duplicates, and verifies transfers.",
    },
    {
      id: "backup" as WizardType,
      icon: Shield,
      title: "External Backup",
      description:
        "Back up files to an external drive. Choose from copy, mirror, sync, or versioned backup modes.",
    },
  ];

  return (
    <div className="space-y-3" data-testid="wizard-type-selector">
      <p className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)] mb-4">
        What would you like to do?
      </p>
      {types.map((t) => (
        <button
          key={t.id}
          onClick={() => onSelect(t.id)}
          className={cn(
            "w-full flex items-start gap-4 p-4 rounded-lg text-left",
            "border border-[var(--color-border)]",
            "hover:border-[var(--color-primary)] hover:bg-[var(--color-hover-bg)]",
            "transition-theme",
          )}
          data-testid={`wizard-type-${t.id}`}
        >
          <t.icon className="h-8 w-8 text-[color:var(--color-primary)] shrink-0 mt-0.5" />
          <div>
            <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
              {t.title}
            </h3>
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mt-1">
              {t.description}
            </p>
          </div>
        </button>
      ))}
    </div>
  );
}

// ──────────────────────────────────────────────
// Source Selector
// ──────────────────────────────────────────────

interface SourceSelectorProps {
  wizardType: WizardType;
  computers: MigrationSource[];
  drives: MigrationSource[];
  selectedSource: MigrationSource | null;
  selectedDest: MigrationDestination | null;
  onSelectSource: (src: MigrationSource) => void;
  onSelectDest: (dest: MigrationSource) => void;
  backupMode: BackupMode;
  onSetBackupMode: (mode: BackupMode) => void;
}

function SourceSelector({
  wizardType,
  computers,
  drives,
  selectedSource,
  selectedDest,
  onSelectSource,
  onSelectDest,
  backupMode,
  onSetBackupMode,
}: SourceSelectorProps) {
  const sources = wizardType === "computer" ? computers : drives;
  const showDestination = wizardType === "drive" || wizardType === "backup";

  return (
    <div className="space-y-4">
      {/* Source selection */}
      <div>
        <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)] mb-2">
          {wizardType === "computer"
            ? "Select Source Computer"
            : "Select Source Drive"}
        </h3>
        {sources.length === 0 ? (
          <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic py-4 text-center">
            {wizardType === "computer"
              ? "No computers found on the network. Ensure both devices have the app running."
              : "No drives detected."}
          </p>
        ) : (
          <div className="space-y-2">
            {sources.map((src) => (
              <button
                key={src.id}
                onClick={() => onSelectSource(src)}
                className={cn(
                  "w-full flex items-center gap-3 p-3 rounded-lg text-left",
                  "border transition-theme",
                  selectedSource?.id === src.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-primary)]/50",
                )}
              >
                {src.type === "computer" ? (
                  <Monitor className="h-5 w-5 text-[color:var(--color-primary)] shrink-0" />
                ) : (
                  <HardDrive className="h-5 w-5 text-[color:var(--color-primary)] shrink-0" />
                )}
                <div className="flex-1 min-w-0">
                  <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)] block truncate">
                    {src.name}
                  </span>
                  <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
                    {src.os && `${src.os} - `}
                    {src.ip && `${src.ip} - `}
                    {formatBytes(src.freeBytes)} free of{" "}
                    {formatBytes(src.totalBytes)}
                    {src.filesystem && ` (${src.filesystem})`}
                  </span>
                </div>
                {selectedSource?.id === src.id && (
                  <Check className="h-4 w-4 text-[color:var(--color-primary)] shrink-0" />
                )}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Destination selection */}
      {showDestination && (
        <div>
          <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)] mb-2">
            Select Destination Drive
          </h3>
          <div className="space-y-2">
            {drives
              .filter((d) => d.id !== selectedSource?.id)
              .map((dest) => (
                <button
                  key={dest.id}
                  onClick={() => onSelectDest(dest)}
                  className={cn(
                    "w-full flex items-center gap-3 p-3 rounded-lg text-left",
                    "border transition-theme",
                    selectedDest?.id === dest.id
                      ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                      : "border-[var(--color-border)] hover:border-[var(--color-primary)]/50",
                  )}
                >
                  <HardDrive className="h-5 w-5 text-[color:var(--color-text-secondary)] shrink-0" />
                  <div className="flex-1 min-w-0">
                    <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)] block truncate">
                      {dest.name}
                    </span>
                    <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
                      {formatBytes(dest.freeBytes)} free of{" "}
                      {formatBytes(dest.totalBytes)}
                      {dest.filesystem && ` (${dest.filesystem})`}
                    </span>
                  </div>
                  {selectedDest?.id === dest.id && (
                    <Check className="h-4 w-4 text-[color:var(--color-primary)] shrink-0" />
                  )}
                </button>
              ))}
          </div>
        </div>
      )}

      {/* Backup mode selector */}
      {wizardType === "backup" && (
        <div>
          <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)] mb-2">
            Backup Mode
          </h3>
          <div className="space-y-2">
            {BACKUP_MODES.map((mode) => (
              <button
                key={mode.id}
                onClick={() => onSetBackupMode(mode.id)}
                className={cn(
                  "w-full flex items-start gap-3 p-3 rounded-lg text-left",
                  "border transition-theme",
                  backupMode === mode.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-primary)]/50",
                )}
              >
                <div
                  className={cn(
                    "w-4 h-4 rounded-full border-2 mt-0.5 shrink-0",
                    "flex items-center justify-center",
                    backupMode === mode.id
                      ? "border-[var(--color-primary)]"
                      : "border-[var(--color-text-tertiary)]",
                  )}
                >
                  {backupMode === mode.id && (
                    <div className="w-2 h-2 rounded-full bg-[var(--color-primary)]" />
                  )}
                </div>
                <div>
                  <span className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-text)]">
                    {mode.label}
                  </span>
                  <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mt-0.5">
                    {mode.description}
                  </p>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Folder Selector
// ──────────────────────────────────────────────

interface FolderSelectorProps {
  folders: MigrationFolder[];
  loading: boolean;
  onToggle: (path: string) => void;
  onSelectAll: () => void;
  totalSelected: number;
}

function FolderSelector({
  folders,
  loading,
  onToggle,
  onSelectAll,
  totalSelected,
}: FolderSelectorProps) {
  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-[color:var(--color-primary)]" />
        <span className="ml-2 text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
          Scanning folders...
        </span>
      </div>
    );
  }

  return (
    <div data-testid="folder-selector">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
          Select Folders to Transfer
        </h3>
        <div className="flex items-center gap-3">
          <button
            onClick={onSelectAll}
            className="text-[length:var(--font-size-xs)] text-[color:var(--color-primary)] hover:underline"
          >
            Select All
          </button>
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            {formatBytes(totalSelected)} selected
          </span>
        </div>
      </div>

      {folders.length === 0 ? (
        <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic text-center py-8">
          No folders found. Select a source first.
        </p>
      ) : (
        <div className="space-y-1 max-h-[300px] overflow-y-auto">
          {folders.map((folder) => (
            <label
              key={folder.path}
              className={cn(
                "flex items-center gap-3 p-2.5 rounded-lg cursor-pointer",
                "hover:bg-[var(--color-hover-bg)] transition-theme",
                folder.selected && "bg-[var(--color-primary)]/5",
              )}
            >
              <input
                type="checkbox"
                checked={folder.selected}
                onChange={() => onToggle(folder.path)}
                className="rounded border-[var(--color-border)]"
              />
              <FolderOpen className="h-4 w-4 text-[color:var(--color-text-secondary)] shrink-0" />
              <div className="flex-1 min-w-0">
                <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)] block truncate">
                  {folder.name}
                </span>
                <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
                  {folder.fileCount.toLocaleString()} files -{" "}
                  {formatBytes(folder.size)}
                </span>
              </div>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Space Analysis View
// ──────────────────────────────────────────────

function SpaceAnalysisView({ analysis }: { analysis: SpaceAnalysis }) {
  const usedPercent =
    analysis.destTotalBytes > 0
      ? ((analysis.destTotalBytes - analysis.destFreeBytes) /
          analysis.destTotalBytes) *
        100
      : 0;

  const afterPercent =
    analysis.destTotalBytes > 0
      ? ((analysis.destTotalBytes - analysis.spaceAfterTransfer) /
          analysis.destTotalBytes) *
        100
      : 0;

  return (
    <div className="space-y-4" data-testid="space-analysis">
      {/* Space bar */}
      <div className="p-4 rounded-lg border border-[var(--color-border)]">
        <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)] mb-3">
          Space Analysis
        </h3>

        <div className="space-y-3">
          <div>
            <div className="flex justify-between text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mb-1">
              <span>Current usage</span>
              <span>{usedPercent.toFixed(0)}%</span>
            </div>
            <div className="h-2 bg-[var(--color-border)] rounded-full overflow-hidden">
              <div
                className="h-full bg-[var(--color-primary)] rounded-full"
                style={{ width: `${Math.min(usedPercent, 100)}%` }}
              />
            </div>
          </div>

          <div>
            <div className="flex justify-between text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mb-1">
              <span>After transfer</span>
              <span>{afterPercent.toFixed(0)}%</span>
            </div>
            <div className="h-2 bg-[var(--color-border)] rounded-full overflow-hidden">
              <div
                className={cn(
                  "h-full rounded-full",
                  afterPercent > 90
                    ? "bg-[var(--color-error)]"
                    : afterPercent > 70
                      ? "bg-[var(--color-warning)]"
                      : "bg-[var(--color-success)]",
                )}
                style={{ width: `${Math.min(afterPercent, 100)}%` }}
              />
            </div>
          </div>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-2 text-[length:var(--font-size-xs)]">
          <div className="text-[color:var(--color-text-secondary)]">
            Transfer size:{" "}
            <span className="font-medium text-[color:var(--color-text)]">
              {formatBytes(analysis.totalSelectedBytes)}
            </span>
          </div>
          <div className="text-[color:var(--color-text-secondary)]">
            Free after:{" "}
            <span className="font-medium text-[color:var(--color-text)]">
              {formatBytes(analysis.spaceAfterTransfer)}
            </span>
          </div>
        </div>

        {analysis.duplicateCount > 0 && (
          <div className="mt-2 text-[length:var(--font-size-xs)] text-[color:var(--color-warning)]">
            <Database className="inline h-3 w-3 mr-1" />
            {analysis.duplicateCount} duplicate files detected (
            {formatBytes(analysis.duplicateBytes)})
          </div>
        )}
      </div>

      {/* Insufficient space warning */}
      {!analysis.hasEnoughSpace && (
        <div className="p-3 rounded-lg bg-[var(--color-error)]/10 text-[color:var(--color-error)] text-[length:var(--font-size-sm)]">
          <AlertTriangle className="inline h-4 w-4 mr-1" />
          Not enough space on destination drive. Need{" "}
          {formatBytes(analysis.totalSelectedBytes - analysis.destFreeBytes)} more
          free space.
        </div>
      )}

      {/* Compatibility issues */}
      {analysis.compatIssues.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-text)]">
            Compatibility Check
          </h4>
          {analysis.compatIssues.map((issue, i) => (
            <div
              key={`${issue.code}-${i}`}
              className={cn(
                "flex items-start gap-2 p-2.5 rounded-lg text-[length:var(--font-size-xs)]",
                issue.severity === "error" && "bg-[var(--color-error)]/10 text-[color:var(--color-error)]",
                issue.severity === "warning" &&
                  "bg-[var(--color-warning)]/10 text-[color:var(--color-warning)]",
                issue.severity === "info" &&
                  "bg-[var(--color-primary)]/10 text-[color:var(--color-primary)]",
              )}
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
              <div>
                <span className="font-medium">{issue.message}</span>
                {issue.detail && (
                  <p className="mt-0.5 opacity-80">{issue.detail}</p>
                )}
                {issue.autoFix && (
                  <span className="text-[9px] font-medium uppercase mt-1 inline-block opacity-60">
                    Auto-fixable
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Preview Step
// ──────────────────────────────────────────────

interface PreviewStepProps {
  source: MigrationSource;
  destination: MigrationDestination;
  selectedFolders: string[];
  totalSize: number;
  wizardType: WizardType;
  backupMode: BackupMode;
  verifyAfterTransfer: boolean;
  onSetVerify: (v: boolean) => void;
  skipDuplicates: boolean;
  onSetSkipDuplicates: (v: boolean) => void;
  preservePermissions: boolean;
  onSetPreservePermissions: (v: boolean) => void;
}

function PreviewStep({
  source,
  destination,
  selectedFolders,
  totalSize,
  wizardType,
  backupMode,
  verifyAfterTransfer,
  onSetVerify,
  skipDuplicates,
  onSetSkipDuplicates,
  preservePermissions,
  onSetPreservePermissions,
}: PreviewStepProps) {
  return (
    <div className="space-y-4" data-testid="preview-step">
      <h3 className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
        Review and Confirm
      </h3>

      {/* Transfer summary */}
      <div className="p-4 rounded-lg border border-[var(--color-border)] space-y-2">
        <div className="flex items-center gap-2 text-[length:var(--font-size-sm)]">
          <span className="text-[color:var(--color-text)] font-medium">
            {source.name}
          </span>
          <ArrowRight className="h-4 w-4 text-[color:var(--color-text-tertiary)]" />
          <span className="text-[color:var(--color-text)] font-medium">
            {destination.name}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">
          <div>Folders: {selectedFolders.length}</div>
          <div>Total size: {formatBytes(totalSize)}</div>
          {wizardType === "backup" && (
            <div>
              Mode: {BACKUP_MODES.find((m) => m.id === backupMode)?.label}
            </div>
          )}
        </div>

        <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          {selectedFolders.slice(0, 5).map((f) => (
            <div key={f} className="truncate">
              {f}
            </div>
          ))}
          {selectedFolders.length > 5 && (
            <div>...and {selectedFolders.length - 5} more</div>
          )}
        </div>
      </div>

      {/* Options */}
      <div className="space-y-2">
        <h4 className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-text)]">
          Options
        </h4>

        <label className="flex items-center gap-2.5 p-2 rounded-lg hover:bg-[var(--color-hover-bg)] cursor-pointer">
          <input
            type="checkbox"
            checked={verifyAfterTransfer}
            onChange={(e) => onSetVerify(e.target.checked)}
            className="rounded border-[var(--color-border)]"
          />
          <div>
            <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)]">
              Verify after transfer
            </span>
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Compare checksums to ensure data integrity
            </p>
          </div>
        </label>

        <label className="flex items-center gap-2.5 p-2 rounded-lg hover:bg-[var(--color-hover-bg)] cursor-pointer">
          <input
            type="checkbox"
            checked={skipDuplicates}
            onChange={(e) => onSetSkipDuplicates(e.target.checked)}
            className="rounded border-[var(--color-border)]"
          />
          <div>
            <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)]">
              Skip duplicates
            </span>
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Skip files that already exist with the same content at destination
            </p>
          </div>
        </label>

        <label className="flex items-center gap-2.5 p-2 rounded-lg hover:bg-[var(--color-hover-bg)] cursor-pointer">
          <input
            type="checkbox"
            checked={preservePermissions}
            onChange={(e) => onSetPreservePermissions(e.target.checked)}
            className="rounded border-[var(--color-border)]"
          />
          <div>
            <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)]">
              Preserve permissions
            </span>
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Keep original file permissions where supported
            </p>
          </div>
        </label>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Executing Step
// ──────────────────────────────────────────────

function ExecutingStep({
  progress,
  onRetry,
}: {
  progress: MigrationProgress | null;
  onRetry?: () => Promise<void>;
}) {
  if (!progress) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-[color:var(--color-primary)]" />
        <span className="ml-2 text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
          Preparing migration...
        </span>
      </div>
    );
  }

  const percent =
    progress.bytesTotal > 0
      ? (progress.bytesTransferred / progress.bytesTotal) * 100
      : 0;

  return (
    <div className="space-y-4" data-testid="executing-step">
      {/* Phase */}
      <div className="text-center">
        <span
          className={cn(
            "text-[length:var(--font-size-xs)] font-medium uppercase tracking-wider",
            progress.phase === "failed"
              ? "text-[color:var(--color-error)]"
              : "text-[color:var(--color-primary)]",
          )}
        >
          {progress.phase === "preparing" && "Preparing..."}
          {progress.phase === "transferring" && "Transferring files..."}
          {progress.phase === "verifying" && "Verifying data integrity..."}
          {progress.phase === "complete" && "Complete!"}
          {progress.phase === "failed" && "Transfer failed"}
        </span>
      </div>

      {/* Progress bar */}
      <div>
        <div className="flex justify-between text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mb-1">
          <span>{percent.toFixed(1)}%</span>
          <span>
            {formatBytes(progress.bytesTransferred)} /{" "}
            {formatBytes(progress.bytesTotal)}
          </span>
        </div>
        <div className="h-3 bg-[var(--color-border)] rounded-full overflow-hidden">
          <div
            className={cn(
              "h-full rounded-full transition-all duration-300",
              progress.phase === "failed"
                ? "bg-[var(--color-error)]"
                : "bg-[var(--color-primary)]",
            )}
            style={{ width: `${Math.min(percent, 100)}%` }}
          />
        </div>
      </div>

      {/* Current file */}
      {progress.currentFile && (
        <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] truncate">
          {progress.currentFile}
        </div>
      )}

      {/* Stats */}
      <div className="grid grid-cols-3 gap-3 text-center">
        <div className="p-2 rounded-lg bg-[var(--color-hover-bg)]">
          <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            Files
          </div>
          <div className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
            {progress.filesCompleted} / {progress.filesTotal}
          </div>
        </div>
        <div className="p-2 rounded-lg bg-[var(--color-hover-bg)]">
          <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            Speed
          </div>
          <div className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
            {formatSpeed(progress.speedBps)}
          </div>
        </div>
        <div className="p-2 rounded-lg bg-[var(--color-hover-bg)]">
          <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            ETA
          </div>
          <div className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
            {progress.etaSeconds != null
              ? formatDuration(progress.etaSeconds * 1000)
              : "--"}
          </div>
        </div>
      </div>

      {/* Failed items + retry */}
      {progress.failedCount > 0 && (
        <div className="flex items-center justify-between p-3 rounded-lg bg-[var(--color-error)]/10 text-[color:var(--color-error)]">
          <span className="text-[length:var(--font-size-xs)]">
            <AlertTriangle className="inline h-3.5 w-3.5 mr-1" />
            {progress.failedCount} file(s) failed
          </span>
          {onRetry && (
            <button
              onClick={onRetry}
              className="text-[length:var(--font-size-xs)] flex items-center gap-1 px-2 py-1 rounded-md hover:bg-[var(--color-error)]/20"
            >
              <RefreshCw className="h-3 w-3" />
              Retry
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Summary Step
// ──────────────────────────────────────────────

function SummaryStep({ summary }: { summary: MigrationSummary }) {
  const allSuccess =
    summary.filesFailed === 0 && summary.verificationPassed;

  return (
    <div className="space-y-4" data-testid="summary-step">
      {/* Status banner */}
      <div
        className={cn(
          "p-4 rounded-lg text-center",
          allSuccess
            ? "bg-[var(--color-success)]/10 text-[color:var(--color-success)]"
            : "bg-[var(--color-warning)]/10 text-[color:var(--color-warning)]",
        )}
      >
        {allSuccess ? (
          <Check className="h-8 w-8 mx-auto mb-2" />
        ) : (
          <AlertTriangle className="h-8 w-8 mx-auto mb-2" />
        )}
        <h3 className="text-[length:var(--font-size-base)] font-semibold">
          {allSuccess
            ? "Migration Complete"
            : "Migration Completed with Issues"}
        </h3>
        {summary.resumed && (
          <p className="text-[length:var(--font-size-xs)] mt-1 opacity-80">
            Resumed from previous session
          </p>
        )}
      </div>

      {/* Stats grid */}
      <div className="grid grid-cols-2 gap-3">
        <StatCard
          label="Files Transferred"
          value={`${summary.filesTransferred.toLocaleString()} / ${summary.totalFiles.toLocaleString()}`}
        />
        <StatCard
          label="Data Transferred"
          value={formatBytes(summary.bytesTransferred)}
        />
        <StatCard
          label="Duration"
          value={formatDuration(summary.durationMs)}
        />
        <StatCard
          label="Average Speed"
          value={formatSpeed(summary.averageSpeedBps)}
        />
        {summary.filesFailed > 0 && (
          <StatCard
            label="Failed"
            value={summary.filesFailed.toLocaleString()}
            error
          />
        )}
        {summary.filesSkipped > 0 && (
          <StatCard
            label="Skipped"
            value={summary.filesSkipped.toLocaleString()}
          />
        )}
      </div>

      {/* Verification */}
      <div
        className={cn(
          "flex items-center gap-2 p-3 rounded-lg text-[length:var(--font-size-sm)]",
          summary.verificationPassed
            ? "bg-[var(--color-success)]/10 text-[color:var(--color-success)]"
            : "bg-[var(--color-error)]/10 text-[color:var(--color-error)]",
        )}
      >
        {summary.verificationPassed ? (
          <Check className="h-4 w-4 shrink-0" />
        ) : (
          <X className="h-4 w-4 shrink-0" />
        )}
        <span>
          {summary.verificationPassed
            ? "Data verification passed - all files match"
            : "Data verification failed - some files may be corrupted"}
        </span>
      </div>

      {/* Errors */}
      {summary.errors.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-error)]">
            Errors ({summary.errors.length})
          </h4>
          <div className="max-h-[120px] overflow-y-auto space-y-0.5">
            {summary.errors.map((err, i) => (
              <div
                key={i}
                className="text-[length:var(--font-size-xs)] text-[color:var(--color-error)] truncate"
              >
                {err}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Warnings */}
      {summary.warnings.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-warning)]">
            Warnings ({summary.warnings.length})
          </h4>
          <div className="max-h-[120px] overflow-y-auto space-y-0.5">
            {summary.warnings.map((warn, i) => (
              <div
                key={i}
                className="text-[length:var(--font-size-xs)] text-[color:var(--color-warning)] truncate"
              >
                {warn}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  error = false,
}: {
  label: string;
  value: string;
  error?: boolean;
}) {
  return (
    <div className="p-3 rounded-lg border border-[var(--color-border)]">
      <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
        {label}
      </div>
      <div
        className={cn(
          "text-[length:var(--font-size-sm)] font-semibold mt-0.5",
          error ? "text-[color:var(--color-error)]" : "text-[color:var(--color-text)]",
        )}
      >
        {value}
      </div>
    </div>
  );
}
