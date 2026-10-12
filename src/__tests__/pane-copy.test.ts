import { describe, it, expect, vi } from "vitest";
import { planPaneCopy, resolveCopyDestName, awaitPaneCopyJobs } from "@/lib/pane-copy";

const entry = (is_dir: boolean, size = 0) => ({ is_dir, size });

describe("resolveCopyDestName", () => {
  it("returns the name unchanged when free", () => {
    expect(resolveCopyDestName("a.txt", new Set())).toBe("a.txt");
  });
  it("matches the Rust keep-both convention on collision", () => {
    const taken = new Set(["a.txt"]);
    expect(resolveCopyDestName("a.txt", taken)).toBe("a (copy).txt");
    taken.add("a (copy).txt");
    expect(resolveCopyDestName("a.txt", taken)).toBe("a (copy 2).txt");
  });
  it("only splits at the last dot (Rust file_stem/extension semantics)", () => {
    expect(resolveCopyDestName("a.tar.gz", new Set(["a.tar.gz"]))).toBe("a.tar (copy).gz");
  });
  it("treats a leading dot as part of the stem", () => {
    expect(resolveCopyDestName(".env", new Set([".env"]))).toBe(".env (copy)");
  });
  it("handles extensionless names", () => {
    expect(resolveCopyDestName("README", new Set(["README"]))).toBe("README (copy)");
  });
});

describe("planPaneCopy", () => {
  it("routes files to verbatim-dest engine jobs and dirs to copy_files", () => {
    const byPath = new Map([
      ["/src/a.bin", entry(false, 100)],
      ["/src/sub", entry(true)],
    ]);
    const plan = planPaneCopy(["/src/a.bin", "/src/sub"], "/dst", byPath, new Set());
    expect(plan.dirPaths).toEqual(["/src/sub"]);
    expect(plan.fileJobs).toEqual([
      { source: "/src/a.bin", dest: "/dst/a.bin", totalBytes: 100 },
    ]);
  });

  it("renames colliding file jobs the way copy_files would", () => {
    const byPath = new Map([["/src/a.txt", entry(false, 5)]]);
    const plan = planPaneCopy(["/src/a.txt"], "/dst", byPath, new Set(["a.txt"]));
    expect(plan.fileJobs[0].dest).toBe("/dst/a (copy).txt");
  });

  it("gives two same-named sources in one batch distinct destinations", () => {
    const byPath = new Map([
      ["/x/a.txt", entry(false, 1)],
      ["/y/a.txt", entry(false, 2)],
    ]);
    const plan = planPaneCopy(["/x/a.txt", "/y/a.txt"], "/dst", byPath, new Set());
    expect(plan.fileJobs.map((j) => j.dest)).toEqual(["/dst/a.txt", "/dst/a (copy).txt"]);
  });

  it("treats paths missing from the listing as files with zero size", () => {
    const plan = planPaneCopy(["/src/ghost.bin"], "/dst", new Map(), new Set());
    expect(plan.fileJobs).toEqual([
      { source: "/src/ghost.bin", dest: "/dst/ghost.bin", totalBytes: 0 },
    ]);
    expect(plan.dirPaths).toEqual([]);
  });

  it("handles a filesystem-root destination", () => {
    const byPath = new Map([["/src/a.txt", entry(false, 1)]]);
    const plan = planPaneCopy(["/src/a.txt"], "/", byPath, new Set());
    expect(plan.fileJobs[0].dest).toBe("/a.txt");
  });
});

describe("awaitPaneCopyJobs", () => {
  const fast = { intervalMs: 1, timeoutMs: 2000 };

  it("resolves when all jobs complete and fires onSettle per job", async () => {
    const onSettle = vi.fn();
    const listJobs = vi
      .fn()
      .mockResolvedValueOnce([
        { id: "a", status: "active" },
        { id: "b", status: "queued" },
      ])
      .mockResolvedValueOnce([
        { id: "a", status: "completed" },
        { id: "b", status: "active" },
      ])
      .mockResolvedValue([
        { id: "a", status: "completed" },
        { id: "b", status: "completed" },
      ]);
    const result = await awaitPaneCopyJobs(["a", "b"], listJobs, { ...fast, onSettle });
    expect(result).toEqual({ completed: 2, failed: 0 });
    expect(onSettle).toHaveBeenCalledTimes(2);
  });

  it("counts failed and cancelled jobs as failed", async () => {
    const listJobs = vi.fn().mockResolvedValue([
      { id: "a", status: "failed" },
      { id: "b", status: "cancelled" },
    ]);
    const result = await awaitPaneCopyJobs(["a", "b"], listJobs, fast);
    expect(result).toEqual({ completed: 0, failed: 2 });
  });

  it("keeps polling while a job sits in conflict", async () => {
    const listJobs = vi
      .fn()
      .mockResolvedValueOnce([
        { id: "a", status: "completed" },
        { id: "b", status: "conflict" },
      ])
      .mockResolvedValue([
        { id: "a", status: "completed" },
        { id: "b", status: "completed" },
      ]);
    const result = await awaitPaneCopyJobs(["a", "b"], listJobs, fast);
    expect(listJobs.mock.calls.length).toBeGreaterThan(1);
    expect(result.completed).toBe(2);
  });

  it("returns partial results on timeout", async () => {
    const listJobs = vi.fn().mockResolvedValue([
      { id: "a", status: "active" },
    ]);
    const result = await awaitPaneCopyJobs(["a"], listJobs, {
      intervalMs: 1,
      timeoutMs: 5,
    });
    expect(result).toEqual({ completed: 0, failed: 0 });
  });
});
