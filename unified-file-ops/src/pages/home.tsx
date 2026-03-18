import { useState, useCallback } from "react";
import { FileManager } from "@/components/file-manager";
import { SimpleModeWrapper } from "@/components/simple-mode";
import { OnboardingWizard } from "@/components/onboarding-wizard";
import { GuidedFlowRouter } from "@/components/guided-flows/guided-flow-router";
import { useUIStore } from "@/stores/ui-store";

/**
 * Home page -- the main file manager view.
 *
 * Orchestrates:
 * - Simple mode wrapper (T-054) - controls which mode renders
 * - Onboarding wizard (T-055) - first-run experience
 * - Guided flows (T-056) - step-by-step wizards
 * - Content design (T-057) - activity feed, structured errors
 *
 * In Simple mode (default), shows a streamlined interface.
 * In Advanced mode, shows all features.
 */
export function HomePage() {
  const onboardingComplete = useUIStore((s) => s.onboardingComplete);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);

  // Allow re-opening onboarding from Help > Getting Started
  const [showOnboarding, setShowOnboarding] = useState(!onboardingComplete);

  const handleOnboardingComplete = useCallback(() => {
    setShowOnboarding(false);
  }, []);

  const handleOpenOnboarding = useCallback(() => {
    useUIStore.getState().resetOnboarding();
    setShowOnboarding(true);
  }, []);

  const handleOpenGuidedFlow = useCallback(
    (flowId: string) => {
      setActiveGuidedFlow(flowId);
    },
    [setActiveGuidedFlow],
  );

  return (
    <>
      {/* Main app content */}
      <SimpleModeWrapper
        fileManager={<FileManager />}
        onOpenOnboarding={handleOpenOnboarding}
        onOpenGuidedFlow={handleOpenGuidedFlow}
      />

      {/* Onboarding wizard (T-055) */}
      {showOnboarding && (
        <OnboardingWizard
          onComplete={handleOnboardingComplete}
          onOpenGuidedFlow={handleOpenGuidedFlow}
        />
      )}

      {/* Guided flow router (T-056) */}
      <GuidedFlowRouter />
    </>
  );
}
