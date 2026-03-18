import { useCallback, useRef, useEffect, useState } from "react";
import { cn } from "@ufop/ui-components";
import { Button, Badge } from "@ufop/ui-components";
import { Search, X, Filter, Save, ChevronDown } from "lucide-react";
import { useFileManagerStore, type SavedSearch } from "@/stores/file-manager-store";

// ──────────────────────────────────────────────
// FilterBar - real-time filter-as-you-type
// ──────────────────────────────────────────────

interface FilterBarProps {
  paneIndex: 0 | 1;
  totalCount: number;
  filteredCount: number;
  onContentSearch?: (query: string, path: string) => void;
  className?: string;
}

export function FilterBar({
  paneIndex,
  totalCount,
  filteredCount,
  onContentSearch,
  className,
}: FilterBarProps) {
  const filterText = useFileManagerStore((s) => s.panes[paneIndex].filterText);
  const setFilterText = useFileManagerStore((s) => s.setFilterText);
  const inputRef = useRef<HTMLInputElement>(null);
  const [searchContents, setSearchContents] = useState(false);
  const contentSearchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Focus filter on Cmd+F
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // Debounced content search
  useEffect(() => {
    if (searchContents && filterText.length >= 3 && onContentSearch) {
      if (contentSearchTimerRef.current) {
        clearTimeout(contentSearchTimerRef.current);
      }
      contentSearchTimerRef.current = setTimeout(() => {
        const store = useFileManagerStore.getState();
        const activePath = store.getActivePath(paneIndex);
        onContentSearch(filterText, activePath);
      }, 500);
    }
    return () => {
      if (contentSearchTimerRef.current) {
        clearTimeout(contentSearchTimerRef.current);
      }
    };
  }, [searchContents, filterText, paneIndex, onContentSearch]);

  const handleClear = useCallback(() => {
    setFilterText(paneIndex, "");
    inputRef.current?.focus();
  }, [paneIndex, setFilterText]);

  return (
    <div
      className={cn(
        "flex items-center gap-2 px-3 py-1 border-b border-[var(--color-border)]",
        className,
      )}
      data-testid="filter-bar"
    >
      <Search className="h-3.5 w-3.5 text-[color:var(--color-text-tertiary)] shrink-0" aria-hidden="true" />
      <input
        ref={inputRef}
        type="text"
        value={filterText}
        onChange={(e) => setFilterText(paneIndex, e.target.value)}
        placeholder="Filter files... (Cmd+F)"
        className={cn(
          "flex-1 h-6 bg-transparent border-none outline-none",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "placeholder:text-[color:var(--color-text-tertiary)]",
        )}
        aria-label="Filter files"
        data-testid="filter-input"
      />
      {filterText && (
        <button
          className={cn(
            "h-4 w-4 flex items-center justify-center rounded-sm",
            "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={handleClear}
          aria-label="Clear filter"
        >
          <X className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {onContentSearch && (
        <label className="flex items-center gap-1 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] shrink-0 cursor-pointer">
          <input
            type="checkbox"
            checked={searchContents}
            onChange={(e) => setSearchContents(e.target.checked)}
            className="h-3 w-3"
          />
          Contents
        </label>
      )}
      <span
        className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] shrink-0"
        aria-live="polite"
      >
        {filterText
          ? `${filteredCount} of ${totalCount}`
          : `${totalCount} items`}
      </span>
    </div>
  );
}

// ──────────────────────────────────────────────
// SearchPanel - deep recursive search
// ──────────────────────────────────────────────

interface SearchPanelProps {
  onSearch: (query: string, path: string, recursive: boolean) => void;
  onCancelSearch: () => void;
  results: SearchResult[];
  isSearching: boolean;
  onResultClick: (result: SearchResult) => void;
  className?: string;
}

export interface SearchResult {
  path: string;
  name: string;
  is_dir: boolean;
  matchReason: string;
}

export function SearchPanel({
  onSearch,
  onCancelSearch,
  results,
  isSearching,
  onResultClick,
  className,
}: SearchPanelProps) {
  const [query, setQuery] = useState("");
  const [searchPath, setSearchPath] = useState("/");
  const [recursive, setRecursive] = useState(true);
  const [showSaved, setShowSaved] = useState(false);

  const savedSearches = useFileManagerStore((s) => s.savedSearches);
  const addSavedSearch = useFileManagerStore((s) => s.addSavedSearch);
  const removeSavedSearch = useFileManagerStore((s) => s.removeSavedSearch);

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (query.trim()) {
        onSearch(query.trim(), searchPath, recursive);
      }
    },
    [query, searchPath, recursive, onSearch],
  );

  const handleSaveSearch = useCallback(() => {
    if (query.trim()) {
      addSavedSearch({
        id: `search-${Date.now()}`,
        name: query.trim(),
        query: query.trim(),
        path: searchPath,
        recursive,
      });
    }
  }, [query, searchPath, recursive, addSavedSearch]);

  const handleLoadSearch = useCallback(
    (search: SavedSearch) => {
      setQuery(search.query);
      setSearchPath(search.path);
      setRecursive(search.recursive);
      onSearch(search.query, search.path, search.recursive);
      setShowSaved(false);
    },
    [onSearch],
  );

  return (
    <div
      className={cn(
        "flex flex-col border-b border-[var(--color-border)]",
        className,
      )}
      data-testid="search-panel"
    >
      {/* Search form */}
      <form onSubmit={handleSubmit} className="flex items-center gap-2 px-3 py-2">
        <Search className="h-4 w-4 text-[color:var(--color-text-tertiary)] shrink-0" aria-hidden="true" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search files..."
          className={cn(
            "flex-1 h-7 px-2 bg-[var(--color-bg)] rounded-[var(--radius-sm)]",
            "border border-[var(--color-border)]",
            "text-[length:var(--font-size-sm)] text-[color:var(--color-text)]",
            "placeholder:text-[color:var(--color-text-tertiary)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          aria-label="Search query"
          data-testid="search-input"
        />
        <Button
          type="submit"
          size="sm"
          disabled={!query.trim() || isSearching}
          aria-label="Start search"
        >
          {isSearching ? "Searching..." : "Search"}
        </Button>
        {isSearching && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onCancelSearch}
            aria-label="Cancel search"
          >
            <X className="h-3.5 w-3.5" aria-hidden="true" />
          </Button>
        )}
      </form>

      {/* Search options */}
      <div className="flex items-center gap-3 px-3 pb-2">
        <label className="flex items-center gap-1.5 text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">
          <input
            type="checkbox"
            checked={recursive}
            onChange={(e) => setRecursive(e.target.checked)}
            className="h-3 w-3"
          />
          Recursive
        </label>

        <div className="flex items-center gap-1 flex-1 min-w-0">
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">in:</span>
          <input
            type="text"
            value={searchPath}
            onChange={(e) => setSearchPath(e.target.value)}
            className={cn(
              "flex-1 h-5 px-1 bg-transparent border-b border-[var(--color-border)]",
              "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
              "focus-visible:border-[var(--color-primary)] outline-none",
            )}
            aria-label="Search path"
          />
        </div>

        <button
          className={cn(
            "flex items-center gap-1 px-1.5 py-0.5 rounded-[var(--radius-sm)]",
            "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
            "hover:bg-[var(--color-hover-bg)] transition-theme",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={handleSaveSearch}
          aria-label="Save search"
          title="Save this search"
        >
          <Save className="h-3 w-3" aria-hidden="true" />
        </button>

        <button
          className={cn(
            "flex items-center gap-1 px-1.5 py-0.5 rounded-[var(--radius-sm)]",
            "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
            "hover:bg-[var(--color-hover-bg)] transition-theme",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={() => setShowSaved(!showSaved)}
          aria-label="Show saved searches"
          aria-expanded={showSaved}
        >
          <Filter className="h-3 w-3" aria-hidden="true" />
          <ChevronDown className="h-2.5 w-2.5" aria-hidden="true" />
        </button>
      </div>

      {/* Saved searches dropdown */}
      {showSaved && savedSearches.length > 0 && (
        <div className="border-t border-[var(--color-border)] px-3 py-1">
          <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)]">
            Saved Searches
          </span>
          {savedSearches.map((s) => (
            <div
              key={s.id}
              className="flex items-center gap-2 py-1"
            >
              <button
                className={cn(
                  "flex-1 text-left px-2 py-0.5 rounded-[var(--radius-sm)]",
                  "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
                  "hover:bg-[var(--color-hover-bg)] transition-theme",
                )}
                onClick={() => handleLoadSearch(s)}
              >
                {s.name}
              </button>
              <button
                className="text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-error)]"
                onClick={() => removeSavedSearch(s.id)}
                aria-label={`Delete saved search: ${s.name}`}
              >
                <X className="h-3 w-3" aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Search results */}
      {results.length > 0 && (
        <div className="border-t border-[var(--color-border)] max-h-64 overflow-auto">
          <div className="px-3 py-1">
            <Badge variant="secondary">{results.length} results</Badge>
          </div>
          {results.map((result) => (
            <button
              key={result.path}
              className={cn(
                "flex items-center gap-2 w-full px-3 py-1.5 text-left",
                "text-[length:var(--font-size-xs)]",
                "hover:bg-[var(--color-hover-bg)] transition-theme",
                "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
              )}
              onClick={() => onResultClick(result)}
            >
              <span className={cn(
                "truncate flex-1",
                result.is_dir ? "text-[color:var(--color-primary)]" : "text-[color:var(--color-text)]",
              )}>
                {result.name}
              </span>
              <span className="text-[color:var(--color-text-tertiary)] truncate max-w-[200px]">
                {result.path}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// useFileFilter - hook for filtering file lists
// ──────────────────────────────────────────────

/**
 * Filter a file list by text query. Runs in <100ms for typical file lists.
 */
export function filterFiles<T extends { name: string; path: string; extension: string | null }>(
  files: T[],
  filterText: string,
): T[] {
  if (!filterText.trim()) return files;

  const query = filterText.toLowerCase().trim();
  const terms = query.split(/\s+/);

  return files.filter((file) => {
    const name = file.name.toLowerCase();
    const ext = (file.extension || "").toLowerCase();

    return terms.every(
      (term) => name.includes(term) || ext.includes(term),
    );
  });
}
