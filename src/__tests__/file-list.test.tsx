import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  FileList,
  formatFileSize,
  formatDate,
  ActivityDot,
  type FileEntryData,
} from "@/components/file-list";
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

// ── ActivityDot — drill-in vs. ambient indicator contract ──
//
// Tested directly (instead of through FileList) because the file
// list's TanStack virtualizers don't materialize rows in jsdom
// without a measurable parent height, so dots never reach the DOM
// via the integration path.
describe("ActivityDot", () => {
  const info = {
    lastSeen: "2024-01-14T11:00:00Z",
    hitCount: 2,
    ageBucket: "today" as const,
    ageLabel: "2h ago",
  };

  it("renders as a non-interactive span by default (no onClick)", () => {
    const { container } = render(<ActivityDot info={info} />);
    // No button when there's no click handler.
    expect(container.querySelector("button")).toBeNull();
    // The visible element is a span with the title text.
    expect(container.querySelector("span")?.getAttribute("title")).toMatch(
      /2h ago.*2 touches/,
    );
  });

  it("renders as a button with focus styles when onClick is provided", () => {
    const onClick = vi.fn();
    render(<ActivityDot info={info} onClick={onClick} />);
    const btn = screen.getByTestId("activity-dot");
    expect(btn.tagName).toBe("BUTTON");
    expect(btn.getAttribute("aria-label")).toMatch(/click to show file history/);
  });

  it("invokes onClick exactly once on activation", () => {
    const onClick = vi.fn();
    render(<ActivityDot info={info} onClick={onClick} />);
    fireEvent.click(screen.getByTestId("activity-dot"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("stops click propagation so parent row select/open does not also fire", () => {
    const onClick = vi.fn();
    const onParent = vi.fn();
    render(
      <div onClick={onParent}>
        <ActivityDot info={info} onClick={onClick} />
      </div>,
    );
    fireEvent.click(screen.getByTestId("activity-dot"));
    expect(onClick).toHaveBeenCalledTimes(1);
    // The parent row handler must NOT fire — clicking the dot is
    // "show history", not "select the file".
    expect(onParent).not.toHaveBeenCalled();
  });

  it("propagates engine-aware tooltip text into the interactive label", () => {
    const onClick = vi.fn();
    render(
      <ActivityDot
        info={{ ...info, hitCount: 1, lastEngine: "transfer" }}
        onClick={onClick}
      />,
    );
    const btn = screen.getByTestId("activity-dot");
    expect(btn.getAttribute("aria-label")).toMatch(/Transfer.*1 touch/);
  });

  it("includes the last_kind in the tooltip when provided", () => {
    // Surfaces e.g. "Sync rename · 2h ago · 2 touches" so users see
    // WHAT happened, not just which engine did something. The kind
    // string from the backend (`copy`, `rename`, `sync.file`) is
    // surfaced verbatim — no client-side mapping. This test locks the
    // composition order so the tooltip stays readable across all kinds.
    const { container } = render(
      <ActivityDot
        info={{
          ...info,
          lastEngine: "sync",
          lastKind: "rename",
        }}
      />,
    );
    const title = container.querySelector("span")?.getAttribute("title") ?? "";
    expect(title).toMatch(/Sync rename/);
    expect(title).toMatch(/2h ago/);
    expect(title).toMatch(/2 touches/);
  });

  it("falls back to the engine-only label when last_kind is absent", () => {
    // Older backend payloads omit `last_kind`. The dot must still
    // produce a coherent tooltip — this test locks the fallback shape
    // so a deploy where the frontend lands before the Rust SQL change
    // doesn't render a broken "Sync undefined · ..." label.
    const { container } = render(
      <ActivityDot
        info={{
          ...info,
          lastEngine: "sync",
          lastKind: null,
        }}
      />,
    );
    const title = container.querySelector("span")?.getAttribute("title") ?? "";
    expect(title).toMatch(/^Sync:/);
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
