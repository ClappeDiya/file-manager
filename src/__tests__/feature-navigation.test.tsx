import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { AllFeatures } from "@/components/simple-mode/all-features";
import { SimpleModeWrapper } from "@/components/simple-mode/simple-mode-wrapper";
import { PLATFORM_FEATURES } from "@/components/simple-mode/feature-catalog";
import { useUIStore } from "@/stores/ui-store";
import { useFeatureNavigation } from "@/stores/feature-navigation-store";
import { getDefaultCommands } from "@/components/command-palette-helpers";

beforeEach(() => {
  useUIStore.setState({ appMode: "simple", sidebarOpen: true });
  useFeatureNavigation.setState({ pending: null });
});
describe("discoverable feature navigation", () => {
  it("previews, applies, persists and cancels sidebar changes without losing All features", () => {
    const view = render(
      <SimpleModeWrapper fileManager={<div>Files workspace</div>} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Customize sidebar" }));
    expect(
      screen.getByRole("button", { name: "Apply sidebar layout" }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("radio", { name: /Advanced/ }));
    expect(useUIStore.getState().appMode).toBe("simple");
    expect(
      within(screen.getByRole("navigation")).queryByRole("button", {
        name: "Automation & AI",
      }),
    ).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Apply sidebar layout" }),
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Advanced sidebar applied. Saved on this device.",
    );
    expect(
      within(screen.getByRole("navigation")).getByRole("button", {
        name: "Automation & AI",
      }),
    ).toBeVisible();
    expect(
      JSON.parse(localStorage.getItem("ufop-ui-state") ?? "{}").state.appMode,
    ).toBe("advanced");
    fireEvent.click(screen.getByRole("radio", { name: /Basic/ }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("radio", { name: /Advanced/ })).toBeChecked();
    view.unmount();
    render(<SimpleModeWrapper fileManager={<div>Files workspace</div>} />);
    expect(
      within(screen.getByRole("navigation")).getByRole("button", {
        name: "Automation & AI",
      }),
    ).toBeVisible();
    expect(
      within(screen.getByRole("navigation")).getByRole("button", {
        name: "All features",
      }),
    ).toBeVisible();
  });
  it("routes catalog sections and queues a tool once across remount", () => {
    render(<SimpleModeWrapper fileManager={<div>Files workspace</div>} />);
    const nav = screen.getByRole("navigation");
    fireEvent.click(within(nav).getByRole("button", { name: "All features" }));
    fireEvent.change(screen.getByLabelText("Find a feature"), {
      target: { value: "rclone setup" },
    });
    fireEvent.click(screen.getByRole("button", { name: /rclone setup/ }));
    expect(screen.getByText("Files workspace")).toBeVisible();
    const request = useFeatureNavigation.getState().pending!;
    expect(request.target).toEqual({ settings: "rclone" });
    expect(useFeatureNavigation.getState().consume(request.token)).toBe(true);
    expect(useFeatureNavigation.getState().consume(request.token)).toBe(false);
    fireEvent.click(within(nav).getByRole("button", { name: "All features" }));
    fireEvent.change(screen.getByLabelText("Find a feature"), {
      target: { value: "Favorites & recent folders" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /Favorites & recent folders/ }),
    );
    expect(screen.getByRole("heading", { name: "Favorites" })).toBeVisible();
  });
  it("searches every group and exposes clear selection requirements", () => {
    const open = vi.fn();
    render(<AllFeatures onOpen={open} />);
    for (const group of new Set(PLATFORM_FEATURES.map((f) => f.group)))
      fireEvent.click(screen.getByText(group, { selector: "summary" }));
    for (const feature of PLATFORM_FEATURES)
      expect(
        screen.getByText(feature.label, { selector: "span" }),
      ).toBeVisible();
    fireEvent.change(screen.getByLabelText("Find a feature"), {
      target: { value: "Batch rename" },
    });
    expect(screen.getByRole("status")).toHaveTextContent("1 features");
    expect(screen.getByText("Select a file in Files first")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /Batch rename/ }));
    expect(open).toHaveBeenCalledWith({ command: "batch-rename" });
    fireEvent.change(screen.getByLabelText("Find a feature"), {
      target: { value: "no-such-tool" },
    });
    expect(screen.getByText(/No matching features/)).toBeVisible();
  });
  it("resolves every catalog command to a real action, including schedules in Basic", () => {
    const callbacks = {
      onOpenPreview: vi.fn(),
      onBatchRename: vi.fn(),
      onComparePanes: vi.fn(),
      onOpenTransferHistory: vi.fn(),
      onOpenMigration: vi.fn(),
      onOpenIntegrityTools: vi.fn(),
      onOpenRcloneRemotes: vi.fn(),
      onOpenNearby: vi.fn(),
      onOpenMounts: vi.fn(),
      onToggleAutomations: vi.fn(),
      onCreateSmartSpace: vi.fn(),
      onOpenSchedules: vi.fn(),
      onOpenVault: vi.fn(),
      onOpenLogViewer: vi.fn(),
      onSendFeedback: vi.fn(),
      onSaveWorkspace: vi.fn(),
      onCustomizeToolbar: vi.fn(),
      onOpenGuidedFlow: vi.fn(),
    };
    const commands = getDefaultCommands(callbacks);
    const ids = new Set<string>();
    for (const feature of PLATFORM_FEATURES) {
      expect(ids.has(feature.id)).toBe(false);
      ids.add(feature.id);
      if (!("command" in feature.target)) continue;
      const id = feature.target.command;
      const command = commands.find((c) => c.id === id);
      expect(
        command,
        `${feature.label} has no registered command`,
      ).toBeDefined();
      const before = Object.values(callbacks).reduce(
        (n, fn) => n + fn.mock.calls.length,
        0,
      );
      command!.action();
      expect(
        Object.values(callbacks).reduce((n, fn) => n + fn.mock.calls.length, 0),
        `${id} did not reach a handler`,
      ).toBe(before + 1);
    }
  });
});

it("opens every catalog settings link at a real expanded section", async () => {
  const { default: SettingsPanel } =
    await import("@/components/settings-panel");
  const targets = PLATFORM_FEATURES.flatMap((f) =>
    "settings" in f.target ? [f.target.settings] : [],
  );
  const view = render(<SettingsPanel />);
  for (const section of targets) {
    view.rerender(<SettingsPanel focusSection={section} />);
    expect(
      within(screen.getByTestId(`settings-section-${section}`)).getByRole(
        "button",
        { expanded: true },
      ),
    ).toBeVisible();
  }
});
