/**
 * formatEntrySize — pure helper that drives the Size column in the
 * file list, accounting for the optional `folderSizes` cache that
 * the "Calculate Size" context-menu action populates.
 *
 * Locks the contract that:
 *   - files always render their `entry.size`
 *   - folders render `--` until a cached size exists
 *   - folders with a cached size render the formatted byte count
 *   - the cache lookup is by exact path (no path normalisation, no
 *     case folding) so the FilePane's exact key is the truth
 *
 * Why a dedicated test: until this iteration the cached folder size
 * went into a `_folderSizes` state variable that was never read.
 * A future refactor that re-orphans the cache would silently re-
 * break the feature — this test guards the surface contract.
 */
import { describe, it, expect } from "vitest";
import { formatEntrySize } from "@/components/file-list";
import type { FileEntryData } from "@/components/file-list";

function makeFile(overrides: Partial<FileEntryData> = {}): FileEntryData {
  return {
    name: "file.txt",
    path: "/x/file.txt",
    is_dir: false,
    is_symlink: false,
    size: 1024,
    modified: new Date().toISOString(),
    created: new Date().toISOString(),
    is_hidden: false,
    extension: "txt",
    permissions: "644",
    ...overrides,
  };
}

describe("formatEntrySize", () => {
  it("formats a file's byte count regardless of any folder cache", () => {
    expect(formatEntrySize(makeFile({ size: 1024 }))).toBe("1.0 KB");
    // formatFileSize returns "--" for 0 bytes (zero-byte files
    // are formatted the same as "unknown" — pre-existing behaviour
    // we deliberately preserve to avoid changing the file list).
    expect(formatEntrySize(makeFile({ size: 0 }))).toBe("--");
  });

  it("ignores folderSizes for non-directories", () => {
    expect(
      formatEntrySize(
        makeFile({ is_dir: false, path: "/x/file.txt", size: 2048 }),
        { "/x/file.txt": 99999999 },
      ),
    ).toBe("2.0 KB");
  });

  it('shows "--" for a folder with no cached size', () => {
    expect(formatEntrySize(makeFile({ is_dir: true }))).toBe("--");
    expect(formatEntrySize(makeFile({ is_dir: true }), {})).toBe("--");
  });

  it("shows the cached size when present in folderSizes", () => {
    const folder = makeFile({ is_dir: true, path: "/x/Documents", size: 0 });
    // 5,242,880 bytes = 5.0 MB after formatFileSize rounding.
    expect(formatEntrySize(folder, { "/x/Documents": 5242880 })).toBe("5.0 MB");
  });

  it("looks up by exact path (no case folding, no path normalisation)", () => {
    const folder = makeFile({ is_dir: true, path: "/x/Documents", size: 0 });
    // Differently-cased key does NOT hit.
    expect(formatEntrySize(folder, { "/x/documents": 1024 })).toBe("--");
  });

  it("renders an empty folder's cached 0-byte size as '--' (matches formatFileSize semantics)", () => {
    // The shared `formatFileSize` returns "--" for 0 bytes. The
    // helper composes that without special-casing, so an empty
    // folder still renders "--" — visually indistinguishable from
    // a not-yet-computed folder. Documented here so a future
    // change to either function flags the test consciously.
    const folder = makeFile({ is_dir: true, path: "/x/Empty", size: 0 });
    expect(formatEntrySize(folder, { "/x/Empty": 0 })).toBe("--");
  });
});
