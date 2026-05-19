/**
 * file-compare — pure line-by-line diff helper.
 *
 * Locks the alignment semantics so a future LCS upgrade can drop in
 * behind this helper without breaking the modal's row renderer.
 *
 * Cases covered:
 *   - identical content (`identical=true`, `changedCount=0`)
 *   - all-different content (every row `changed`)
 *   - partial overlap (some matching, some changed)
 *   - left longer than right (trailing extra rows on left)
 *   - right longer than left (trailing extra rows on right)
 *   - empty files (single empty row each)
 *   - trailing newlines do NOT introduce phantom blank rows
 *   - CRLF line endings handled the same as LF
 *   - line numbers populated correctly on the non-null side
 */
import { describe, it, expect } from "vitest";
import { compareFilesByLine } from "@/lib/file-compare";

describe("compareFilesByLine", () => {
  it("returns identical=true and zero changes for identical content", () => {
    const r = compareFilesByLine("hello\nworld", "hello\nworld");
    expect(r.identical).toBe(true);
    expect(r.changedCount).toBe(0);
    expect(r.rows).toHaveLength(2);
    expect(r.rows.every((row) => !row.changed)).toBe(true);
  });

  it("marks every row changed when nothing matches", () => {
    const r = compareFilesByLine("a\nb\nc", "x\ny\nz");
    expect(r.identical).toBe(false);
    expect(r.changedCount).toBe(3);
    expect(r.rows.every((row) => row.changed)).toBe(true);
  });

  it("flags only the differing rows for partial overlap", () => {
    const r = compareFilesByLine("alpha\nbeta\ngamma", "alpha\nBETA\ngamma");
    expect(r.changedCount).toBe(1);
    expect(r.rows[0].changed).toBe(false);
    expect(r.rows[1].changed).toBe(true);
    expect(r.rows[2].changed).toBe(false);
  });

  it("pads trailing rows on the right when left is longer", () => {
    const r = compareFilesByLine("one\ntwo\nthree", "one");
    expect(r.rows).toHaveLength(3);
    expect(r.rows[0]).toMatchObject({
      left: "one",
      right: "one",
      leftLineNum: 1,
      rightLineNum: 1,
      changed: false,
    });
    expect(r.rows[1]).toMatchObject({
      left: "two",
      right: null,
      leftLineNum: 2,
      rightLineNum: null,
      changed: true,
    });
    expect(r.rows[2]).toMatchObject({
      left: "three",
      right: null,
      leftLineNum: 3,
      rightLineNum: null,
      changed: true,
    });
  });

  it("pads trailing rows on the left when right is longer", () => {
    const r = compareFilesByLine("one", "one\ntwo");
    expect(r.rows).toHaveLength(2);
    expect(r.rows[1]).toMatchObject({
      left: null,
      right: "two",
      leftLineNum: null,
      rightLineNum: 2,
      changed: true,
    });
  });

  it("treats both empty files as one empty matching row", () => {
    const r = compareFilesByLine("", "");
    expect(r.rows).toHaveLength(1);
    expect(r.identical).toBe(true);
    expect(r.rows[0]).toMatchObject({
      left: "",
      right: "",
      leftLineNum: 1,
      rightLineNum: 1,
      changed: false,
    });
  });

  it("does not introduce phantom rows for trailing newlines", () => {
    const r = compareFilesByLine("a\nb\n", "a\nb\n");
    // Both files have 2 lines, not 3 — the trailing newline does
    // not count as a separate empty line. The modal renders what
    // a text editor would render.
    expect(r.rows).toHaveLength(2);
    expect(r.identical).toBe(true);
  });

  it("treats CRLF and LF as equivalent line endings", () => {
    const r = compareFilesByLine("a\nb", "a\r\nb");
    expect(r.identical).toBe(true);
    expect(r.changedCount).toBe(0);
  });

  it("populates 1-based line numbers correctly", () => {
    const r = compareFilesByLine("a\nb\nc", "a\nB\nc\nd");
    expect(r.rows[0].leftLineNum).toBe(1);
    expect(r.rows[0].rightLineNum).toBe(1);
    expect(r.rows[3].leftLineNum).toBeNull();
    expect(r.rows[3].rightLineNum).toBe(4);
  });
});
