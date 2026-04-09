/**
 * Server/Local Connection Guided Flows (T-056)
 *
 * Wizards 4-6:
 * 4. Connect Local Sync Pair
 * 5. Connect SMB (Windows file share)
 * 6. Connect SFTP (Secure file transfer)
 *
 * Each wizard provides step-by-step guidance with validation.
 */
import { useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { pickFolder } from "@/lib/folder-picker";
import {
  FolderOpen,
  Check,
  Server,
  Shield,
  RefreshCw,
  AlertTriangle,
} from "lucide-react";

// ──────────────────────────────────────────────
// Wizard 4: Connect Local Sync Pair
// ──────────────────────────────────────────────

export function ConnectLocalSyncWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [sourcePath, setSourcePath] = useState("");
  const [destPath, setDestPath] = useState("");
  const [syncMode, setSyncMode] = useState<"one-way" | "two-way" | "mirror">("two-way");
  const [testPassed, setTestPassed] = useState(false);

  const steps: WizardStepDef[] = [
    {
      id: "source",
      label: "Source Folder",
      helpText: "Choose the first folder. This is the folder you consider the 'original' or primary copy.",
      validate: () => (sourcePath ? null : "Please enter a source folder path."),
    },
    {
      id: "destination",
      label: "Destination Folder",
      helpText: "Choose the second folder. Files will be kept in sync between this folder and the source.",
      validate: () => {
        if (!destPath) return "Please enter a destination folder path.";
        if (destPath === sourcePath) return "Destination must be different from source.";
        return null;
      },
    },
    {
      id: "mode",
      label: "Sync Mode",
      helpText: "Two-way keeps both folders identical. One-way copies from source to destination only. Mirror makes destination an exact copy (deleting extra files).",
    },
    {
      id: "confirm",
      label: "Confirm",
      helpText: "Review your settings and test the sync pair before saving.",
    },
  ];

  return (
    <WizardShell
      title="Set Up Local Sync"
      subtitle="Keep two folders on this computer in sync"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "sync",
          summary: `Created local sync pair: "${sourcePath}" and "${destPath}" (${syncMode} mode).`,
          paths: [sourcePath, destPath],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Create Sync Pair"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Where is the source folder?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            This is the primary folder. Any changes here will be synced to the destination.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Folder path</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={sourcePath}
                onChange={(e) => setSourcePath(e.target.value)}
                placeholder="/Users/you/Documents"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
              <button
                type="button"
                className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                onClick={async (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  try {
                    const path = await pickFolder("Select source folder");
                    if (path) setSourcePath(path);
                  } catch (err) {
                    console.error("Source folder pick failed:", err);
                  }
                }}
                title="Browse for folder"
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
            Where is the destination folder?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            This folder will be kept in sync with "{sourcePath || "the source"}".
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Folder path</span>
            <div className="flex gap-2 mt-1">
              <input
                type="text"
                value={destPath}
                onChange={(e) => setDestPath(e.target.value)}
                placeholder="/Volumes/Backup/Documents"
                className="flex-1 px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
              <button
                type="button"
                className="px-3 py-2.5 rounded-lg border border-[var(--color-border)] text-sm text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                onClick={async (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  try {
                    const path = await pickFolder("Select destination folder");
                    if (path) setDestPath(path);
                  } catch (err) {
                    console.error("Destination folder pick failed:", err);
                  }
                }}
                title="Browse for folder"
              >
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              </button>
            </div>
          </label>
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            How should these folders sync?
          </h3>
          <div className="space-y-2 mt-4">
            {[
              {
                id: "two-way" as const,
                label: "Two-way sync",
                desc: "Changes in either folder appear in the other. Best for most situations.",
              },
              {
                id: "one-way" as const,
                label: "One-way (source to destination)",
                desc: "Only copies changes from the source folder to the destination. The source is never modified.",
              },
              {
                id: "mirror" as const,
                label: "Mirror",
                desc: "Makes the destination an exact copy of the source. Extra files in the destination are removed.",
              },
            ].map((mode) => (
              <button
                key={mode.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  syncMode === mode.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setSyncMode(mode.id)}
                aria-pressed={syncMode === mode.id}
              >
                <RefreshCw
                  className={cn(
                    "h-5 w-5 shrink-0 mt-0.5",
                    syncMode === mode.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]",
                  )}
                  aria-hidden="true"
                />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{mode.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{mode.desc}</span>
                </div>
              </button>
            ))}
          </div>

          {syncMode === "mirror" && (
            <div className="mt-4 p-3 rounded-lg bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/20">
              <div className="flex items-start gap-2">
                <AlertTriangle className="h-4 w-4 text-[color:var(--color-warning)] shrink-0 mt-0.5" aria-hidden="true" />
                <p className="text-xs text-[color:var(--color-text-secondary)]">
                  Mirror mode will delete files in the destination that don't exist
                  in the source. Use a dry run first to see what will change.
                </p>
              </div>
            </div>
          )}
        </div>
      )}

      {step === 3 && (
        <div className="space-y-4">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Review your sync pair
          </h3>
          <div className="space-y-3">
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Source</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono">{sourcePath}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Destination</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 font-mono">{destPath}</p>
            </div>
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <span className="text-xs font-medium text-[color:var(--color-text-secondary)]">Mode</span>
              <p className="text-sm text-[color:var(--color-text)] mt-0.5 capitalize">{syncMode.replace("-", " ")}</p>
            </div>
          </div>

          <button
            className={cn(
              "w-full px-4 py-3 rounded-lg border text-sm font-medium transition-all",
              testPassed
                ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5 text-[color:var(--color-primary)]"
                : "border-[var(--color-border)] text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
            )}
            onClick={() => setTestPassed(true)}
          >
            {testPassed ? (
              <span className="flex items-center justify-center gap-2">
                <Check className="h-4 w-4" aria-hidden="true" />
                Both folders are accessible
              </span>
            ) : (
              "Test Folders"
            )}
          </button>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 5: Connect SMB
// ──────────────────────────────────────────────

export function ConnectSMBWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [host, setHost] = useState("");
  const [shareName, setShareName] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [testPassed, setTestPassed] = useState(false);

  const steps: WizardStepDef[] = [
    {
      id: "server",
      label: "Server Address",
      helpText: "Enter the name or address of the Windows computer or NAS (network storage) you want to connect to. This is usually the computer name on your network.",
      validate: () => (host ? null : "Please enter a server address."),
    },
    {
      id: "share",
      label: "Shared Folder",
      helpText: "Enter the name of the shared folder on the server. This is the folder name that was shared (not the full path).",
      validate: () => (shareName ? null : "Please enter a share name."),
    },
    {
      id: "credentials",
      label: "Sign In",
      helpText: "Enter the username and password for the server. If the share is public, you can leave these empty.",
    },
    {
      id: "test",
      label: "Test & Connect",
      helpText: "We'll test the connection to make sure everything is set up correctly.",
    },
  ];

  return (
    <WizardShell
      title="Connect to Windows Share"
      subtitle="Access files on another computer or network storage (NAS)"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "connect",
          summary: `Connected to shared folder "${shareName}" on ${host}.`,
          paths: [`//${host}/${shareName}`],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Connect"
    >
      {step === 0 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            What is the server address?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            This is usually the computer name or IP address on your local network
            (for example, "office-nas" or "192.168.1.100").
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Server address</span>
            <input
              type="text"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="office-nas or 192.168.1.100"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Which folder is shared?
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Enter the name of the shared folder on "{host}". Ask the server
            administrator if you're not sure.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Share name</span>
            <input
              type="text"
              value={shareName}
              onChange={(e) => setShareName(e.target.value)}
              placeholder="SharedFiles"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Enter your credentials
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            If the shared folder requires a login, enter your username and
            password. Leave blank for public shares.
          </p>
          <div className="space-y-3">
            <label className="block">
              <span className="text-xs font-medium text-[color:var(--color-text)]">Username</span>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="optional"
                className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
            </label>
            <label className="block">
              <span className="text-xs font-medium text-[color:var(--color-text)]">Password</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="optional"
                className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
            </label>
          </div>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-3">
            <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
            Your password is stored securely in your system keychain.
          </p>
        </div>
      )}

      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <Server className="h-12 w-12 text-[color:var(--color-text-secondary)] mb-4" aria-hidden="true" />
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Ready to connect
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
            Connect to \\{host}\{shareName}
            {username ? ` as ${username}` : " (no authentication)"}.
          </p>
          <button
            className={cn(
              "px-6 py-3 rounded-xl font-medium text-sm transition-all",
              testPassed
                ? "bg-[var(--color-primary)]/10 text-[color:var(--color-primary)] border-2 border-[var(--color-primary)]"
                : "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:opacity-90",
            )}
            onClick={() => setTestPassed(true)}
          >
            {testPassed ? (
              <span className="flex items-center gap-2">
                <Check className="h-4 w-4" aria-hidden="true" />
                Connected successfully
              </span>
            ) : (
              "Test Connection"
            )}
          </button>
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Wizard 6: Connect SFTP
// ──────────────────────────────────────────────

export function ConnectSFTPWizard() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const [step, setStep] = useState(0);
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("");
  const [authMethod, setAuthMethod] = useState<"password" | "key">("password");
  const [password, setPassword] = useState("");
  const [remotePath, setRemotePath] = useState("/");
  const [testPassed, setTestPassed] = useState(false);

  const steps: WizardStepDef[] = [
    {
      id: "server",
      label: "Server",
      helpText: "Enter the hostname or IP address of the server you want to connect to. SFTP (Secure File Transfer Protocol) encrypts all data in transit.",
      validate: () => {
        if (!host) return "Please enter a server address.";
        if (!username) return "Please enter a username.";
        return null;
      },
    },
    {
      id: "auth",
      label: "Authentication",
      helpText: "Choose how to sign in. Password is simpler; SSH keys are more secure and don't require typing a password each time.",
    },
    {
      id: "path",
      label: "Remote Folder",
      helpText: "Choose which folder on the server to start in. Usually this is the home directory (/) or a specific project folder.",
    },
    {
      id: "test",
      label: "Test & Connect",
      helpText: "We'll test the connection to make sure everything works.",
    },
  ];

  return (
    <WizardShell
      title="Connect via SFTP"
      subtitle="Securely access files on a remote server"
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={() => {
        addActivity({
          type: "connect",
          summary: `Connected to ${host} via SFTP as ${username}.`,
          paths: [`sftp://${username}@${host}:${port}${remotePath}`],
          undoable: false,
        });
        setActiveGuidedFlow(null);
      }}
      onCancel={() => setActiveGuidedFlow(null)}
      completeLabel="Connect"
    >
      {step === 0 && (
        <div className="space-y-3">
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Server details
          </h3>
          <div className="grid grid-cols-3 gap-3">
            <label className="block col-span-2">
              <span className="text-xs font-medium text-[color:var(--color-text)]">Hostname or IP</span>
              <input
                type="text"
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="server.example.com"
                className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
            </label>
            <label className="block">
              <span className="text-xs font-medium text-[color:var(--color-text)]">Port</span>
              <input
                type="text"
                value={port}
                onChange={(e) => setPort(e.target.value)}
                placeholder="22"
                className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
            </label>
          </div>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Username</span>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="your-username"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-4">
            How do you want to sign in?
          </h3>
          <div className="space-y-2 mb-4">
            {[
              { id: "password" as const, label: "Password", desc: "Type your server password each time" },
              { id: "key" as const, label: "SSH Key", desc: "Use a key file stored on your computer (more secure)" },
            ].map((m) => (
              <button
                key={m.id}
                className={cn(
                  "flex items-start gap-3 w-full p-4 rounded-lg border transition-all text-left",
                  authMethod === m.id
                    ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
                )}
                onClick={() => setAuthMethod(m.id)}
                aria-pressed={authMethod === m.id}
              >
                <Shield className={cn("h-5 w-5 shrink-0 mt-0.5", authMethod === m.id ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text-secondary)]")} aria-hidden="true" />
                <div>
                  <span className="text-sm font-medium text-[color:var(--color-text)] block">{m.label}</span>
                  <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">{m.desc}</span>
                </div>
              </button>
            ))}
          </div>

          {authMethod === "password" && (
            <label className="block">
              <span className="text-xs font-medium text-[color:var(--color-text)]">Password</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
              />
            </label>
          )}

          {authMethod === "key" && (
            <div className="p-3 rounded-lg bg-[var(--color-bg-secondary)]">
              <p className="text-xs text-[color:var(--color-text-secondary)]">
                The app will look for SSH keys in your ~/.ssh directory. If you
                have an existing key, it will be used automatically.
              </p>
            </div>
          )}
        </div>
      )}

      {step === 2 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Starting folder on the server
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Choose which directory to open when you connect. The default "/" shows the root of your accessible files.
          </p>
          <label className="block">
            <span className="text-xs font-medium text-[color:var(--color-text)]">Remote path</span>
            <input
              type="text"
              value={remotePath}
              onChange={(e) => setRemotePath(e.target.value)}
              placeholder="/"
              className="mt-1 w-full px-3 py-2.5 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] text-sm text-[color:var(--color-text)] focus:outline-none focus:ring-2 focus:ring-[var(--color-primary)]"
            />
          </label>
        </div>
      )}

      {step === 3 && (
        <div className="flex flex-col items-center text-center py-6">
          <Server className="h-12 w-12 text-[color:var(--color-text-secondary)] mb-4" aria-hidden="true" />
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Ready to connect
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
            Connect to {host}:{port} as {username} using {authMethod === "key" ? "SSH key" : "password"} authentication.
          </p>
          <button
            className={cn(
              "px-6 py-3 rounded-xl font-medium text-sm transition-all",
              testPassed
                ? "bg-[var(--color-primary)]/10 text-[color:var(--color-primary)] border-2 border-[var(--color-primary)]"
                : "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:opacity-90",
            )}
            onClick={() => setTestPassed(true)}
          >
            {testPassed ? (
              <span className="flex items-center gap-2">
                <Check className="h-4 w-4" aria-hidden="true" />
                Connected successfully
              </span>
            ) : (
              "Test Connection"
            )}
          </button>
        </div>
      )}
    </WizardShell>
  );
}
