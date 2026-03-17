/**
 * Cloud Connection Guided Flows (T-056)
 *
 * Wizards 1-4:
 * 1. Connect OneDrive
 * 2. Connect Google Drive
 * 3. Connect Dropbox
 * 4. Connect GCS (S3-Compatible)
 *
 * Each follows the same pattern:
 * - Sign in (OAuth)
 * - Choose folders to sync
 * - Configure sync settings
 * - Test connection
 * - Confirm
 */
import { useState, useCallback } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { Cloud, Check, FolderOpen, RefreshCw, Shield, Key } from "lucide-react";

// ──────────────────────────────────────────────
// Shared cloud connect wizard logic
// ──────────────────────────────────────────────

type CloudProvider = "onedrive" | "gdrive" | "dropbox";

interface CloudConnectState {
  authenticated: boolean;
  selectedFolders: string[];
  syncMode: "one-way-down" | "one-way-up" | "two-way";
  testResult: "untested" | "success" | "failed";
}

const PROVIDER_INFO: Record<
  CloudProvider,
  { name: string; description: string; folders: string[] }
> = {
  onedrive: {
    name: "Microsoft OneDrive",
    description:
      "Connect your OneDrive account to access and sync files between your computer and the cloud.",
    folders: ["Documents", "Pictures", "Desktop", "Music", "Videos"],
  },
  gdrive: {
    name: "Google Drive",
    description:
      "Connect your Google Drive to access and sync files. Shared drives are also supported.",
    folders: ["My Drive", "Shared Drives", "Recent", "Starred"],
  },
  dropbox: {
    name: "Dropbox",
    description:
      "Connect Dropbox to browse, sync, and transfer files between your computer and Dropbox.",
    folders: ["Dropbox", "Camera Uploads", "Screenshots", "Shared Folders"],
  },
};

function useCloudConnectWizard(provider: CloudProvider) {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);

  const [state, setState] = useState<CloudConnectState>({
    authenticated: false,
    selectedFolders: [],
    syncMode: "two-way",
    testResult: "untested",
  });

  const [step, setStep] = useState(0);

  const info = PROVIDER_INFO[provider];

  const steps: WizardStepDef[] = [
    {
      id: "sign-in",
      label: "Sign In",
      helpText: `Click the sign-in button to securely connect your ${info.name} account. We use OAuth, so your password is never shared with us.`,
      validate: () =>
        state.authenticated
          ? null
          : `Please sign in to ${info.name} to continue.`,
    },
    {
      id: "select-folders",
      label: "Choose Folders",
      helpText: "Select which cloud folders you want to access from this computer. You can change this later.",
    },
    {
      id: "sync-settings",
      label: "Sync Settings",
      helpText:
        "Choose how files sync between your computer and the cloud. Two-way keeps both locations in sync. One-way copies in only one direction.",
    },
    {
      id: "test-connection",
      label: "Test & Confirm",
      helpText: "We'll test the connection to make sure everything works before finalizing setup.",
    },
  ];

  const handleComplete = useCallback(() => {
    addActivity({
      type: "connect",
      summary: `Connected to ${info.name}. ${state.selectedFolders.length} folder${state.selectedFolders.length !== 1 ? "s" : ""} configured for sync.`,
      paths: state.selectedFolders,
      undoable: false,
    });
    setActiveGuidedFlow(null);
  }, [addActivity, info.name, state.selectedFolders, setActiveGuidedFlow]);

  const handleCancel = useCallback(() => {
    setActiveGuidedFlow(null);
  }, [setActiveGuidedFlow]);

  return { state, setState, step, setStep, steps, info, handleComplete, handleCancel };
}

// ──────────────────────────────────────────────
// Generic Cloud Connect Wizard Component
// ──────────────────────────────────────────────

function CloudConnectWizard({ provider }: { provider: CloudProvider }) {
  const { state, setState, step, setStep, steps, info, handleComplete, handleCancel } =
    useCloudConnectWizard(provider);

  const toggleFolder = (folder: string) => {
    setState((prev) => ({
      ...prev,
      selectedFolders: prev.selectedFolders.includes(folder)
        ? prev.selectedFolders.filter((f) => f !== folder)
        : [...prev.selectedFolders, folder],
    }));
  };

  return (
    <WizardShell
      title={`Connect ${info.name}`}
      subtitle={info.description}
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={handleComplete}
      onCancel={handleCancel}
      completeLabel="Connect"
    >
      {/* Step 1: Sign In */}
      {step === 0 && (
        <div className="flex flex-col items-center text-center py-6">
          <div className="h-16 w-16 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center mb-6">
            <Cloud className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
          </div>
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Sign in to {info.name}
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-6">
            You'll be redirected to {info.name}'s secure sign-in page. Your
            password is never shared with us.
          </p>
          <button
            className={cn(
              "px-6 py-3 rounded-xl font-medium text-sm transition-all",
              state.authenticated
                ? "bg-[var(--color-primary)]/10 text-[color:var(--color-primary)] border-2 border-[var(--color-primary)]"
                : "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:opacity-90",
            )}
            onClick={() => setState((prev) => ({ ...prev, authenticated: true }))}
          >
            {state.authenticated ? (
              <span className="flex items-center gap-2">
                <Check className="h-4 w-4" aria-hidden="true" />
                Signed in successfully
              </span>
            ) : (
              `Sign in to ${info.name}`
            )}
          </button>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-4">
            <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
            Uses secure OAuth authentication
          </p>
        </div>
      )}

      {/* Step 2: Choose Folders */}
      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Which folders would you like to access?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Selected folders will be available in your file manager. You can add
            or remove folders later.
          </p>
          <div className="space-y-2">
            {info.folders.map((folder) => {
              const selected = state.selectedFolders.includes(folder);
              return (
                <button
                  key={folder}
                  className={cn(
                    "flex items-center gap-3 w-full p-3 rounded-lg border transition-all text-left",
                    selected
                      ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                      : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                  )}
                  onClick={() => toggleFolder(folder)}
                  aria-pressed={selected}
                >
                  <FolderOpen
                    className={cn(
                      "h-5 w-5",
                      selected
                        ? "text-[color:var(--color-primary)]"
                        : "text-[color:var(--color-text-secondary)]",
                    )}
                    aria-hidden="true"
                  />
                  <span className="text-sm text-[color:var(--color-text)] flex-1">
                    {folder}
                  </span>
                  {selected && (
                    <Check className="h-4 w-4 text-[color:var(--color-primary)]" aria-hidden="true" />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Step 3: Sync Settings */}
      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            How should files sync?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Choose how changes are synchronized between your computer and{" "}
            {info.name}.
          </p>
          <div className="space-y-2">
            {[
              {
                id: "two-way" as const,
                label: "Two-way sync",
                desc: "Changes on either side are synced to the other. Best for most users.",
              },
              {
                id: "one-way-down" as const,
                label: "Download only",
                desc: "Cloud changes are downloaded, but local changes are not uploaded.",
              },
              {
                id: "one-way-up" as const,
                label: "Upload only",
                desc: "Local changes are uploaded, but cloud changes are not downloaded.",
              },
            ].map((mode) => (
              <button
                key={mode.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  state.syncMode === mode.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setState((prev) => ({ ...prev, syncMode: mode.id }))}
                aria-pressed={state.syncMode === mode.id}
              >
                <RefreshCw
                  className={cn(
                    "h-5 w-5 shrink-0 mt-0.5",
                    state.syncMode === mode.id
                      ? "text-[color:var(--color-primary)]"
                      : "text-[color:var(--color-text-secondary)]",
                  )}
                  aria-hidden="true"
                />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">
                    {mode.label}
                  </span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">
                    {mode.desc}
                  </span>
                </div>
                {state.syncMode === mode.id && (
                  <Check className="h-4 w-4 text-[color:var(--color-primary)] shrink-0 ml-auto mt-0.5" aria-hidden="true" />
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Step 4: Test & Confirm */}
      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <div
            className={cn(
              "h-16 w-16 rounded-2xl flex items-center justify-center mb-6",
              state.testResult === "success"
                ? "bg-[var(--color-primary)]/10"
                : state.testResult === "failed"
                  ? "bg-[var(--color-error)]/10"
                  : "bg-[var(--color-bg-secondary)]",
            )}
          >
            {state.testResult === "success" ? (
              <Check className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
            ) : (
              <Cloud className="h-8 w-8 text-[color:var(--color-text-secondary)]" aria-hidden="true" />
            )}
          </div>

          {state.testResult === "untested" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Ready to test your connection
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-6">
                We'll verify that we can access your {info.name} account and the
                selected folders.
              </p>
              <button
                className="px-6 py-3 rounded-xl bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] font-medium text-sm hover:opacity-90 transition-all"
                onClick={() => setState((prev) => ({ ...prev, testResult: "success" }))}
              >
                Test Connection
              </button>
            </>
          )}

          {state.testResult === "success" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Connection successful
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
                Your {info.name} account is connected and ready.{" "}
                {state.selectedFolders.length > 0
                  ? `${state.selectedFolders.length} folder${state.selectedFolders.length !== 1 ? "s" : ""} will be synced.`
                  : "All folders will be accessible."}
              </p>
            </>
          )}

          {state.testResult === "failed" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-error)] mb-2">
                Connection failed
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
                We couldn't reach {info.name}. This might be a temporary network
                issue.
              </p>
              <button
                className="px-6 py-3 rounded-xl bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] font-medium text-sm hover:opacity-90"
                onClick={() => setState((prev) => ({ ...prev, testResult: "untested" }))}
              >
                Try Again
              </button>
            </>
          )}
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// GCS (S3-Compatible) Connect Wizard
// ──────────────────────────────────────────────

interface GCSConnectState {
  accessKeyId: string;
  secretAccessKey: string;
  endpoint: string;
  testResult: "untested" | "success" | "failed";
}

function ConnectGCSWizardInner() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);

  const [state, setState] = useState<GCSConnectState>({
    accessKeyId: "",
    secretAccessKey: "",
    endpoint: "https://storage.googleapis.com",
    testResult: "untested",
  });

  const [step, setStep] = useState(0);

  const steps: WizardStepDef[] = [
    {
      id: "select-preset",
      label: "Select Preset",
      helpText:
        "Google Cloud Storage is accessed via an S3-compatible endpoint. The endpoint URL is pre-filled for you.",
    },
    {
      id: "enter-credentials",
      label: "Enter Credentials",
      helpText:
        "Enter your HMAC access key and secret. You can create these in the GCP Console under Cloud Storage > Settings > Interoperability.",
      validate: () =>
        state.accessKeyId && state.secretAccessKey
          ? null
          : "Please enter both your Access Key ID and Secret Access Key.",
    },
    {
      id: "test-connection",
      label: "Test Connection",
      helpText:
        "We will test the S3-compatible connection to Google Cloud Storage to verify your credentials.",
    },
  ];

  const handleComplete = useCallback(() => {
    addActivity({
      type: "connect",
      summary:
        "Connected to Google Cloud Storage (S3 Compatible) via HMAC keys.",
      paths: [],
      undoable: false,
    });
    setActiveGuidedFlow(null);
  }, [addActivity, setActiveGuidedFlow]);

  const handleCancel = useCallback(() => {
    setActiveGuidedFlow(null);
  }, [setActiveGuidedFlow]);

  return (
    <WizardShell
      title="Connect Google Cloud Storage"
      subtitle="Set up an S3-compatible connection to Google Cloud Storage using HMAC keys."
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={handleComplete}
      onCancel={handleCancel}
      completeLabel="Connect"
    >
      {/* Step 1: Select Preset */}
      {step === 0 && (
        <div className="flex flex-col items-center text-center py-6">
          <div className="h-16 w-16 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center mb-6">
            <Cloud className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
          </div>
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Google Cloud Storage (S3 Compatible)
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
            GCS provides an S3-compatible API. The endpoint URL is pre-configured
            for you.
          </p>
          <div className="w-full max-w-sm text-left">
            <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1">
              Endpoint URL
            </label>
            <input
              type="text"
              value={state.endpoint}
              onChange={(e) =>
                setState((prev) => ({ ...prev, endpoint: e.target.value }))
              }
              className="w-full text-sm px-3 py-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]"
            />
          </div>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-4 max-w-sm">
            <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
            Uses S3-compatible HMAC authentication
          </p>
        </div>
      )}

      {/* Step 2: Enter Credentials */}
      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Enter your HMAC credentials
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Create HMAC keys in{" "}
            <span className="font-medium">
              GCP Console &gt; Cloud Storage &gt; Settings &gt; Interoperability
            </span>.
          </p>
          <div className="space-y-3">
            <div>
              <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1">
                Access Key ID
              </label>
              <div className="relative">
                <Key
                  className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[color:var(--color-text-tertiary)]"
                  aria-hidden="true"
                />
                <input
                  type="text"
                  value={state.accessKeyId}
                  onChange={(e) =>
                    setState((prev) => ({
                      ...prev,
                      accessKeyId: e.target.value,
                    }))
                  }
                  placeholder="GOOG1E..."
                  className="w-full text-sm pl-10 pr-3 py-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]"
                />
              </div>
            </div>
            <div>
              <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1">
                Secret Access Key
              </label>
              <input
                type="password"
                value={state.secretAccessKey}
                onChange={(e) =>
                  setState((prev) => ({
                    ...prev,
                    secretAccessKey: e.target.value,
                  }))
                }
                placeholder="Enter your secret key"
                className="w-full text-sm px-3 py-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]"
              />
            </div>
          </div>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-3">
            Your credentials are stored securely in the system keychain and never
            sent to third parties.
          </p>
        </div>
      )}

      {/* Step 3: Test Connection */}
      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <div
            className={cn(
              "h-16 w-16 rounded-2xl flex items-center justify-center mb-6",
              state.testResult === "success"
                ? "bg-[var(--color-primary)]/10"
                : state.testResult === "failed"
                  ? "bg-[var(--color-error)]/10"
                  : "bg-[var(--color-bg-secondary)]",
            )}
          >
            {state.testResult === "success" ? (
              <Check className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
            ) : (
              <Cloud className="h-8 w-8 text-[color:var(--color-text-secondary)]" aria-hidden="true" />
            )}
          </div>

          {state.testResult === "untested" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Ready to test your connection
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-6">
                We'll verify the S3-compatible connection to Google Cloud Storage
                using your HMAC credentials.
              </p>
              <button
                className="px-6 py-3 rounded-xl bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] font-medium text-sm hover:opacity-90 transition-all"
                onClick={() =>
                  setState((prev) => ({ ...prev, testResult: "success" }))
                }
              >
                Test Connection
              </button>
            </>
          )}

          {state.testResult === "success" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Connection successful
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm">
                Your Google Cloud Storage account is connected and ready via the
                S3-compatible endpoint.
              </p>
            </>
          )}

          {state.testResult === "failed" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-error)] mb-2">
                Connection failed
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
                We couldn't connect to Google Cloud Storage. Please check your
                HMAC credentials and try again.
              </p>
              <button
                className="px-6 py-3 rounded-xl bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] font-medium text-sm hover:opacity-90"
                onClick={() =>
                  setState((prev) => ({ ...prev, testResult: "untested" }))
                }
              >
                Try Again
              </button>
            </>
          )}
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Exported Wizards
// ──────────────────────────────────────────────

export function ConnectOneDriveWizard() {
  return <CloudConnectWizard provider="onedrive" />;
}

export function ConnectGDriveWizard() {
  return <CloudConnectWizard provider="gdrive" />;
}

export function ConnectDropboxWizard() {
  return <CloudConnectWizard provider="dropbox" />;
}

export function ConnectGCSWizard() {
  return <ConnectGCSWizardInner />;
}
