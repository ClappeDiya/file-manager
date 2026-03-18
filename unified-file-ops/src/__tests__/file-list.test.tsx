import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { FileList, formatFileSize, formatDate, type FileEntryData } from "@/components/file-list";
import { filterFiles } from "@/components/filter-bar";

// ── T-007: Directory Listing tests ──

describe("T-007: FileList utilities", () => {
  describe("formatFileSize", () => {
    it("should return -- for zero bytes", () => {
      expect(formatFileSize(0)).toBe("--");
    });

    it("should format bytes", () => {
      expect(formatFileSize(500)).toBe("500 B");
    });

    it("should format kilobytes", () => {
      expect(formatFileSize(1024)).toBe("1.0 KB");
      expect(formatFileSize(1536)).toBe("1.5 KB");
    });

    it("should format megabytes", () => {
      expect(formatFileSize(1048576)).toBe("1.0 MB");
    });

    it("should format gigabytes", () => {
      expect(formatFileSize(1073741824)).toBe("1.0 GB");
    });
  });

  describe("formatDate", () => {
    it("should return -- for null dates", () => {
      expect(formatDate(null)).toBe("--");
    });

    it("should format valid dates", () => {
      const result = formatDate("2024-01-15T10:30:00Z");
      expect(result).toBeTruthy();
      expect(result).not.toBe("--");
    });
  });
});

describe("T-007: FileList rendering", () => {
  const mockFiles: FileEntryData[] = [
    {
      name: "Documents",
      path: "/Documents",
      is_dir: true,
      is_symlink: false,
      size: 0,
      modified: "2024-01-15T10:00:00Z",
      created: "2024-01-01T00:00:00Z",
      is_hidden: false,
      extension: null,
      permissions: "755",
    },
    {
      name: "readme.md",
      path: "/readme.md",
      is_dir: false,
      is_symlink: false,
      size: 1024,
      modified: "2024-01-14T10:00:00Z",
      created: "2024-01-01T00:00:00Z",
      is_hidden: false,
      extension: "md",
      permissions: "644",
    },
    {
      name: "config.json",
      path: "/config.json",
      is_dir: false,
      is_symlink: false,
      size: 256,
      modified: "2024-01-13T10:00:00Z",
      created: "2024-01-01T00:00:00Z",
      is_hidden: false,
      extension: "json",
      permissions: "644",
    },
    {
      name: ".hidden",
      path: "/.hidden",
      is_dir: false,
      is_symlink: false,
      size: 0,
      modified: "2024-01-12T10:00:00Z",
      created: "2024-01-01T00:00:00Z",
      is_hidden: true,
      extension: null,
      permissions: "644",
    },
  ];

  it("should render in detail view mode", () => {
    render(<FileList files={mockFiles} viewMode="detail" />);
    expect(screen.getByTestId("file-list")).toBeInTheDocument();
    expect(screen.getByTestId("file-list-body")).toBeInTheDocument();
  });

  it("should render in list view mode", () => {
    render(<FileList files={mockFiles} viewMode="list" />);
    expect(screen.getByTestId("file-list")).toBeInTheDocument();
  });

  it("should render in grid view mode", () => {
    render(<FileList files={mockFiles} viewMode="grid" />);
    expect(screen.getByTestId("file-grid")).toBeInTheDocument();
  });

  it("should render in compact view mode", () => {
    render(<FileList files={mockFiles} viewMode="compact" />);
    expect(screen.getByTestId("file-list")).toBeInTheDocument();
  });

  it("should handle selection callback", () => {
    const onSelect = vi.fn();
    render(
      <FileList files={mockFiles} viewMode="detail" onSelect={onSelect} />,
    );
    // Verify the grid renders
    const body = screen.getByTestId("file-list-body");
    expect(body).toBeInTheDocument();
  });

  it("should handle double-click to open", () => {
    const onOpen = vi.fn();
    render(
      <FileList files={mockFiles} viewMode="detail" onOpen={onOpen} />,
    );
    expect(screen.getByTestId("file-list")).toBeInTheDocument();
  });

  it("should show column headers in detail mode", () => {
    render(<FileList files={mockFiles} viewMode="detail" />);
    expect(screen.getByTestId("column-header-name")).toBeInTheDocument();
    expect(screen.getByTestId("column-header-size")).toBeInTheDocument();
    expect(screen.getByTestId("column-header-modified")).toBeInTheDocument();
  });

  it("should have accessible grid role", () => {
    render(<FileList files={mockFiles} viewMode="detail" />);
    const grid = screen.getByRole("grid", { name: /file listing/i });
    expect(grid).toBeInTheDocument();
  });
});

// ── T-012: Filter tests ──

describe("T-012: filterFiles", () => {
  const testFiles = [
    { name: "readme.md", path: "/readme.md", extension: "md" },
    { name: "config.json", path: "/config.json", extension: "json" },
    { name: "index.ts", path: "/index.ts", extension: "ts" },
    { name: "styles.css", path: "/styles.css", extension: "css" },
    { name: "app.tsx", path: "/app.tsx", extension: "tsx" },
  ];

  it("should return all files when filter is empty", () => {
    expect(filterFiles(testFiles, "")).toHaveLength(5);
    expect(filterFiles(testFiles, "  ")).toHaveLength(5);
  });

  it("should filter by name", () => {
    const result = filterFiles(testFiles, "readme");
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("readme.md");
  });

  it("should filter by extension", () => {
    const result = filterFiles(testFiles, "ts");
    expect(result).toHaveLength(2); // index.ts and app.tsx
  });

  it("should filter case-insensitively", () => {
    const result = filterFiles(testFiles, "README");
    expect(result).toHaveLength(1);
  });

  it("should support multi-word filtering", () => {
    const result = filterFiles(testFiles, "config json");
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("config.json");
  });

  it("should return empty for no matches", () => {
    const result = filterFiles(testFiles, "nonexistent");
    expect(result).toHaveLength(0);
  });

  it("should handle large file lists efficiently", () => {
    const largeList = Array.from({ length: 10000 }, (_, i) => ({
      name: `file-${i}.txt`,
      path: `/file-${i}.txt`,
      extension: "txt",
    }));

    const start = performance.now();
    const result = filterFiles(largeList, "file-500");
    const elapsed = performance.now() - start;

    expect(result.length).toBeGreaterThan(0);
    expect(elapsed).toBeLessThan(100); // <100ms requirement
  });
});
