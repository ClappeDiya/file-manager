/**
 * Indexed search (names across subfolders, and file contents).
 * Types mirror `search_engine` / `search_commands.rs`.
 */
import { tauriInvoke } from "@/hooks/use-tauri";

export interface IndexRoot {
  path: string;
  content: boolean;
  indexed_at: string | null;
  files: number;
}

export interface IndexStats {
  scanned: number;
  added: number;
  updated: number;
  removed: number;
  cancelled: boolean;
}

export interface IndexJob {
  root: string;
  scanned: number;
  running: boolean;
  last: IndexStats | null;
  error: string | null;
}

export interface IndexOverview {
  roots: IndexRoot[];
  jobs: IndexJob[];
}

export interface SearchHit {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  /** Seconds since the Unix epoch. */
  modified: number;
  /** Matched text with `[` `]` around the terms. */
  snippet: string | null;
  /** Protocol of a file seen while browsing a server; null = local. */
  source: string | null;
}

/** Index roots holding server listings are named `remote:<protocol>`. */
export const REMOTE_ROOT_PREFIX = "remote:";

export interface SearchResponse {
  hits: SearchHit[];
  root: IndexRoot | null;
  indexing: boolean;
}

/** Where the filter bar searches. */
export type SearchScope = "folder" | "names" | "contents";

export const searchIndex = {
  query: (text: string, root: string, content: boolean, limit = 200) =>
    tauriInvoke<SearchResponse>("search_files_query", { text, root, content, limit }),
  overview: () => tauriInvoke<IndexOverview>("search_index_roots"),
  add: (path: string, content: boolean) => tauriInvoke<IndexOverview>("search_index_add", { path, content }),
  refresh: (path: string) => tauriInvoke<IndexOverview>("search_index_refresh", { path }),
  remove: (path: string) => tauriInvoke<IndexOverview>("search_index_remove", { path }),
  cancel: (path: string) => tauriInvoke<IndexOverview>("search_index_cancel", { path }),
};

/** Split a snippet into plain and matched parts (`[term]`). */
export function snippetParts(snippet: string): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  const re = /\[([^\]]*)\]/g;
  let last = 0;
  for (let m = re.exec(snippet); m; m = re.exec(snippet)) {
    if (m.index > last) out.push({ text: snippet.slice(last, m.index), hit: false });
    out.push({ text: m[1], hit: true });
    last = m.index + m[0].length;
  }
  if (last < snippet.length) out.push({ text: snippet.slice(last), hit: false });
  return out;
}
