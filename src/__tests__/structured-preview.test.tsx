/** CSV/TSV table and JSON tree previews. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { StructuredPreview } from "@/components/structured-preview";
import { detectDelimiter, parseDelimited, parseJsonish, structuredKind } from "@/lib/structured-preview";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("parsing", () => {
  it("handles quotes, escaped quotes, embedded newlines, CRLF and BOM", () => {
    const text = '﻿name,note\r\n"Smith, J","said ""hi""\nthen left"\r\nb,\n';
    expect(parseDelimited(text, ",")).toEqual([["name", "note"], ["Smith, J", 'said "hi"\nthen left'], ["b", ""]]);
  });

  it("keeps a record cut off by the preview cap", () => {
    expect(parseDelimited('a,b\n1,"unterminated', ",")).toEqual([["a", "b"], ["1", "unterminated"]]);
  });

  it("detects the delimiter", () => {
    expect(detectDelimiter("x.tsv", "a,b")).toBe("\t");
    expect(detectDelimiter("x.csv", "a;b;c\n1;2;3")).toBe(";");
    expect(detectDelimiter("x.csv", "single")).toBe(",");
  });

  it("parses JSON and JSON Lines", () => {
    expect(parseJsonish('{"a":1}')).toEqual({ value: { a: 1 } });
    expect(parseJsonish('{"a":1}\n{"a":2}')).toEqual({ value: [{ a: 1 }, { a: 2 }] });
    expect(parseJsonish('{"a":')).toBeNull();
    expect([structuredKind("A.CSV"), structuredKind("x.ndjson"), structuredKind("x.txt")]).toEqual(["csv", "json", null]);
  });
});

describe("StructuredPreview", () => {
  it("shows a CSV as a table and can switch to raw", () => {
    render(<StructuredPreview kind="csv" text={"id,name\n1,Ann\n2,Bo"} fileName="p.csv" path="/p.csv" raw={<pre>RAW</pre>} />);
    const table = screen.getByRole("table", { name: "p.csv table" });
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["#", "id", "name"]);
    expect(table.textContent).toContain("Bo");
    expect(screen.getByText(/2 rows × 2 columns/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Raw" }));
    expect(screen.getByText("RAW")).toBeTruthy();
  });

  it("renders a collapsible JSON tree", () => {
    render(<StructuredPreview kind="json" text={'{"user":{"name":"Ann","tags":["a","b"]},"n":3}'} fileName="d.json" path="/d.json" raw={null} />);
    const tree = screen.getByRole("tree");
    expect(tree.textContent).toContain('"Ann"');
    const tags = screen.getByRole("button", { name: /tags/ });
    expect(tags.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(tags);
    expect(tree.textContent).toContain('"b"');
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("loads the whole file when the capped text is incomplete JSON", async () => {
    mockInvoke.mockResolvedValue('{"big":[1,2,3]}');
    render(<StructuredPreview kind="json" text={'{"big":[1,'} fileName="b.json" path="/b.json" raw={null} />);
    await waitFor(() => expect(screen.getByRole("tree").textContent).toContain("big"));
    expect(mockInvoke).toHaveBeenCalledWith("read_text_file_full", { path: "/b.json" });
  });
});
