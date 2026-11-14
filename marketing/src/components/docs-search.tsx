'use client';

import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import Link from 'next/link';
import { Search } from 'lucide-react';
import { hitHref, isFocusShortcut, searchDocs, tokenize, type DocsIndex, type Segment } from '@/lib/docs-search';

const INDEX_URL = '/docs/search-index.json';

function Marked({ segments }: { segments: Segment[] }) {
  return (
    <>
      {segments.map((s, i) =>
        s.hit ? (
          <mark key={i} className="rounded-sm bg-primary/20 px-0.5 text-foreground">
            {s.text}
          </mark>
        ) : (
          <span key={i}>{s.text}</span>
        ),
      )}
    </>
  );
}

/** Search box over the published docs; press `/` anywhere on the page to focus it. */
export function DocsSearch() {
  const router = useRouter();
  const inputRef = useRef<HTMLInputElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const listId = useId();
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState<DocsIndex | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const requested = useRef(false);

  const load = useCallback(() => {
    if (requested.current) return;
    requested.current = true;
    fetch(INDEX_URL)
      .then((r) => (r.ok ? (r.json() as Promise<DocsIndex>) : Promise.reject(new Error(String(r.status)))))
      .then(setIndex)
      .catch(() => {
        requested.current = false;
        setFailed(true);
      });
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isFocusShortcut({ key: e.key, ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey, defaultPrevented: e.defaultPrevented, target: e.target as HTMLElement | null })) return;
      e.preventDefault();
      inputRef.current?.focus();
      inputRef.current?.select();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, []);

  const hits = useMemo(() => (index ? searchDocs(index, query) : []), [index, query]);
  const searching = tokenize(query).length > 0 && query.trim().length >= 2;
  const shown = open && searching;
  const current = Math.min(active, Math.max(0, hits.length - 1));

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      if (query) setQuery('');
      else inputRef.current?.blur();
      setOpen(false);
    } else if (e.key === 'ArrowDown' && hits.length) {
      e.preventDefault();
      setOpen(true);
      setActive((current + 1) % hits.length);
    } else if (e.key === 'ArrowUp' && hits.length) {
      e.preventDefault();
      setActive((current - 1 + hits.length) % hits.length);
    } else if (e.key === 'Enter' && shown && hits[current]) {
      e.preventDefault();
      setOpen(false);
      router.push(hitHref(hits[current].section));
    }
  };

  return (
    <div ref={boxRef} className="relative mb-8 max-w-2xl" role="search">
      <label htmlFor={`${listId}-input`} className="sr-only">
        Search the documentation
      </label>
      <Search size={16} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-foreground-secondary" aria-hidden />
      <input
        id={`${listId}-input`}
        ref={inputRef}
        type="search"
        autoComplete="off"
        spellCheck={false}
        placeholder="Search the docs"
        value={query}
        role="combobox"
        aria-expanded={shown}
        aria-controls={listId}
        aria-activedescendant={shown && hits[current] ? `${listId}-${current}` : undefined}
        onFocus={() => {
          load();
          setOpen(true);
        }}
        onChange={(e) => {
          load();
          setQuery(e.target.value);
          setActive(0);
          setOpen(true);
        }}
        onKeyDown={onKeyDown}
        className="w-full rounded-md border border-border bg-background py-2 pl-9 pr-10 text-sm text-foreground placeholder:text-foreground-secondary focus:outline-none focus:ring-2 focus:ring-primary"
      />
      {!query && (
        <kbd className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 rounded border border-border bg-background-secondary px-1.5 text-xs text-foreground-secondary" aria-hidden>
          /
        </kbd>
      )}

      <div id={listId} role="listbox" aria-label="Search results" hidden={!shown} className="absolute left-0 right-0 z-20 mt-2 max-h-[28rem] overflow-y-auto rounded-md border border-border bg-background-elevated shadow-lg">
        {shown && failed && !index && <p className="px-4 py-3 text-sm text-foreground-secondary">The search index could not be loaded. Reload the page to try again.</p>}
        {shown && !failed && !index && <p className="px-4 py-3 text-sm text-foreground-secondary">Loading the search index…</p>}
        {shown && index && !hits.length && <p className="px-4 py-3 text-sm text-foreground-secondary">No page matches “{query.trim()}”. Try fewer or different words.</p>}
        {shown &&
          hits.map((h, i) => (
            <Link
              key={`${h.section.slug}#${h.section.anchor ?? ''}`}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === current}
              href={hitHref(h.section)}
              onClick={() => setOpen(false)}
              onMouseEnter={() => setActive(i)}
              className={`block border-b border-border px-4 py-3 last:border-b-0 ${i === current ? 'bg-background-secondary' : ''}`}
            >
              <span className="block text-xs text-foreground-secondary">
                <Marked segments={h.docSegments} />
              </span>
              {h.headingSegments && (
                <span className="block text-sm font-semibold text-foreground">
                  <Marked segments={h.headingSegments} />
                </span>
              )}
              <span className="mt-0.5 block text-sm leading-relaxed text-foreground-secondary">
                <Marked segments={h.snippet} />
              </span>
            </Link>
          ))}
      </div>
      <p className="sr-only" aria-live="polite">
        {shown && index ? `${hits.length} result${hits.length === 1 ? '' : 's'}` : ''}
      </p>
    </div>
  );
}
