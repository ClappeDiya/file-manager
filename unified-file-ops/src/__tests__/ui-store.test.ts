import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "@/stores/ui-store";

describe("UI Store (Zustand)", () => {
  beforeEach(() => {
    // Reset the store between tests
    const { setState } = useUIStore;
    setState({
      theme: "system",
      resolvedTheme: "light",
      sidebarOpen: true,
      sidebarWidth: 256,
      appMode: "simple",
      viewMode: "list",
      statusBarVisible: true,
      previewPanelOpen: false,
      previewPanelWidth: 320,
      onboardingComplete: false,
      onboardingStep: "welcome",
      userStyle: "personal",
      activityFeed: [],
      structuredErrors: [],
      activeGuidedFlow: null,
      toolbarItems: ["sidebar", "view-modes", "ai", "terminal", "theme"],
      toolbarCustomizerOpen: false,
    });
    localStorage.clear();
  });

  describe("Theme", () => {
    it("defaults to system theme", () => {
      expect(useUIStore.getState().theme).toBe("system");
    });

    it("sets light theme", () => {
      useUIStore.getState().setTheme("light");
      expect(useUIStore.getState().theme).toBe("light");
      expect(useUIStore.getState().resolvedTheme).toBe("light");
    });

    it("sets dark theme", () => {
      useUIStore.getState().setTheme("dark");
      expect(useUIStore.getState().theme).toBe("dark");
      expect(useUIStore.getState().resolvedTheme).toBe("dark");
    });

    it("sets high-contrast theme", () => {
      useUIStore.getState().setTheme("high-contrast");
      expect(useUIStore.getState().theme).toBe("high-contrast");
      expect(useUIStore.getState().resolvedTheme).toBe("high-contrast");
    });

    it("applies data-theme attribute to document", () => {
      useUIStore.getState().setTheme("dark");
      expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

      useUIStore.getState().setTheme("light");
      expect(document.documentElement.getAttribute("data-theme")).toBe("light");

      useUIStore.getState().setTheme("high-contrast");
      expect(document.documentElement.getAttribute("data-theme")).toBe(
        "high-contrast",
      );
    });

    it("resolves system theme based on matchMedia", () => {
      // Default mock returns false for dark mode
      useUIStore.getState().setTheme("system");
      expect(useUIStore.getState().resolvedTheme).toBe("light");
    });
  });

  describe("Sidebar", () => {
    it("defaults to open", () => {
      expect(useUIStore.getState().sidebarOpen).toBe(true);
    });

    it("toggles sidebar", () => {
      useUIStore.getState().toggleSidebar();
      expect(useUIStore.getState().sidebarOpen).toBe(false);

      useUIStore.getState().toggleSidebar();
      expect(useUIStore.getState().sidebarOpen).toBe(true);
    });

    it("sets sidebar open state", () => {
      useUIStore.getState().setSidebarOpen(false);
      expect(useUIStore.getState().sidebarOpen).toBe(false);
    });

    it("sets sidebar width within bounds", () => {
      useUIStore.getState().setSidebarWidth(300);
      expect(useUIStore.getState().sidebarWidth).toBe(300);

      // Minimum bound: 180
      useUIStore.getState().setSidebarWidth(100);
      expect(useUIStore.getState().sidebarWidth).toBe(180);

      // Maximum bound: 480
      useUIStore.getState().setSidebarWidth(600);
      expect(useUIStore.getState().sidebarWidth).toBe(480);
    });
  });

  describe("Application Mode", () => {
    it("defaults to simple mode", () => {
      expect(useUIStore.getState().appMode).toBe("simple");
    });

    it("switches to advanced mode", () => {
      useUIStore.getState().setAppMode("advanced");
      expect(useUIStore.getState().appMode).toBe("advanced");
    });

    it("switches back to simple mode", () => {
      useUIStore.getState().setAppMode("advanced");
      useUIStore.getState().setAppMode("simple");
      expect(useUIStore.getState().appMode).toBe("simple");
    });
  });

  describe("View Mode", () => {
    it("defaults to list view", () => {
      expect(useUIStore.getState().viewMode).toBe("list");
    });

    it("switches to grid view", () => {
      useUIStore.getState().setViewMode("grid");
      expect(useUIStore.getState().viewMode).toBe("grid");
    });

    it("switches to details view", () => {
      useUIStore.getState().setViewMode("details");
      expect(useUIStore.getState().viewMode).toBe("details");
    });

    it("switches to columns view", () => {
      useUIStore.getState().setViewMode("columns");
      expect(useUIStore.getState().viewMode).toBe("columns");
    });
  });

  describe("Status Bar", () => {
    it("defaults to visible", () => {
      expect(useUIStore.getState().statusBarVisible).toBe(true);
    });

    it("hides status bar", () => {
      useUIStore.getState().setStatusBarVisible(false);
      expect(useUIStore.getState().statusBarVisible).toBe(false);
    });
  });

  describe("Preview Panel", () => {
    it("defaults to closed", () => {
      expect(useUIStore.getState().previewPanelOpen).toBe(false);
    });

    it("toggles preview panel", () => {
      useUIStore.getState().togglePreviewPanel();
      expect(useUIStore.getState().previewPanelOpen).toBe(true);

      useUIStore.getState().togglePreviewPanel();
      expect(useUIStore.getState().previewPanelOpen).toBe(false);
    });

    it("sets preview panel width within bounds", () => {
      useUIStore.getState().setPreviewPanelWidth(400);
      expect(useUIStore.getState().previewPanelWidth).toBe(400);

      // Minimum: 200
      useUIStore.getState().setPreviewPanelWidth(100);
      expect(useUIStore.getState().previewPanelWidth).toBe(200);

      // Maximum: 600
      useUIStore.getState().setPreviewPanelWidth(800);
      expect(useUIStore.getState().previewPanelWidth).toBe(600);
    });
  });

  // ── Onboarding (T-055) ──

  describe("Onboarding", () => {
    it("defaults to not complete", () => {
      expect(useUIStore.getState().onboardingComplete).toBe(false);
    });

    it("sets onboarding complete", () => {
      useUIStore.getState().setOnboardingComplete(true);
      expect(useUIStore.getState().onboardingComplete).toBe(true);
    });

    it("sets onboarding step", () => {
      useUIStore.getState().setOnboardingStep("choose-style");
      expect(useUIStore.getState().onboardingStep).toBe("choose-style");
    });

    it("sets user style and auto-adjusts app mode", () => {
      useUIStore.getState().setUserStyle("power");
      expect(useUIStore.getState().userStyle).toBe("power");
      expect(useUIStore.getState().appMode).toBe("advanced");

      useUIStore.getState().setUserStyle("personal");
      expect(useUIStore.getState().userStyle).toBe("personal");
      expect(useUIStore.getState().appMode).toBe("simple");
    });

    it("resets onboarding", () => {
      useUIStore.getState().setOnboardingComplete(true);
      useUIStore.getState().setOnboardingStep("enter-workspace");
      useUIStore.getState().resetOnboarding();
      expect(useUIStore.getState().onboardingComplete).toBe(false);
      expect(useUIStore.getState().onboardingStep).toBe("welcome");
    });
  });

  // ── Activity Feed (T-057) ──

  describe("Activity Feed", () => {
    it("starts empty", () => {
      expect(useUIStore.getState().activityFeed).toEqual([]);
    });

    it("adds an activity entry", () => {
      useUIStore.getState().addActivity({
        type: "copy",
        summary: "Copied file.txt",
        paths: ["/file.txt"],
        undoable: true,
      });
      const feed = useUIStore.getState().activityFeed;
      expect(feed).toHaveLength(1);
      expect(feed[0].summary).toBe("Copied file.txt");
      expect(feed[0].id).toBeTruthy();
      expect(feed[0].timestamp).toBeTruthy();
    });

    it("adds entries in reverse chronological order", () => {
      const store = useUIStore.getState();
      store.addActivity({ type: "copy", summary: "First", paths: [], undoable: false });
      store.addActivity({ type: "move", summary: "Second", paths: [], undoable: false });
      const feed = useUIStore.getState().activityFeed;
      expect(feed[0].summary).toBe("Second");
      expect(feed[1].summary).toBe("First");
    });

    it("clears activity feed", () => {
      useUIStore.getState().addActivity({
        type: "copy",
        summary: "test",
        paths: [],
        undoable: false,
      });
      useUIStore.getState().clearActivity();
      expect(useUIStore.getState().activityFeed).toEqual([]);
    });
  });

  // ── Structured Errors (T-057) ──

  describe("Structured Errors", () => {
    it("starts empty", () => {
      expect(useUIStore.getState().structuredErrors).toEqual([]);
    });

    it("adds a structured error", () => {
      useUIStore.getState().addStructuredError({
        what: "File not found",
        why: "File was deleted",
        appDid: "Skipped the operation",
        userAction: "Check the file path",
      });
      const errors = useUIStore.getState().structuredErrors;
      expect(errors).toHaveLength(1);
      expect(errors[0].what).toBe("File not found");
      expect(errors[0].dismissed).toBe(false);
      expect(errors[0].id).toBeTruthy();
    });

    it("dismisses an error", () => {
      useUIStore.getState().addStructuredError({
        what: "Error",
        why: "Reason",
        appDid: "Action",
        userAction: "Fix",
      });
      const errorId = useUIStore.getState().structuredErrors[0].id;
      useUIStore.getState().dismissError(errorId);
      expect(useUIStore.getState().structuredErrors[0].dismissed).toBe(true);
    });

    it("clears all errors", () => {
      useUIStore.getState().addStructuredError({
        what: "E1",
        why: "R1",
        appDid: "A1",
        userAction: "U1",
      });
      useUIStore.getState().addStructuredError({
        what: "E2",
        why: "R2",
        appDid: "A2",
        userAction: "U2",
      });
      useUIStore.getState().clearErrors();
      expect(useUIStore.getState().structuredErrors).toEqual([]);
    });
  });

  // ── Guided Flow (T-056) ──

  describe("Guided Flow", () => {
    it("defaults to null", () => {
      expect(useUIStore.getState().activeGuidedFlow).toBeNull();
    });

    it("sets and clears guided flow", () => {
      useUIStore.getState().setActiveGuidedFlow("upload-flow");
      expect(useUIStore.getState().activeGuidedFlow).toBe("upload-flow");
      useUIStore.getState().setActiveGuidedFlow(null);
      expect(useUIStore.getState().activeGuidedFlow).toBeNull();
    });
  });

  // ── Toolbar Customization ──

  describe("Toolbar Customization", () => {
    it("has default toolbar items", () => {
      const items = useUIStore.getState().toolbarItems;
      expect(items).toContain("sidebar");
      expect(items).toContain("view-modes");
    });

    it("sets custom toolbar items", () => {
      useUIStore.getState().setToolbarItems(["sidebar", "theme"]);
      expect(useUIStore.getState().toolbarItems).toEqual(["sidebar", "theme"]);
    });

    it("toggles toolbar customizer", () => {
      expect(useUIStore.getState().toolbarCustomizerOpen).toBe(false);
      useUIStore.getState().toggleToolbarCustomizer();
      expect(useUIStore.getState().toolbarCustomizerOpen).toBe(true);
      useUIStore.getState().toggleToolbarCustomizer();
      expect(useUIStore.getState().toolbarCustomizerOpen).toBe(false);
    });
  });

  describe("Persistence", () => {
    it("persists theme to localStorage", () => {
      useUIStore.getState().setTheme("dark");
      const stored = JSON.parse(
        localStorage.getItem("ufop-ui-state") || "{}",
      );
      expect(stored.state.theme).toBe("dark");
    });

    it("persists sidebar state to localStorage", () => {
      useUIStore.getState().setSidebarOpen(false);
      const stored = JSON.parse(
        localStorage.getItem("ufop-ui-state") || "{}",
      );
      expect(stored.state.sidebarOpen).toBe(false);
    });

    it("persists app mode to localStorage", () => {
      useUIStore.getState().setAppMode("advanced");
      const stored = JSON.parse(
        localStorage.getItem("ufop-ui-state") || "{}",
      );
      expect(stored.state.appMode).toBe("advanced");
    });
  });
});
