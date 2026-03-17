import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DualPaneLayout } from "@/components/dual-pane-layout";
import { useFileManagerStore } from "@/stores/file-manager-store";

describe("T-006: DualPaneLayout", () => {
  beforeEach(() => {
    useFileManagerStore.setState({
      panes: [
        {
          id: "pane-0",
          tabs: [{ id: "tab-0", path: "/", label: "Root", pinned: false, pathHistory: ["/"], historyIndex: 0 }],
          activeTabId: "tab-0",
          viewMode: "detail",
          sortBy: "name",
          sortAsc: true,
          filterText: "",
          groupBy: null,
        },
        {
          id: "pane-1",
          tabs: [{ id: "tab-1", path: "/", label: "Root", pinned: false, pathHistory: ["/"], historyIndex: 0 }],
          activeTabId: "tab-1",
          viewMode: "detail",
          sortBy: "name",
          sortAsc: true,
          filterText: "",
          groupBy: null,
        },
      ],
      activePaneIndex: 0,
      singlePaneMode: true,
      paneSplitPercent: 50,
      favorites: [],
      recentLocations: [],
      expandedTreePaths: [],
      selectedPaths: {},
      savedSearches: [],
      undoStack: [],
      commandPaletteOpen: false,
    });
  });

  const renderPane = (index: 0 | 1) => (
    <div data-testid={`pane-content-${index}`}>Pane {index}</div>
  );

  it("should render in single-pane mode by default", () => {
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByTestId("dual-pane-layout")).toBeInTheDocument();
    expect(screen.getByTestId("pane-0")).toBeInTheDocument();
    expect(screen.queryByTestId("pane-1")).not.toBeInTheDocument();
    expect(screen.getByTestId("pane-content-0")).toBeInTheDocument();
  });

  it("should render both panes in dual-pane mode", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByTestId("pane-0")).toBeInTheDocument();
    expect(screen.getByTestId("pane-1")).toBeInTheDocument();
    expect(screen.getByTestId("pane-content-0")).toBeInTheDocument();
    expect(screen.getByTestId("pane-content-1")).toBeInTheDocument();
  });

  it("should show divider in dual-pane mode", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByTestId("pane-divider")).toBeInTheDocument();
    expect(screen.getByRole("separator")).toBeInTheDocument();
  });

  it("should not show divider in single-pane mode", () => {
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.queryByTestId("pane-divider")).not.toBeInTheDocument();
  });

  it("should render pane toolbar", () => {
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByTestId("pane-toolbar")).toBeInTheDocument();
  });

  it("should have toggle button to switch modes", () => {
    render(<DualPaneLayout renderPane={renderPane} />);
    const toggle = screen.getByTestId("toggle-pane-mode");
    expect(toggle).toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-label", "Enable dual-pane mode");
  });

  it("should toggle to dual-pane mode when button clicked", () => {
    render(<DualPaneLayout renderPane={renderPane} />);
    const toggle = screen.getByTestId("toggle-pane-mode");
    fireEvent.click(toggle);
    expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
  });

  it("should show pane focus buttons in dual-pane mode", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByTestId("focus-left-pane")).toBeInTheDocument();
    expect(screen.getByTestId("focus-right-pane")).toBeInTheDocument();
  });

  it("should switch active pane when pane focus button clicked", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    fireEvent.click(screen.getByTestId("focus-right-pane"));
    expect(useFileManagerStore.getState().activePaneIndex).toBe(1);
  });

  it("should have accessible separator with ARIA attributes", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    const separator = screen.getByRole("separator");
    expect(separator).toHaveAttribute("aria-orientation", "vertical");
    expect(separator).toHaveAttribute("aria-valuenow", "50");
    expect(separator).toHaveAttribute("aria-valuemin", "20");
    expect(separator).toHaveAttribute("aria-valuemax", "80");
  });

  it("should handle keyboard resize on divider", () => {
    useFileManagerStore.setState({ singlePaneMode: false, paneSplitPercent: 50 });
    render(<DualPaneLayout renderPane={renderPane} />);
    const divider = screen.getByTestId("pane-divider");

    fireEvent.keyDown(divider, { key: "ArrowLeft" });
    expect(useFileManagerStore.getState().paneSplitPercent).toBe(48);

    fireEvent.keyDown(divider, { key: "ArrowRight" });
    expect(useFileManagerStore.getState().paneSplitPercent).toBe(50);
  });

  it("should have region roles on panes", () => {
    useFileManagerStore.setState({ singlePaneMode: false });
    render(<DualPaneLayout renderPane={renderPane} />);
    expect(screen.getByRole("region", { name: /left file pane/i })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: /right file pane/i })).toBeInTheDocument();
  });
});
