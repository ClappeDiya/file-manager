/**
 * Wizard Shell (T-056)
 *
 * Shared wizard framework for all 15 guided flows.
 * Provides:
 * - Step indicator with progress
 * - Back / Next / Cancel navigation
 * - Contextual help panel
 * - Validation per step
 * - Consistent styling
 */
import { useState, useCallback } from "react";
import { cn } from "@ufop/ui-components";
import { Button } from "@ufop/ui-components";
import {
  ArrowLeft,
  ArrowRight,
  X,
  Check,
  HelpCircle,
  Loader2,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export interface WizardStepDef {
  id: string;
  label: string;
  /** Help text shown in the contextual help panel */
  helpText?: string;
  /** Validate this step before allowing next. Return error message or null. */
  validate?: () => string | null;
}

interface WizardShellProps {
  /** Wizard title */
  title: string;
  /** Subtitle / description */
  subtitle?: string;
  /** Step definitions (order matters) */
  steps: WizardStepDef[];
  /** Current step index */
  currentStep: number;
  /** Callback to change step */
  onStepChange: (index: number) => void;
  /** Callback when wizard completes (last step Next) */
  onComplete: () => void;
  /** Callback when wizard is cancelled */
  onCancel: () => void;
  /** Whether the wizard is processing (shows spinner on Next) */
  loading?: boolean;
  /** Step content */
  children: React.ReactNode;
  /** Override the next button label */
  nextLabel?: string;
  /** Override the complete button label */
  completeLabel?: string;
}

// ──────────────────────────────────────────────
// Component
// ──────────────────────────────────────────────

export function WizardShell({
  title,
  subtitle,
  steps,
  currentStep,
  onStepChange,
  onComplete,
  onCancel,
  loading = false,
  children,
  nextLabel,
  completeLabel,
}: WizardShellProps) {
  const [showHelp, setShowHelp] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const isLastStep = currentStep === steps.length - 1;
  const isFirstStep = currentStep === 0;
  const currentStepDef = steps[currentStep];

  const handleNext = useCallback(() => {
    // Run validation if defined
    if (currentStepDef?.validate) {
      const error = currentStepDef.validate();
      if (error) {
        setValidationError(error);
        return;
      }
    }
    setValidationError(null);

    if (isLastStep) {
      onComplete();
    } else {
      onStepChange(currentStep + 1);
    }
  }, [currentStep, currentStepDef, isLastStep, onComplete, onStepChange]);

  const handleBack = useCallback(() => {
    setValidationError(null);
    if (!isFirstStep) {
      onStepChange(currentStep - 1);
    }
  }, [currentStep, isFirstStep, onStepChange]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className="w-full max-w-2xl mx-4 bg-[var(--color-bg)] rounded-2xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
          <div>
            <h2 className="text-base font-semibold text-[color:var(--color-text)]">
              {title}
            </h2>
            {subtitle && (
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-0.5">
                {subtitle}
              </p>
            )}
          </div>
          <div className="flex items-center gap-2">
            {currentStepDef?.helpText && (
              <Button
                variant="ghost"
                size="icon"
                onClick={() => setShowHelp(!showHelp)}
                aria-label={showHelp ? "Hide help" : "Show help"}
                aria-pressed={showHelp}
              >
                <HelpCircle className="h-4 w-4" aria-hidden="true" />
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              onClick={onCancel}
              aria-label="Cancel wizard"
            >
              <X className="h-4 w-4" aria-hidden="true" />
            </Button>
          </div>
        </div>

        {/* Step indicator */}
        <div className="px-6 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
          <div className="flex items-center gap-1">
            {steps.map((step, i) => (
              <div key={step.id} className="flex items-center flex-1">
                <div className="flex items-center gap-2 flex-1">
                  <div
                    className={cn(
                      "h-6 w-6 rounded-full flex items-center justify-center text-xs font-medium shrink-0",
                      i < currentStep
                        ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
                        : i === currentStep
                          ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] ring-4 ring-[var(--color-primary)]/20"
                          : "bg-[var(--color-border)] text-[color:var(--color-text-tertiary)]",
                    )}
                  >
                    {i < currentStep ? (
                      <Check className="h-3.5 w-3.5" aria-hidden="true" />
                    ) : (
                      i + 1
                    )}
                  </div>
                  <span
                    className={cn(
                      "text-xs truncate hidden sm:block",
                      i === currentStep
                        ? "text-[color:var(--color-text)] font-medium"
                        : "text-[color:var(--color-text-tertiary)]",
                    )}
                  >
                    {step.label}
                  </span>
                </div>
                {i < steps.length - 1 && (
                  <div
                    className={cn(
                      "h-0.5 flex-1 mx-2",
                      i < currentStep
                        ? "bg-[var(--color-primary)]"
                        : "bg-[var(--color-border)]",
                    )}
                  />
                )}
              </div>
            ))}
          </div>
        </div>

        {/* Help panel (contextual) */}
        {showHelp && currentStepDef?.helpText && (
          <div className="px-6 py-3 bg-[var(--color-primary)]/5 border-b border-[var(--color-primary)]/20">
            <p className="text-xs text-[color:var(--color-text-secondary)] leading-relaxed">
              {currentStepDef.helpText}
            </p>
          </div>
        )}

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-5">
          {children}

          {/* Validation error */}
          {validationError && (
            <div
              className="mt-4 p-3 rounded-lg bg-[var(--color-error)]/5 border border-[var(--color-error)]/20"
              role="alert"
            >
              <p className="text-xs text-[color:var(--color-error)]">
                {validationError}
              </p>
            </div>
          )}
        </div>

        {/* Footer with navigation */}
        <div className="flex items-center justify-between px-6 py-4 border-t border-[var(--color-border)]">
          <div>
            {!isFirstStep && (
              <Button variant="ghost" onClick={handleBack} disabled={loading}>
                <ArrowLeft className="h-4 w-4 mr-2" aria-hidden="true" />
                Back
              </Button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button variant="ghost" onClick={onCancel} disabled={loading}>
              Cancel
            </Button>
            <Button variant="default" onClick={handleNext} disabled={loading}>
              {loading ? (
                <>
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" aria-hidden="true" />
                  Processing...
                </>
              ) : isLastStep ? (
                <>
                  {completeLabel || "Finish"}
                  <Check className="h-4 w-4 ml-2" aria-hidden="true" />
                </>
              ) : (
                <>
                  {nextLabel || "Next"}
                  <ArrowRight className="h-4 w-4 ml-2" aria-hidden="true" />
                </>
              )}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
