/**
 * Iter-18 tests: path-aware cross-pane refresh helper.
 *
 * Exercises `shouldRefreshPane` across every edge case the dispatch
 * pattern must handle correctly:
 *
 *   - Exact match (affected path IS the viewed directory)
 *   - Direct child (a file inside the viewed directory)
 *   - Deep descendant (a file nested in a subfolder)
 *   - Unrelated sibling (must NOT refresh)
 *   - Ancestor (viewed path is INSIDE affected path — must NOT refresh)
 *   - Empty affectedPaths (unknown = safe refresh, backwards compat)
 *   - Root path edge cases
 *   - Trailing slash normalisation
 *   - Prefix-matching false positive guard (e.g. /home/src vs /home/srcs)
 *
 * Pure unit tests — no React, no Zustand, no IPC. Matches the
 * architectural boundary: the helper is a pure predicate over strings.
 */
import { describe, it, expect } from "vitest";
import {
  shouldRefreshPane,
  parentDirectoriesOf,
  REFRESH_EVENT,
} from "../lib/refresh-affected";

describe("shouldRefreshPane", () => {
  it("returns true when affectedPaths is empty (unknown = safe refresh)", () => {
    expect(shouldRefreshPane("/home/projects", [])).toBe(true);
  });

  it("returns true on exact match", () => {
    expect(
      shouldRefreshPane("/home/projects", ["/home/projects"]),
    ).toBe(true);
  });

  it("returns true when an affected path is a direct child of the viewed dir", () => {
    expect(
      shouldRefreshPane("/home/projects", ["/home/projects/foo.txt"]),
    ).toBe(true);
  });

  it("returns true when an affected path is a deeper descendant", () => {
    expect(
      shouldRefreshPane("/home/projects", [
        "/home/projects/sub/deep/file.md",
      ]),
    ).toBe(true);
  });

  it("returns false for a completely unrelated path", () => {
    expect(
      shouldRefreshPane("/home/projects", ["/var/log/syslog"]),
    ).toBe(false);
  });

  it("returns false when the affected path is an ancestor of the viewed path", () => {
    // Viewing /home/src. Affected path is /home. Our directory's
    // contents didn't change just because a sibling in /home did.
    expect(shouldRefreshPane("/home/src", ["/home"])).toBe(false);
  });

  it("returns false for prefix-matching false positives", () => {
    // /home/srcs starts with /home/src but is NOT a child of it.
    expect(shouldRefreshPane("/home/src", ["/home/srcs/file"])).toBe(
      false,
    );
  });

  it("handles multiple affected paths where only one matches", () => {
    expect(
      shouldRefreshPane("/home/projects", [
        "/var/log/syslog",
        "/home/projects/new.txt",
        "/etc/hosts",
      ]),
    ).toBe(true);
  });

  it("handles multiple affected paths where none match", () => {
    expect(
      shouldRefreshPane("/home/projects", [
        "/var/log/syslog",
        "/etc/hosts",
        "/tmp/file",
      ]),
    ).toBe(false);
  });

  it("normalises trailing slashes on the viewed path", () => {
    expect(
      shouldRefreshPane("/home/projects/", ["/home/projects/foo"]),
    ).toBe(true);
  });

  it("normalises trailing slashes on affected paths", () => {
    expect(
      shouldRefreshPane("/home/projects", ["/home/projects/"]),
    ).toBe(true);
  });

  it("handles the root path as viewed", () => {
    expect(shouldRefreshPane("/", ["/home"])).toBe(true);
    expect(shouldRefreshPane("/", ["/etc/hosts"])).toBe(true);
  });

  it("handles root as an affected path against a non-root view", () => {
    // Viewing /home, affected is "/". The affected path is NOT a
    // child of /home, so no refresh.
    expect(shouldRefreshPane("/home", ["/"])).toBe(false);
  });

  it("handles the cross-pane move case: both source and dest affected", () => {
    // Source pane at /src — expect refresh.
    expect(
      shouldRefreshPane("/src", ["/dst", "/src/a.txt", "/src/b.txt"]),
    ).toBe(true);
    // Dest pane at /dst — expect refresh.
    expect(
      shouldRefreshPane("/dst", ["/dst", "/src/a.txt", "/src/b.txt"]),
    ).toBe(true);
    // Unrelated pane at /tmp — expect NO refresh.
    expect(
      shouldRefreshPane("/tmp", ["/dst", "/src/a.txt", "/src/b.txt"]),
    ).toBe(false);
  });
});

describe("Iter 19 migration scenarios (intent regression)", () => {
  // These tests pin down the INTENT of each iter-19 handler migration
  // so a future refactor that passes the wrong affectedPaths list to
  // dispatchRefresh breaks a test that explicitly documents the
  // expected cross-pane behavior.

  it("handleDelete: any pane viewing the source directory refreshes", () => {
    // dispatchRefresh([currentPath, ...selectedPaths])
    const dispatched = [
      "/home/work",
      "/home/work/a.txt",
      "/home/work/b.txt",
    ];
    expect(shouldRefreshPane("/home/work", dispatched)).toBe(true);
    expect(shouldRefreshPane("/home/other", dispatched)).toBe(false);
  });

  it("handleRename: pane viewing parent dir refreshes via child-path prefix match", () => {
    // dispatchRefresh([filePath, newPath])
    const dispatched = ["/home/work/old.txt", "/home/work/new.txt"];
    expect(shouldRefreshPane("/home/work", dispatched)).toBe(true);
    expect(shouldRefreshPane("/home/elsewhere", dispatched)).toBe(false);
  });

  it("handleNewFolder: pane viewing parent dir refreshes when a child folder is added", () => {
    // dispatchRefresh([currentPath, newPath])
    const dispatched = ["/home/work", "/home/work/Project X"];
    expect(shouldRefreshPane("/home/work", dispatched)).toBe(true);
    // A pane viewing the NEW folder itself (unlikely but safe) also
    // gets a refresh so its contents populate.
    expect(shouldRefreshPane("/home/work/Project X", dispatched)).toBe(
      true,
    );
  });

  it("handleArchiveStale: both source dir AND the archive subfolder refresh", () => {
    // dispatchRefresh([currentPath, destDir])
    const dispatched = ["/home/work", "/home/work/.archive/2026-04"];
    // The source pane sees files disappear.
    expect(shouldRefreshPane("/home/work", dispatched)).toBe(true);
    // A pane viewing the .archive subfolder sees files arrive.
    expect(
      shouldRefreshPane("/home/work/.archive/2026-04", dispatched),
    ).toBe(true);
  });

  it("Smart Send-To: source pane AND dest pane both refresh (iter-18-class fix)", () => {
    // dispatchRefresh([currentPath, destDir])
    const dispatched = ["/src", "/dst"];
    expect(shouldRefreshPane("/src", dispatched)).toBe(true);
    expect(shouldRefreshPane("/dst", dispatched)).toBe(true);
    // Unrelated pane stays put.
    expect(shouldRefreshPane("/tmp", dispatched)).toBe(false);
  });

  it("drag-drop: dest pane AND any pane viewing the source files' parent refresh", () => {
    // dispatchRefresh([currentPath, ...droppedPaths])
    const dispatched = [
      "/dst",
      "/src/file-a.txt",
      "/src/file-b.txt",
    ];
    // Drop destination refreshes.
    expect(shouldRefreshPane("/dst", dispatched)).toBe(true);
    // Source pane (viewing /src) refreshes because /src/file-a.txt
    // is a child of /src, so the prefix match fires.
    expect(shouldRefreshPane("/src", dispatched)).toBe(true);
  });

  it("handleCreateSymlink: any pane viewing either the link's parent OR the original currentPath refreshes", () => {
    // dispatchRefresh([currentPath, linkPath])
    const dispatched = ["/home/work", "/home/other/my.link"];
    // Source pane (maybe currentPath == target's dir) refreshes.
    expect(shouldRefreshPane("/home/work", dispatched)).toBe(true);
    // Pane viewing the link's parent dir refreshes because
    // /home/other/my.link is a child of /home/other.
    expect(shouldRefreshPane("/home/other", dispatched)).toBe(true);
    // Unrelated pane stays put.
    expect(shouldRefreshPane("/tmp", dispatched)).toBe(false);
  });
});

describe("parentDirectoriesOf (iter 25)", () => {
  it("returns an empty array for empty input", () => {
    expect(parentDirectoriesOf([])).toEqual([]);
  });

  it("extracts the single parent of one file", () => {
    expect(parentDirectoriesOf(["/home/work/foo.pdf"])).toEqual([
      "/home/work",
    ]);
  });

  it("extracts multiple distinct parents", () => {
    expect(
      parentDirectoriesOf([
        "/a/one.txt",
        "/b/two.txt",
        "/c/three.txt",
      ]),
    ).toEqual(["/a", "/b", "/c"]);
  });

  it("deduplicates identical parents while preserving first-seen order", () => {
    expect(
      parentDirectoriesOf([
        "/home/work/first.pdf",
        "/home/work/second.pdf",
        "/home/work/third.pdf",
      ]),
    ).toEqual(["/home/work"]);
  });

  it("handles root-level files (parent = '/')", () => {
    expect(parentDirectoriesOf(["/foo.txt", "/bar.txt"])).toEqual(["/"]);
  });

  it("mixes root-level and nested files correctly", () => {
    expect(
      parentDirectoriesOf([
        "/top.txt",
        "/home/nested.txt",
        "/home/other.txt",
      ]),
    ).toEqual(["/", "/home"]);
  });

  it("preserves first-seen order across heterogeneous inputs", () => {
    expect(
      parentDirectoriesOf([
        "/b/file1",
        "/a/file2",
        "/b/file3",
        "/c/file4",
        "/a/file5",
      ]),
    ).toEqual(["/b", "/a", "/c"]);
  });

  it("does not mutate the input array", () => {
    const input = ["/x/a", "/y/b", "/x/c"];
    const before = [...input];
    parentDirectoriesOf(input);
    expect(input).toEqual(before);
  });

  it("simulates the iter-25 batch-rename scenario end-to-end", () => {
    // User selects 5 files in /home/work and runs batch rename.
    // onComplete dispatches parentDirectoriesOf, which should
    // produce exactly one parent ("/home/work"), meaning one
    // refresh event reaches every pane viewing that directory.
    const renamed = [
      "/home/work/report-01.pdf",
      "/home/work/report-02.pdf",
      "/home/work/report-03.pdf",
      "/home/work/report-04.pdf",
      "/home/work/report-05.pdf",
    ];
    const parents = parentDirectoriesOf(renamed);
    expect(parents).toEqual(["/home/work"]);
    // And the helper plays nicely with shouldRefreshPane: any pane
    // viewing /home/work gets the refresh.
    expect(shouldRefreshPane("/home/work", parents)).toBe(true);
    // Unrelated pane does NOT.
    expect(shouldRefreshPane("/home/other", parents)).toBe(false);
  });
});

describe("REFRESH_EVENT constant", () => {
  it("matches the legacy event name for backwards compat", () => {
    expect(REFRESH_EVENT).toBe("ufop:refresh-directory");
  });
});
