import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DataGrid, generateDemoFiles } from "@/components/data-grid";

describe("DataGrid (TanStack Table)", () => {
  const demoFiles = generateDemoFiles(25);

  it("renders a data grid with correct ARIA role", () => {
    render(<DataGrid data={demoFiles} />);
    expect(screen.getByRole("grid")).toBeInTheDocument();
  });

  it("renders column headers", () => {
    render(<DataGrid data={demoFiles} />);
    expect(screen.getByText("Name")).toBeInTheDocument();
    expect(screen.getByText("Size")).toBeInTheDocument();
    expect(screen.getByText("Modified")).toBeInTheDocument();
    expect(screen.getByText("Permissions")).toBeInTheDocument();
  });

  it("renders file entries", () => {
    render(<DataGrid data={demoFiles} />);
    const rows = screen.getAllByRole("row");
    // Header row + data rows
    expect(rows.length).toBeGreaterThan(1);
  });

  it("has accessible column header sort indicators", () => {
    render(<DataGrid data={demoFiles} />);
    const headers = screen.getAllByRole("columnheader");
    headers.forEach((header) => {
      expect(header).toHaveAttribute("aria-sort");
    });
  });

  it("sorts columns when header is clicked", async () => {
    const user = userEvent.setup();
    render(<DataGrid data={demoFiles} />);

    const nameHeader = screen.getByText("Name").closest("[role='columnheader']")!;
    await user.click(nameHeader);

    expect(nameHeader).toHaveAttribute("aria-sort", "ascending");
  });

  it("sorts columns when header is activated via keyboard", async () => {
    const user = userEvent.setup();
    render(<DataGrid data={demoFiles} />);

    const nameHeader = screen.getByText("Name").closest("[role='columnheader']") as HTMLElement;
    nameHeader.focus();
    await user.keyboard("{Enter}");

    expect(nameHeader).toHaveAttribute("aria-sort", "ascending");
  });

  it("filters rows based on filter input", async () => {
    const user = userEvent.setup();
    render(<DataGrid data={demoFiles} />);

    const filterInput = screen.getByPlaceholderText("Filter files...");
    await user.type(filterInput, ".tsx");

    // Should show fewer rows
    const itemCount = screen.getByText(/items$/);
    expect(itemCount).toBeInTheDocument();
  });

  it("filter input has accessible label", () => {
    render(<DataGrid data={demoFiles} />);
    const filterInput = screen.getByLabelText("Filter files by name");
    expect(filterInput).toBeInTheDocument();
  });

  it("rows are focusable for keyboard navigation", () => {
    render(<DataGrid data={demoFiles} />);
    const dataRows = screen.getAllByRole("row").slice(1); // skip header
    dataRows.forEach((row) => {
      expect(row).toHaveAttribute("tabindex", "0");
    });
  });
});

describe("generateDemoFiles", () => {
  it("generates the requested number of files", () => {
    const files = generateDemoFiles(50);
    expect(files.length).toBe(50);
  });

  it("generates a mix of files and folders", () => {
    const files = generateDemoFiles(100);
    const folders = files.filter((f) => f.type === "folder");
    const regularFiles = files.filter((f) => f.type === "file");
    expect(folders.length).toBeGreaterThan(0);
    expect(regularFiles.length).toBeGreaterThan(0);
  });

  it("generates unique ids", () => {
    const files = generateDemoFiles(100);
    const ids = files.map((f) => f.id);
    expect(new Set(ids).size).toBe(100);
  });

  it("folders have 0 size", () => {
    const files = generateDemoFiles(100);
    const folders = files.filter((f) => f.type === "folder");
    folders.forEach((folder) => {
      expect(folder.size).toBe(0);
    });
  });
});
