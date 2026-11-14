/**
 * Simple-mode Search: one box over every indexed folder, names or
 * contents, plus the list of indexed folders.
 */
import { useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import { errorText } from "@/lib/error-text";
import { SearchHitList } from "@/components/indexed-search-results";
import { SearchIndexFolders } from "@/components/search-index-folders";
import { searchIndex, type SearchHit } from "@/stores/search-store";

export function SimpleSearchView({ onOpen }: { onOpen: (path: string, isDir: boolean) => void }) {
  const [text, setText] = useState("");
  const [content, setContent] = useState(false);
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  useEffect(() => {
    const id = ++seq.current;
    if (text.trim().length < 2) {
      setHits(null);
      return;
    }
    const t = setTimeout(() => {
      searchIndex
        .query(text.trim(), "", content)
        .then((r) => id === seq.current && (setHits(r.hits), setError(null)))
        .catch((e) => id === seq.current && setError(errorText(e)));
    }, 250);
    return () => clearTimeout(t);
  }, [text, content]);

  return (
    <div className="flex flex-col h-full p-4 gap-3 overflow-y-auto">
      <div className="flex items-center gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2">
        <Search className="h-4 w-4 text-[color:var(--color-text-tertiary)]" aria-hidden="true" />
        <input
          autoFocus
          className="flex-1 bg-transparent outline-none text-sm"
          placeholder="Search your indexed folders"
          aria-label="Search files"
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <label className="flex items-center gap-1 text-xs text-[color:var(--color-text-secondary)]">
          <input type="checkbox" checked={content} onChange={(e) => setContent(e.target.checked)} /> inside files
        </label>
      </div>
      {error && <p className="text-xs text-red-500" role="alert">{error}</p>}
      {hits && (
        <div className="rounded-lg border border-[var(--color-border)] text-xs" aria-label="Search results">
          {hits.length === 0 ? <p className="p-3 text-[color:var(--color-text-tertiary)]">No matches.</p> : <SearchHitList hits={hits} onOpen={onOpen} />}
        </div>
      )}
      <section aria-labelledby="indexed-h">
        <h3 id="indexed-h" className="text-xs font-semibold mb-2">Folders searched</h3>
        <SearchIndexFolders />
      </section>
    </div>
  );
}
