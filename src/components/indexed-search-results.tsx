/**
 * Results of an indexed search under the pane's folder: names across all
 * subfolders, or file contents with the matching text. Offers to index
 * the folder when it isn't yet, and shows progress while indexing.
 */
import { useEffect, useRef, useState } from "react";
import { FileText, Folder, Loader2 } from "lucide-react";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { searchIndex, snippetParts, type IndexJob, type SearchHit, type SearchResponse } from "@/stores/search-store";

const POLL_MS = 1000;

/** Hits with folder (relative to `root` when given) and matched text. */
export function SearchHitList({ hits, root, onOpen }: { hits: SearchHit[]; root?: string; onOpen: (path: string, isDir: boolean) => void }) {
  const base = root?.replace(/[\\/]+$/, "");
  const folderOf = (h: SearchHit) => {
    const dir = h.path.slice(0, Math.max(0, h.path.length - h.name.length - 1));
    return base && dir.startsWith(base) ? dir.slice(base.length + 1) || "." : dir;
  };
  return (
    <ul>
      {hits.map((h) => (
        <li key={h.path}>
          <button
            type="button"
            // Server files open from the connection they were seen on.
            disabled={!!h.source}
            onClick={() => onOpen(h.path, h.is_dir)}
            className="w-full text-left px-3 py-1 hover:bg-[var(--color-hover-bg)] disabled:hover:bg-transparent focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
            title={h.source ? `${h.path} — seen on ${h.source.toUpperCase()}; open it from that connection` : h.path}
          >
            <span className="flex items-center gap-1.5">
              {h.is_dir ? <Folder className="h-3.5 w-3.5 shrink-0" /> : <FileText className="h-3.5 w-3.5 shrink-0" />}
              <span className="font-medium truncate">{h.name}</span>
              {h.source && <span className="shrink-0 rounded px-1 text-[10px] bg-blue-500/15 text-blue-600 dark:text-blue-300">{h.source.toUpperCase()}</span>}
              <span className="text-[color:var(--color-text-tertiary)] truncate">{folderOf(h)}</span>
              {!h.is_dir && <span className="ml-auto text-[color:var(--color-text-tertiary)] shrink-0">{formatBytes(h.size)}</span>}
            </span>
            {h.snippet && (
              <span className="block pl-5 text-[color:var(--color-text-secondary)] truncate">
                {snippetParts(h.snippet).map((p, i) => (p.hit ? <mark key={i} className="bg-amber-300/40 text-inherit rounded-sm">{p.text}</mark> : <span key={i}>{p.text}</span>))}
              </span>
            )}
          </button>
        </li>
      ))}
    </ul>
  );
}

export function IndexedSearchResults({
  text,
  root,
  content,
  onOpen,
}: {
  text: string;
  root: string;
  content: boolean;
  /** Reveal a hit: navigate to it (folder) or to its folder and select it. */
  onOpen: (path: string, isDir: boolean) => void;
}) {
  const [res, setRes] = useState<SearchResponse | null>(null);
  const [job, setJob] = useState<IndexJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [indexContent, setIndexContent] = useState(true);
  const [tick, setTick] = useState(0);
  const seq = useRef(0);

  useEffect(() => {
    const id = ++seq.current;
    setError(null);
    const t = setTimeout(() => {
      searchIndex
        .query(text, root, content)
        .then((r) => id === seq.current && setRes(r))
        .catch((e) => id === seq.current && setError(errorText(e)));
    }, 250);
    return () => clearTimeout(t);
  }, [text, root, content, tick]);

  // While the covering folder is being indexed, poll progress and re-run
  // the query when it finishes.
  const indexedRoot = res?.root?.path ?? null;
  const indexing = !!res?.indexing || !!job?.running;
  useEffect(() => {
    if (!indexing || !indexedRoot) return;
    const t = setInterval(async () => {
      const o = await searchIndex.overview().catch(() => null);
      const j = o?.jobs.find((x) => x.root === indexedRoot) ?? null;
      setJob(j);
      if (j && !j.running) setTick((n) => n + 1);
    }, POLL_MS);
    return () => clearInterval(t);
  }, [indexing, indexedRoot]);

  const startIndex = async () => {
    try {
      const o = await searchIndex.add(root, indexContent);
      setJob(o.jobs.find((j) => j.root === root.replace(/[\\/]+$/, "")) ?? null);
      setTick((n) => n + 1);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const box = "border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[length:var(--font-size-xs)]";
  if (error) return <div className={`${box} px-3 py-2 text-red-500`} role="alert">{error}</div>;
  if (!res) return <div className={`${box} px-3 py-2 text-[color:var(--color-text-tertiary)]`}>Searching…</div>;

  if (!res.root || (content && !res.root.content)) {
    return (
      <div className={`${box} px-3 py-2 flex items-center gap-2 flex-wrap`} role="status">
        <span>{res.root ? "This folder's index has names only." : "This folder isn't indexed yet."}</span>
        <label className="flex items-center gap-1">
          <input type="checkbox" checked={indexContent} onChange={(e) => setIndexContent(e.target.checked)} /> include file contents
        </label>
        <button type="button" onClick={startIndex} className="px-2 py-0.5 rounded bg-[var(--color-primary)] text-white">
          Index this folder
        </button>
      </div>
    );
  }

  return (
    <div className={`${box} max-h-[45vh] overflow-y-auto`} aria-label="Search results">
      <div className="flex items-center gap-2 px-3 py-1 text-[color:var(--color-text-tertiary)]">
        <span>
          {res.hits.length}
          {res.hits.length >= 200 ? "+" : ""} result{res.hits.length === 1 ? "" : "s"} {content ? "in file contents" : "in all subfolders"}
        </span>
        {indexing && (
          <span className="flex items-center gap-1" role="status">
            <Loader2 className="h-3 w-3 animate-spin" /> Updating index{job ? ` (${job.scanned.toLocaleString()} items)` : ""}…
          </span>
        )}
      </div>
      <SearchHitList hits={res.hits} root={root} onOpen={onOpen} />
      {res.hits.length === 0 && <p className="px-3 py-2 text-[color:var(--color-text-tertiary)]">No matches{indexing ? " yet" : ""}.</p>}
    </div>
  );
}
