/**
 * Parsing for the table (CSV/TSV) and tree (JSON) previews.
 *
 * Preview text is capped by the backend (first 500 lines), so the CSV
 * parser tolerates a cut-off final record and the JSON preview loads the
 * whole file (up to the editor's 1 MB limit) when the capped text doesn't
 * parse.
 */

export type StructuredKind = "csv" | "json" | null;

export function structuredKind(fileName: string): StructuredKind {
  const ext = fileName.split(".").pop()?.toLowerCase() ?? "";
  if (ext === "csv" || ext === "tsv") return "csv";
  if (ext === "json" || ext === "geojson" || ext === "jsonl" || ext === "ndjson") return "json";
  return null;
}

/** Delimiter by extension, else the most frequent of , ; \t | on line 1. */
export function detectDelimiter(fileName: string, text: string): string {
  if (fileName.toLowerCase().endsWith(".tsv")) return "\t";
  const first = text.split("\n", 1)[0] ?? "";
  const counts = [",", ";", "\t", "|"].map((d) => [d, first.split(d).length - 1] as const);
  const best = counts.reduce((a, b) => (b[1] > a[1] ? b : a));
  return best[1] > 0 ? best[0] : ",";
}

/** RFC 4180 records: quoted fields may hold delimiters, "" and newlines. */
export function parseDelimited(text: string, delimiter: string, maxRows = Infinity): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let i = 0;
  const src = text.replace(/^﻿/, "");
  const endRow = () => {
    row.push(field);
    field = "";
    if (!(row.length === 1 && row[0] === "")) rows.push(row);
    row = [];
  };
  while (i < src.length && rows.length < maxRows) {
    const c = src[i];
    if (quoted) {
      if (c === '"' && src[i + 1] === '"') {
        field += '"';
        i += 2;
        continue;
      }
      if (c === '"') quoted = false;
      else field += c;
      i++;
      continue;
    }
    if (c === '"' && field === "") quoted = true;
    else if (c === delimiter) {
      row.push(field);
      field = "";
    } else if (c === "\n") endRow();
    else if (c !== "\r") field += c;
    i++;
  }
  if ((field !== "" || row.length > 0) && rows.length < maxRows) endRow();
  return rows;
}

/** JSON or JSON Lines; `null` when the text isn't (complete) JSON. */
export function parseJsonish(text: string): { value: unknown } | null {
  try {
    return { value: JSON.parse(text) };
  } catch {
    const lines = text.split("\n").filter((l) => l.trim());
    if (lines.length < 2) return null;
    try {
      return { value: lines.map((l) => JSON.parse(l)) };
    } catch {
      return null;
    }
  }
}
