/**
 * Onboarding Wizard (T-055)
 *
 * First-run wizard with 6 steps:
 * 1. Welcome - Intro screen
 * 2. Choose Style - Personal / Power / Work
 * 3. Connect Locations - OAuth cloud connections + local folders
 * 4. First Action - Quick actions to get started
 * 5. Explain Compatibility - How cross-platform file naming works
 * 6. Enter Workspace - You're all set
 *
 * Features:
 * - Skip option at every step
 * - Re-accessible from Help > Getting Started
 * - Remembers completion state
 */
import { useState, useCallback } from "react";
import { useUIStore, type OnboardingStep, type UserStyle } from "@/stores/ui-store";
import { cn } from "@ufop/ui-components";
import { Button } from "@ufop/ui-components";
import {
  FolderOpen,
  User,
  Zap,
  Briefcase,
  Cloud,
  HardDrive,
  ArrowRight,
  ArrowLeft,
  Check,
  Shield,
  Sparkles,
  RefreshCw,
  Send,
} from "lucide-react";

// ──────────────────────────────────────────────
// Step definitions
// ──────────────────────────────────────────────

const STEPS: OnboardingStep[] = [
  "welcome",
  "choose-style",
  "connect-locations",
  "first-action",
  "explain-compat",
  "enter-workspace",
];

const STEP_LABELS: Record<OnboardingStep, string> = {
  "welcome": "Welcome",
  "choose-style": "Your Style",
  "connect-locations": "Locations",
  "first-action": "Quick Start",
  "explain-compat": "Compatibility",
  "enter-workspace": "Ready",
};

// ──────────────────────────────────────────────
// Main Component
// ──────────────────────────────────────────────

interface OnboardingWizardProps {
  /** Called when the wizard is completed or skipped */
  onComplete: () => void;
  /** Called to open a guided flow from within the wizard */
  onOpenGuidedFlow?: (flowId: string) => void;
}

export function OnboardingWizard({ onComplete, onOpenGuidedFlow }: OnboardingWizardProps) {
  const onboardingStep = useUIStore((s) => s.onboardingStep);
  const setOnboardingStep = useUIStore((s) => s.setOnboardingStep);
  const setOnboardingComplete = useUIStore((s) => s.setOnboardingComplete);
  const userStyle = useUIStore((s) => s.userStyle);
  const setUserStyle = useUIStore((s) => s.setUserStyle);
  const addActivity = useUIStore((s) => s.addActivity);

  const currentIndex = STEPS.indexOf(onboardingStep);

  const handleNext = useCallback(() => {
    const nextIndex = currentIndex + 1;
    if (nextIndex >= STEPS.length) {
      setOnboardingComplete(true);
      addActivity({
        type: "connect",
        summary: "Completed the onboarding wizard and set up your workspace.",
        paths: [],
        undoable: false,
      });
      onComplete();
    } else {
      setOnboardingStep(STEPS[nextIndex]);
    }
  }, [currentIndex, setOnboardingStep, setOnboardingComplete, addActivity, onComplete]);

  const handleBack = useCallback(() => {
    const prevIndex = currentIndex - 1;
    if (prevIndex >= 0) {
      setOnboardingStep(STEPS[prevIndex]);
    }
  }, [currentIndex, setOnboardingStep]);

  const handleSkip = useCallback(() => {
    setOnboardingComplete(true);
    addActivity({
      type: "connect",
      summary: "Skipped the onboarding wizard. You can re-run it from Help > Getting Started.",
      paths: [],
      undoable: false,
    });
    onComplete();
  }, [setOnboardingComplete, addActivity, onComplete]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Setup wizard"
    >
      <div className="w-full max-w-2xl mx-4 bg-[var(--color-bg)] rounded-2xl shadow-2xl overflow-hidden">
        {/* Step indicator */}
        <div className="flex items-center justify-between px-6 pt-5">
          <div className="flex items-center gap-2">
            {STEPS.map((step, i) => (
              <div key={step} className="flex items-center">
                <div
                  className={cn(
                    "h-2.5 w-2.5 rounded-full transition-all",
                    i < currentIndex
                      ? "bg-[var(--color-primary)]"
                      : i === currentIndex
                        ? "bg-[var(--color-primary)] ring-4 ring-[var(--color-primary)]/20"
                        : "bg-[var(--color-border)]",
                  )}
                  title={STEP_LABELS[step]}
                />
                {i < STEPS.length - 1 && (
                  <div
                    className={cn(
                      "h-0.5 w-6 mx-1",
                      i < currentIndex
                        ? "bg-[var(--color-primary)]"
                        : "bg-[var(--color-border)]",
                    )}
                  />
                )}
              </div>
            ))}
          </div>
          <button
            onClick={handleSkip}
            className="text-xs text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] transition-colors"
            aria-label="Skip setup wizard"
          >
            Skip
          </button>
        </div>

        {/* Step content */}
        <div className="px-8 py-6 min-h-[380px]">
          {onboardingStep === "welcome" && <WelcomeStep />}
          {onboardingStep === "choose-style" && (
            <ChooseStyleStep userStyle={userStyle} onSelect={setUserStyle} />
          )}
          {onboardingStep === "connect-locations" && (
            <ConnectLocationsStep onOpenGuidedFlow={onOpenGuidedFlow} />
          )}
          {onboardingStep === "first-action" && (
            <FirstActionStep onOpenGuidedFlow={onOpenGuidedFlow} />
          )}
          {onboardingStep === "explain-compat" && <ExplainCompatStep />}
          {onboardingStep === "enter-workspace" && <EnterWorkspaceStep />}
        </div>

        {/* Navigation buttons */}
        <div className="flex items-center justify-between px-8 pb-6">
          <div>
            {currentIndex > 0 && (
              <Button variant="ghost" onClick={handleBack}>
                <ArrowLeft className="h-4 w-4 mr-2" aria-hidden="true" />
                Back
              </Button>
            )}
          </div>
          <Button variant="default" onClick={handleNext}>
            {currentIndex === STEPS.length - 1 ? (
              <>
                Get Started
                <Check className="h-4 w-4 ml-2" aria-hidden="true" />
              </>
            ) : (
              <>
                Continue
                <ArrowRight className="h-4 w-4 ml-2" aria-hidden="true" />
              </>
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 1: Welcome
// ──────────────────────────────────────────────

function WelcomeStep() {
  return (
    <div className="flex flex-col items-center text-center">
      <div className="h-16 w-16 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center mb-6">
        <FolderOpen className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
      </div>
      <h2 className="text-2xl font-bold text-[color:var(--color-text)] mb-3">
        Welcome to File Manager
      </h2>
      <p className="text-[color:var(--color-text-secondary)] max-w-md leading-relaxed">
        Manage, transfer, and sync your files across computers, drives, and cloud
        services -- all from one app. Let's get you set up in under a minute.
      </p>
      <div className="flex items-center gap-6 mt-8 text-sm text-[color:var(--color-text-tertiary)]">
        <div className="flex items-center gap-2">
          <HardDrive className="h-4 w-4" aria-hidden="true" />
          Local drives
        </div>
        <div className="flex items-center gap-2">
          <Cloud className="h-4 w-4" aria-hidden="true" />
          Cloud storage
        </div>
        <div className="flex items-center gap-2">
          <Shield className="h-4 w-4" aria-hidden="true" />
          Secure transfers
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 2: Choose Style
// ──────────────────────────────────────────────

function ChooseStyleStep({
  userStyle,
  onSelect,
}: {
  userStyle: UserStyle;
  onSelect: (style: UserStyle) => void;
}) {
  const styles: {
    id: UserStyle;
    icon: React.ReactNode;
    label: string;
    description: string;
  }[] = [
    {
      id: "personal",
      icon: <User className="h-6 w-6" aria-hidden="true" />,
      label: "Personal",
      description:
        "I manage photos, documents, and music. Keep things simple and organized.",
    },
    {
      id: "power",
      icon: <Zap className="h-6 w-6" aria-hidden="true" />,
      label: "Power User",
      description:
        "I need advanced features like regex, checksums, scripting, and detailed logs.",
    },
    {
      id: "work",
      icon: <Briefcase className="h-6 w-6" aria-hidden="true" />,
      label: "Workplace",
      description:
        "I share files with a team, manage servers, and need compliance features.",
    },
  ];

  return (
    <div>
      <h2 className="text-xl font-bold text-[color:var(--color-text)] mb-2">
        How will you use File Manager?
      </h2>
      <p className="text-sm text-[color:var(--color-text-secondary)] mb-6">
        This customizes your experience. You can change this anytime in Settings.
      </p>

      <div className="space-y-3">
        {styles.map((style) => (
          <button
            key={style.id}
            className={cn(
              "flex items-start gap-4 w-full p-4 rounded-xl border-2 text-left transition-all",
              userStyle === style.id
                ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
            )}
            onClick={() => onSelect(style.id)}
            aria-pressed={userStyle === style.id}
          >
            <div
              className={cn(
                "h-10 w-10 rounded-lg flex items-center justify-center shrink-0",
                userStyle === style.id
                  ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
                  : "bg-[var(--color-bg-secondary)] text-[color:var(--color-text-secondary)]",
              )}
            >
              {style.icon}
            </div>
            <div>
              <span className="text-sm font-semibold text-[color:var(--color-text)] block">
                {style.label}
              </span>
              <span className="text-xs text-[color:var(--color-text-secondary)] mt-0.5 block leading-relaxed">
                {style.description}
              </span>
            </div>
            {userStyle === style.id && (
              <Check className="h-5 w-5 text-[color:var(--color-primary)] shrink-0 ml-auto mt-2" aria-hidden="true" />
            )}
          </button>
        ))}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 3: Connect Locations
// ──────────────────────────────────────────────

function ConnectLocationsStep({
  onOpenGuidedFlow,
}: {
  onOpenGuidedFlow?: (flowId: string) => void;
}) {
  const [connected, setConnected] = useState<string[]>([]);

  const locations = [
    {
      id: "onedrive",
      label: "Microsoft OneDrive",
      icon: <Cloud className="h-5 w-5" aria-hidden="true" />,
      flowId: "connect-onedrive",
    },
    {
      id: "gdrive",
      label: "Google Drive",
      icon: <Cloud className="h-5 w-5" aria-hidden="true" />,
      flowId: "connect-gdrive",
    },
    {
      id: "dropbox",
      label: "Dropbox",
      icon: <Cloud className="h-5 w-5" aria-hidden="true" />,
      flowId: "connect-dropbox",
    },
    {
      id: "local",
      label: "Local Folder",
      icon: <HardDrive className="h-5 w-5" aria-hidden="true" />,
      flowId: "connect-local-sync",
    },
  ];

  const handleConnect = useCallback(
    (locationId: string, flowId: string) => {
      if (onOpenGuidedFlow) {
        onOpenGuidedFlow(flowId);
      }
      // Simulate connection for the wizard flow
      setConnected((prev) =>
        prev.includes(locationId)
          ? prev.filter((id) => id !== locationId)
          : [...prev, locationId],
      );
    },
    [onOpenGuidedFlow],
  );

  return (
    <div>
      <h2 className="text-xl font-bold text-[color:var(--color-text)] mb-2">
        Connect your storage
      </h2>
      <p className="text-sm text-[color:var(--color-text-secondary)] mb-6">
        Link cloud services or folders you use often. You can add more later.
      </p>

      <div className="space-y-2">
        {locations.map((loc) => {
          const isConnected = connected.includes(loc.id);
          return (
            <button
              key={loc.id}
              className={cn(
                "flex items-center gap-3 w-full p-4 rounded-xl border transition-all",
                isConnected
                  ? "border-[var(--color-primary)] bg-[var(--color-primary)]/5"
                  : "border-[var(--color-border)] hover:border-[var(--color-text-tertiary)]",
              )}
              onClick={() => handleConnect(loc.id, loc.flowId)}
            >
              <div
                className={cn(
                  "h-10 w-10 rounded-lg flex items-center justify-center",
                  isConnected
                    ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
                    : "bg-[var(--color-bg-secondary)] text-[color:var(--color-text-secondary)]",
                )}
              >
                {loc.icon}
              </div>
              <span className="text-sm font-medium text-[color:var(--color-text)] flex-1 text-left">
                {loc.label}
              </span>
              {isConnected ? (
                <Check className="h-5 w-5 text-[color:var(--color-primary)]" aria-hidden="true" />
              ) : (
                <span className="text-xs text-[color:var(--color-text-tertiary)]">
                  Connect
                </span>
              )}
            </button>
          );
        })}
      </div>

      <p className="text-xs text-[color:var(--color-text-tertiary)] mt-4">
        No account needed for local folders. Cloud services use secure sign-in (OAuth).
      </p>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 4: First Action
// ──────────────────────────────────────────────

function FirstActionStep({
  onOpenGuidedFlow,
}: {
  onOpenGuidedFlow?: (flowId: string) => void;
}) {
  const actions = [
    {
      id: "transfer",
      label: "Transfer files between locations",
      description: "Copy or move files from one place to another",
      icon: <Send className="h-5 w-5" aria-hidden="true" />,
      flowId: "start-transfer",
    },
    {
      id: "sync",
      label: "Set up folder sync",
      description: "Keep two folders automatically in sync",
      icon: <RefreshCw className="h-5 w-5" aria-hidden="true" />,
      flowId: "create-sync",
    },
    {
      id: "backup",
      label: "Back up a folder",
      description: "Create a safe copy of important files",
      icon: <Shield className="h-5 w-5" aria-hidden="true" />,
      flowId: "backup-folder",
    },
    {
      id: "browse",
      label: "Just browse my files",
      description: "Open the file manager and explore",
      icon: <FolderOpen className="h-5 w-5" aria-hidden="true" />,
      flowId: "",
    },
  ];

  return (
    <div>
      <h2 className="text-xl font-bold text-[color:var(--color-text)] mb-2">
        What would you like to do first?
      </h2>
      <p className="text-sm text-[color:var(--color-text-secondary)] mb-6">
        Pick an action to get started, or skip to explore on your own.
      </p>

      <div className="space-y-2">
        {actions.map((action) => (
          <button
            key={action.id}
            className={cn(
              "flex items-center gap-3 w-full p-4 rounded-xl border",
              "border-[var(--color-border)] hover:border-[var(--color-primary)] hover:bg-[var(--color-primary)]/5",
              "transition-all text-left",
            )}
            onClick={() => {
              if (action.flowId && onOpenGuidedFlow) {
                onOpenGuidedFlow(action.flowId);
              }
            }}
          >
            <div className="h-10 w-10 rounded-lg bg-[var(--color-bg-secondary)] flex items-center justify-center text-[color:var(--color-primary)]">
              {action.icon}
            </div>
            <div className="flex-1 min-w-0">
              <span className="text-sm font-medium text-[color:var(--color-text)] block">
                {action.label}
              </span>
              <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">
                {action.description}
              </span>
            </div>
            <ArrowRight className="h-4 w-4 text-[color:var(--color-text-tertiary)]" aria-hidden="true" />
          </button>
        ))}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 5: Explain Compatibility
// ──────────────────────────────────────────────

function ExplainCompatStep() {
  return (
    <div>
      <h2 className="text-xl font-bold text-[color:var(--color-text)] mb-2">
        Cross-platform compatibility
      </h2>
      <p className="text-sm text-[color:var(--color-text-secondary)] mb-6">
        File Manager helps your files work everywhere -- Mac, Windows, Linux, and
        cloud services -- by watching for naming issues.
      </p>

      <div className="space-y-4">
        <div className="flex items-start gap-3 p-3 rounded-lg bg-[var(--color-bg-secondary)]">
          <Shield className="h-5 w-5 text-[color:var(--color-primary)] shrink-0 mt-0.5" aria-hidden="true" />
          <div>
            <span className="text-sm font-medium text-[color:var(--color-text)] block">
              Automatic detection
            </span>
            <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">
              The app warns you when a filename might cause problems on another
              system (for example, "CON" or "file:name" are not allowed on Windows).
            </span>
          </div>
        </div>

        <div className="flex items-start gap-3 p-3 rounded-lg bg-[var(--color-bg-secondary)]">
          <Sparkles className="h-5 w-5 text-[color:var(--color-primary)] shrink-0 mt-0.5" aria-hidden="true" />
          <div>
            <span className="text-sm font-medium text-[color:var(--color-text)] block">
              Safe renaming
            </span>
            <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">
              When transferring files, the app can automatically fix problematic
              names while keeping the originals safe.
            </span>
          </div>
        </div>

        <div className="flex items-start gap-3 p-3 rounded-lg bg-[var(--color-bg-secondary)]">
          <RefreshCw className="h-5 w-5 text-[color:var(--color-primary)] shrink-0 mt-0.5" aria-hidden="true" />
          <div>
            <span className="text-sm font-medium text-[color:var(--color-text)] block">
              Reversible changes
            </span>
            <span className="text-xs text-[color:var(--color-text-secondary)] block mt-0.5">
              Every rename can be undone. The original names are stored so you can
              restore them at any time.
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Step 6: Enter Workspace
// ──────────────────────────────────────────────

function EnterWorkspaceStep() {
  const userStyle = useUIStore((s) => s.userStyle);

  return (
    <div className="flex flex-col items-center text-center">
      <div className="h-16 w-16 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center mb-6">
        <Check className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
      </div>
      <h2 className="text-2xl font-bold text-[color:var(--color-text)] mb-3">
        You're all set!
      </h2>
      <p className="text-[color:var(--color-text-secondary)] max-w-md leading-relaxed">
        {userStyle === "personal" && (
          <>
            Your workspace is ready with a clean, simple interface. Browse your
            files, set up sync, or connect cloud storage whenever you need it.
          </>
        )}
        {userStyle === "power" && (
          <>
            Advanced mode is enabled with regex renaming, checksums, scripting, and
            detailed logs. You have full control over every operation.
          </>
        )}
        {userStyle === "work" && (
          <>
            Your workspace is configured for team use with server connections and
            compliance features. You can add more connections from the Cloud &
            Servers section.
          </>
        )}
      </p>
      <div className="mt-6 p-4 rounded-xl bg-[var(--color-bg-secondary)] text-sm text-[color:var(--color-text-secondary)]">
        You can re-run this wizard anytime from{" "}
        <strong>Help &gt; Getting Started</strong> or{" "}
        <strong>Settings</strong>.
      </div>
    </div>
  );
}
