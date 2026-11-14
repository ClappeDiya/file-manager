/**
 * Folders in the search index: status, refresh, stop, remove, and adding
 * a folder (names only, or names and text contents).
 */
import { useCallback, useEffect, useState } from "react";
import { FolderPlus, Loader2, RefreshCw, Square, Trash2 } from "lucide-react";
import { errorText } from "@/lib/error-text";
import { pickFolder } from "@/lib/folder-picker";
import { REMOTE_ROOT_PREFIX, searchIndex, type IndexOverview } from "@/stores/search-store";

export function SearchIndexFolders() {
  const [o, setO] = useState<IndexOverview>({ roots: [], jobs: [] });
  const [content, setContent] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async (p: Promise<IndexOverview>) => {
    setError(null);
    try {
      setO(await p);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    void run(searchIndex.overview());
  }, [run]);
  const running = o.jobs.some((j) => j.running);
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => void run(searchIndex.overview()), 1000);
    return () => clearInterval(t);
  }, [running, run]);

  return (
    <div className="space-y-2 text-xs">
      {o.roots.length === 0 && <p className="text-gray-400">No folders indexed yet.</p>}
      <ul className="space-y-1" aria-label="Indexed folders">
        {o.roots.map((r) => {
          const job = o.jobs.find((j) => j.root === r.path);
          const remote = r.path.startsWith(REMOTE_ROOT_PREFIX) ? r.path.slice(REMOTE_ROOT_PREFIX.length).toUpperCase() : null;
          if (remote) {
            return (
              <li key={r.path} className="flex items-center gap-2 p-1.5 rounded border border-gray-200 dark:border-gray-700">
                <div className="flex-1 min-w-0">
                  <div>Folders browsed on {remote}</div>
                  <div className="text-gray-500">{r.files.toLocaleString()} names remembered as you browse</div>
                </div>
                <button type="button" aria-label={`Forget names seen on ${remote}`} className="text-red-500" onClick={() => run(searchIndex.remove(r.path))}>
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </li>
            );
          }
          return (
            <li key={r.path} className="flex items-center gap-2 p-1.5 rounded border border-gray-200 dark:border-gray-700">
              <div className="flex-1 min-w-0">
                <div className="font-mono truncate" title={r.path}>{r.path}</div>
                <div className="text-gray-500">
                  {r.content ? "Names and contents" : "Names only"} · {r.files.toLocaleString()} items
                  {job?.running
                    ? ` · indexing (${job.scanned.toLocaleString()} scanned)…`
                    : r.indexed_at
                      ? ` · updated ${new Date(r.indexed_at).toLocaleString()}`
                      : " · not indexed yet"}
                  {job?.error && <span className="text-red-500"> · {job.error}</span>}
                </div>
              </div>
              {job?.running ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  <button type="button" aria-label={`Stop indexing ${r.path}`} onClick={() => run(searchIndex.cancel(r.path))}>
                    <Square className="h-3.5 w-3.5" />
                  </button>
                </>
              ) : (
                <button type="button" aria-label={`Update index of ${r.path}`} onClick={() => run(searchIndex.refresh(r.path))}>
                  <RefreshCw className="h-3.5 w-3.5" />
                </button>
              )}
              <button type="button" aria-label={`Remove ${r.path} from the index`} className="text-red-500" onClick={() => run(searchIndex.remove(r.path))}>
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </li>
          );
        })}
      </ul>
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="flex items-center gap-1 px-2 py-1 rounded border border-gray-200 dark:border-gray-700"
          onClick={async () => {
            const p = await pickFolder("Folder to index for search");
            if (p) await run(searchIndex.add(p, content));
          }}
        >
          <FolderPlus className="h-3.5 w-3.5" /> Add folder…
        </button>
        <label className="flex items-center gap-1" title="Text files, PDFs, and Word, Excel, PowerPoint, OpenDocument and RTF documents">
          <input type="checkbox" checked={content} onChange={(e) => setContent(e.target.checked)} /> include file contents
        </label>
      </div>
      {error && <p className="text-red-500" role="alert">{error}</p>}
    </div>
  );
}
