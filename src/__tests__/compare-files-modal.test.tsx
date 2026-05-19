/**
 * CompareFilesModal — render contract.
 *
 * Locks the contract that the modal:
 *   - renders nothing when `data` is null (the orphan-state guard)
 *   - renders both file names in the header when data is present
 *   - reports the difference count (or "identical")
 *   - dispatches `onClose` on backdrop click, X button click, Esc key
 *   - highlights changed rows visually (data-testid presence check)
 *
 * Why a dedicated test: until this iteration the `_compareData`
 * state was orphaned (set but never rendered). A future refactor
 * that re-orphans the state would silently re-break the feature
 * unless an integration-level smoke test catches it.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { CompareFilesModal, type CompareData } from "@/components/compare-files-modal";

function makeData(overrides: Partial<CompareData> = {}): CompareData {
  return {
    left: "alpha\nbeta\ngamma",
    right: "alpha\nBETA\ngamma",
    leftName: "before.txt",
    rightName: "after.txt",
    ...overrides,
  };
}

describe("CompareFilesModal", () => {
  it("renders nothing when data is null", () => {
    const { container } = render(
      <CompareFilesModal data={null} onClose={() => {}} />,
    );
    expect(container.textContent).toBe("");
  });

  it("renders both file names in the header when data is present", () => {
    render(<CompareFilesModal data={makeData()} onClose={() => {}} />);
    expect(screen.getByText("before.txt")).toBeInTheDocument();
    expect(screen.getByText("after.txt")).toBeInTheDocument();
  });

  it("reports the difference count for non-identical files", () => {
    render(<CompareFilesModal data={makeData()} onClose={() => {}} />);
    expect(screen.getByText(/1 difference/)).toBeInTheDocument();
  });

  it('reports "Files are identical" when content matches', () => {
    render(
      <CompareFilesModal
        data={makeData({ left: "same", right: "same" })}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/Files are identical/)).toBeInTheDocument();
  });

  it("dispatches onClose when the × button is clicked", () => {
    const onClose = vi.fn();
    render(<CompareFilesModal data={makeData()} onClose={onClose} />);
    fireEvent.click(screen.getByTestId("compare-files-close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("dispatches onClose when the backdrop is clicked", () => {
    const onClose = vi.fn();
    render(<CompareFilesModal data={makeData()} onClose={onClose} />);
    fireEvent.click(screen.getByTestId("compare-files-backdrop"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("dispatches onClose on Escape key press", () => {
    const onClose = vi.fn();
    render(<CompareFilesModal data={makeData()} onClose={onClose} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("renders the diff body region (data-testid present)", () => {
    render(<CompareFilesModal data={makeData()} onClose={() => {}} />);
    expect(screen.getByTestId("compare-files-body")).toBeInTheDocument();
  });
});
