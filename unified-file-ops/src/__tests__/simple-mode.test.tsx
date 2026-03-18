/**
 * Tests for Simple Mode (T-054), Onboarding (T-055),
 * Activity Feed (T-057), and UI Store enhancements.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { act } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

// ──────────────────────────────────────────────
// UI Store - Mode and Onboarding Tests
// ──────────────────────────────────────────────

describe("useUIStore - AppMode", () => {
  beforeEach(() => {
    const store = useUIStore.getState();
    store.setAppMode("simple");
    store.setOnboardingComplete(false);
    store.setOnboardingStep("welcome");
    store.clearActivity();
    store.clearErrors();
  });

  it("defaults to simple mode", () => {
    const { appMode } = useUIStore.getState();
    expect(appMode).toBe("simple");
  });

  it("switches to advanced mode instantly", () => {
    act(() => {
      useUIStore.getState().setAppMode("advanced");
    });
    expect(useUIStore.getState().appMode).toBe("advanced");
  });

  it("switches back to simple mode instantly", () => {
    act(() => {
      useUIStore.getState().setAppMode("advanced");
      useUIStore.getState().setAppMode("simple");
    });
    expect(useUIStore.getState().appMode).toBe("simple");
  });
});

describe("useUIStore - Onboarding", () => {
  beforeEach(() => {
    const store = useUIStore.getState();
    store.setOnboardingComplete(false);
    store.setOnboardingStep("welcome");
    store.setUserStyle("personal");
  });

  it("starts with onboarding incomplete", () => {
    expect(useUIStore.getState().onboardingComplete).toBe(false);
  });

  it("starts at welcome step", () => {
    expect(useUIStore.getState().onboardingStep).toBe("welcome");
  });

  it("progresses through steps", () => {
    act(() => {
      useUIStore.getState().setOnboardingStep("choose-style");
    });
    expect(useUIStore.getState().onboardingStep).toBe("choose-style");
  });

  it("sets user style to personal (simple mode)", () => {
    act(() => {
      useUIStore.getState().setUserStyle("personal");
    });
    expect(useUIStore.getState().userStyle).toBe("personal");
    expect(useUIStore.getState().appMode).toBe("simple");
  });

  it("sets user style to power (advanced mode)", () => {
    act(() => {
      useUIStore.getState().setUserStyle("power");
    });
    expect(useUIStore.getState().userStyle).toBe("power");
    expect(useUIStore.getState().appMode).toBe("advanced");
  });

  it("sets user style to work (simple mode)", () => {
    act(() => {
      useUIStore.getState().setUserStyle("work");
    });
    expect(useUIStore.getState().userStyle).toBe("work");
    expect(useUIStore.getState().appMode).toBe("simple");
  });

  it("marks onboarding as complete", () => {
    act(() => {
      useUIStore.getState().setOnboardingComplete(true);
    });
    expect(useUIStore.getState().onboardingComplete).toBe(true);
  });

  it("resets onboarding", () => {
    act(() => {
      useUIStore.getState().setOnboardingComplete(true);
      useUIStore.getState().setOnboardingStep("enter-workspace");
      useUIStore.getState().resetOnboarding();
    });
    expect(useUIStore.getState().onboardingComplete).toBe(false);
    expect(useUIStore.getState().onboardingStep).toBe("welcome");
  });
});

// ──────────────────────────────────────────────
// Activity Feed Tests (T-057)
// ──────────────────────────────────────────────

describe("useUIStore - Activity Feed", () => {
  beforeEach(() => {
    useUIStore.getState().clearActivity();
  });

  it("starts with empty activity feed", () => {
    expect(useUIStore.getState().activityFeed).toHaveLength(0);
  });

  it("adds an activity entry", () => {
    act(() => {
      useUIStore.getState().addActivity({
        type: "copy",
        summary: "Copied 5 files to Documents",
        paths: ["/Users/test/Documents"],
        undoable: true,
      });
    });
    const feed = useUIStore.getState().activityFeed;
    expect(feed).toHaveLength(1);
    expect(feed[0].type).toBe("copy");
    expect(feed[0].summary).toBe("Copied 5 files to Documents");
    expect(feed[0].undoable).toBe(true);
    expect(feed[0].id).toBeTruthy();
    expect(feed[0].timestamp).toBeTruthy();
  });

  it("maintains newest-first order", () => {
    act(() => {
      useUIStore.getState().addActivity({
        type: "copy",
        summary: "First action",
        paths: [],
        undoable: false,
      });
      useUIStore.getState().addActivity({
        type: "move",
        summary: "Second action",
        paths: [],
        undoable: false,
      });
    });
    const feed = useUIStore.getState().activityFeed;
    expect(feed).toHaveLength(2);
    expect(feed[0].summary).toBe("Second action");
    expect(feed[1].summary).toBe("First action");
  });

  it("limits to 100 entries", () => {
    act(() => {
      for (let i = 0; i < 110; i++) {
        useUIStore.getState().addActivity({
          type: "copy",
          summary: `Action ${i}`,
          paths: [],
          undoable: false,
        });
      }
    });
    expect(useUIStore.getState().activityFeed).toHaveLength(100);
  });

  it("clears activity feed", () => {
    act(() => {
      useUIStore.getState().addActivity({
        type: "copy",
        summary: "Some action",
        paths: [],
        undoable: false,
      });
      useUIStore.getState().clearActivity();
    });
    expect(useUIStore.getState().activityFeed).toHaveLength(0);
  });
});

// ──────────────────────────────────────────────
// Structured Errors Tests (T-057)
// ──────────────────────────────────────────────

describe("useUIStore - Structured Errors", () => {
  beforeEach(() => {
    useUIStore.getState().clearErrors();
  });

  it("starts with no errors", () => {
    expect(useUIStore.getState().structuredErrors).toHaveLength(0);
  });

  it("adds a structured error with what/why/did/next", () => {
    act(() => {
      useUIStore.getState().addStructuredError({
        what: "Could not copy file.txt",
        why: "The destination drive is full.",
        appDid: "Stopped the transfer and kept your original file safe.",
        userAction: "Free up space on the destination drive, then try again.",
      });
    });
    const errors = useUIStore.getState().structuredErrors;
    expect(errors).toHaveLength(1);
    expect(errors[0].what).toBe("Could not copy file.txt");
    expect(errors[0].why).toBe("The destination drive is full.");
    expect(errors[0].appDid).toBe("Stopped the transfer and kept your original file safe.");
    expect(errors[0].userAction).toBe("Free up space on the destination drive, then try again.");
    expect(errors[0].dismissed).toBe(false);
  });

  it("dismisses an error", () => {
    act(() => {
      useUIStore.getState().addStructuredError({
        what: "Error",
        why: "Reason",
        appDid: "Action",
        userAction: "Fix",
      });
    });
    const errorId = useUIStore.getState().structuredErrors[0].id;
    act(() => {
      useUIStore.getState().dismissError(errorId);
    });
    expect(useUIStore.getState().structuredErrors[0].dismissed).toBe(true);
  });

  it("clears all errors", () => {
    act(() => {
      useUIStore.getState().addStructuredError({
        what: "Error 1",
        why: "R1",
        appDid: "A1",
        userAction: "U1",
      });
      useUIStore.getState().addStructuredError({
        what: "Error 2",
        why: "R2",
        appDid: "A2",
        userAction: "U2",
      });
      useUIStore.getState().clearErrors();
    });
    expect(useUIStore.getState().structuredErrors).toHaveLength(0);
  });
});

// ──────────────────────────────────────────────
// Guided Flow State Tests (T-056)
// ──────────────────────────────────────────────

describe("useUIStore - Guided Flows", () => {
  it("starts with no active guided flow", () => {
    expect(useUIStore.getState().activeGuidedFlow).toBeNull();
  });

  it("sets an active guided flow", () => {
    act(() => {
      useUIStore.getState().setActiveGuidedFlow("connect-onedrive");
    });
    expect(useUIStore.getState().activeGuidedFlow).toBe("connect-onedrive");
  });

  it("clears active guided flow", () => {
    act(() => {
      useUIStore.getState().setActiveGuidedFlow("connect-onedrive");
      useUIStore.getState().setActiveGuidedFlow(null);
    });
    expect(useUIStore.getState().activeGuidedFlow).toBeNull();
  });
});
