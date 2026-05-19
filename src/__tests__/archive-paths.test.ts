import { describe, it, expect } from "vitest";
import {
  archiveExtractDest,
  stripArchiveExtension,
} from "@/lib/archive-paths";

describe("stripArchiveExtension", () => {
  it("strips simple .zip", () => {
    expect(stripArchiveExtension("foo.zip")).toBe("foo");
  });

  it("strips simple .7z", () => {
    expect(stripArchiveExtension("foo.7z")).toBe("foo");
  });

  it("strips simple .tar", () => {
    expect(stripArchiveExtension("foo.tar")).toBe("foo");
  });

  it("strips compound .tar.gz (not just .gz)", () => {
    expect(stripArchiveExtension("foo.tar.gz")).toBe("foo");
  });

  it("strips compound .tar.bz2", () => {
    expect(stripArchiveExtension("foo.tar.bz2")).toBe("foo");
  });

  it("strips compound .tar.xz", () => {
    expect(stripArchiveExtension("foo.tar.xz")).toBe("foo");
  });

  it("strips .tgz (shorthand compound)", () => {
    expect(stripArchiveExtension("foo.tgz")).toBe("foo");
  });

  it("matches extensions case-insensitively", () => {
    expect(stripArchiveExtension("FOO.ZIP")).toBe("FOO");
    expect(stripArchiveExtension("Foo.Tar.Gz")).toBe("Foo");
  });

  it("preserves the input when no extension matches", () => {
    expect(stripArchiveExtension("README")).toBe("README");
    expect(stripArchiveExtension("foo.txt")).toBe("foo.txt");
  });

  it("strips only the trailing extension when both compound and single fit", () => {
    // `.tar.gz` should win over `.gz` — verifies the compound list is checked first.
    expect(stripArchiveExtension("backup.tar.gz")).toBe("backup");
  });
});

describe("archiveExtractDest", () => {
  it("returns sibling subfolder of the archive", () => {
    expect(archiveExtractDest("/path/to/foo.zip")).toBe("/path/to/foo");
  });

  it("handles compound extensions", () => {
    expect(archiveExtractDest("/path/to/foo.tar.gz")).toBe("/path/to/foo");
  });

  it("handles archives at the filesystem root", () => {
    expect(archiveExtractDest("/foo.zip")).toBe("/foo");
  });

  it("handles archives with deeply nested parents", () => {
    expect(archiveExtractDest("/a/b/c/d/e/f/foo.7z")).toBe("/a/b/c/d/e/f/foo");
  });

  it("handles relative paths", () => {
    expect(archiveExtractDest("foo.zip")).toBe("foo");
  });

  it("falls back gracefully when extension is unknown", () => {
    expect(archiveExtractDest("/path/to/foo.bin")).toBe("/path/to/foo.bin");
  });
});
