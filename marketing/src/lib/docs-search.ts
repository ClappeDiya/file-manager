// Search over the published docs: the indexer (Markdown -> sections) and the scorer.
//
// No dependencies and no Node/DOM imports: the index is built at build time by the
// /docs/search-index.json route from the same Markdown the docs pages render, and
// the browser runs `searchDocs` over it. A section is one heading and the text
// under it, so a hit links straight to the heading (`#anchor`).

export interface DocSection {
  /** Published slug of the document (/docs/<slug>). */
  slug: string;
  /** Title of the document. */
  doc: string;
  /** Heading text, or null for text before the first heading. */
  heading: string | null;
  /** Fragment of the heading on the docs page, or null without a heading. */
  anchor: string | null;
  /** Plain text under the heading (Markdown syntax removed). */
  text: string;
}

export interface DocsIndex {
  version: 1;
  sections: DocSection[];
}

/** A piece of text and whether it matched the query (rendered as <mark>). */
export interface Segment {
  text: string;
  hit: boolean;
}

export interface SearchHit {
  section: DocSection;
  score: number;
  /** The document title with the matches marked. */
  docSegments: Segment[];
  /** The heading with the matches marked (null without a heading). */
  headingSegments: Segment[] | null;
  /** An excerpt of the section with the matches marked. */
  snippet: Segment[];
}

export interface SearchOptions {
  /** Hits returned (default 20). */
  limit?: number;
  /** Hits from one document (default 4), so one long guide cannot fill the list. */
  perDoc?: number;
  /** Characters in a snippet (default 160). */
  snippetLength?: number;
}

// ------------------------------------------------------------------------------------------
// Markdown -> text
// ------------------------------------------------------------------------------------------

const ENTITIES: Record<string, string> = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'", '&nbsp;': ' ' };

function stripEmphasis(s: string): string {
  // Escaped characters are set aside so that `\*not emphasis\*` survives the emphasis rules.
  return s
    .replace(/\\([\\`*_{}[\]()#+\-.!|<>~])/g, (_, c: string) => `\uE000${c.charCodeAt(0).toString(16)}\uE001`)
    .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, '$2')
    .replace(/\*(?=\S)([^*]*?\S)\*/g, '$1')
    .replace(/(^|[^\p{L}\p{N}_])_(?=\S)([^_]*?\S)_(?![\p{L}\p{N}_])/gu, '$1$2')
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '$1')
    .replace(/\uE000([0-9a-f]+)\uE001/g, (_, h: string) => String.fromCharCode(parseInt(h, 16)));
}

/** Inline Markdown (links, code, emphasis, HTML) reduced to its text. */
export function stripInline(md: string): string {
  const noLinks = md
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\[[^\]]*\]/g, '$1')
    .replace(/<(https?:\/\/[^>\s]+)>/g, '$1')
    .replace(/<\/?[A-Za-z][^>]*>/g, '');
  // Code spans are kept verbatim (their `*` and `_` are not emphasis).
  const text = noLinks
    .split(/(`[^`]+`)/)
    .map((part, i) => (i % 2 ? part.slice(1, -1) : stripEmphasis(part)))
    .join('')
    .replace(/&(?:amp|lt|gt|quot|#39|nbsp);/g, (e) => ENTITIES[e] ?? e);
  return text.replace(/\s+/g, ' ').trim();
}

const FENCE = /^\s*(```+|~~~+)/;
const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;
const THEMATIC_BREAK = /^\s*([-*_])(\s*\1){2,}\s*$/;

/** Block Markdown (lists, quotes, tables, code blocks) reduced to plain text lines joined by spaces. */
export function markdownToText(md: string): string {
  const out: string[] = [];
  let fence: string | null = null;
  for (const raw of md.split(/\r?\n/)) {
    const f = FENCE.exec(raw);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      else if (raw.trim()) out.push(raw.trim());
      continue;
    }
    if (f) {
      fence = f[1];
      continue;
    }
    if (TABLE_RULE.test(raw) || THEMATIC_BREAK.test(raw)) continue;
    const line = raw
      .replace(/^\s*>+\s?/, '')
      .replace(/^\s*([-*+]|\d+[.)])\s+(\[[ xX]\]\s+)?/, '')
      .replace(/^\s*#{1,6}\s+/, '')
      .replace(/\|/g, ' ');
    const text = stripInline(line);
    if (text) out.push(text);
  }
  return out.join(' ');
}

// ------------------------------------------------------------------------------------------
// Heading anchors
// ------------------------------------------------------------------------------------------

/** GitHub-style anchor of a heading's text. */
export function slugify(text: string): string {
  const s = text
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, '')
    .replace(/ /g, '-');
  return s || 'section';
}

/** Anchors for the headings of one document, in document order; repeated headings get -1, -2, … */
export function createSlugger(): (text: string) => string {
  const seen = new Map<string, number>();
  return (text) => {
    const base = slugify(text);
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return n === 0 ? base : `${base}-${n}`;
  };
}

// ------------------------------------------------------------------------------------------
// Indexer
// ------------------------------------------------------------------------------------------

const HEADING = /^(#{1,6})\s+(.+?)(?:\s+#+)?\s*$/;
const SKIPPED_HEADINGS = /^(table of contents|contents)$/i;

/** The sections of one Markdown document. A table of contents is left out (it only repeats the headings). */
export function splitSections(slug: string, doc: string, markdown: string): DocSection[] {
  const sections: DocSection[] = [];
  const anchor = createSlugger();
  let heading: string | null = null;
  let id: string | null = null;
  let body: string[] = [];
  let fence: string | null = null;

  const flush = () => {
    const text = markdownToText(body.join('\n'));
    if ((heading !== null || text) && !(heading !== null && SKIPPED_HEADINGS.test(heading))) sections.push({ slug, doc, heading, anchor: id, text });
    body = [];
  };

  for (const line of markdown.split(/\r?\n/)) {
    const f = FENCE.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      body.push(line);
      continue;
    }
    if (f) {
      fence = f[1];
      body.push(line);
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      flush();
      heading = stripInline(h[2]);
      // Every heading takes its anchor, also a skipped one, so the numbering matches the page.
      id = anchor(heading);
      continue;
    }
    body.push(line);
  }
  flush();
  return sections;
}

export function buildSearchIndex(docs: { slug: string; title: string; content: string }[]): DocsIndex {
  return { version: 1, sections: docs.flatMap((d) => splitSections(d.slug, d.title, d.content)) };
}

// ------------------------------------------------------------------------------------------
// Scorer
// ------------------------------------------------------------------------------------------

const WORD_CHAR = /[\p{L}\p{N}]/u;
const MAX_TERMS = 8;

/** Lower-case words of a query (letters and digits), without repeats. */
export function tokenize(query: string): string[] {
  const terms = query
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  return [...new Set(terms)].slice(0, MAX_TERMS);
}

const lowered = (s: string) => {
  const l = s.toLowerCase();
  // A few characters change length when lower-cased; offsets must stay valid for the original.
  return l.length === s.length ? l : s;
};

const cache = new WeakMap<DocSection, { doc: string; heading: string; text: string }>();
function lowerOf(s: DocSection) {
  let c = cache.get(s);
  if (!c) {
    c = { doc: lowered(s.doc), heading: lowered(s.heading ?? ''), text: lowered(s.text) };
    cache.set(s, c);
  }
  return c;
}

/** Index of the first word in `hay` that starts with `term`, from `from`; -1 if none. */
function wordStart(hay: string, term: string, from = 0): number {
  for (let i = hay.indexOf(term, from); i !== -1; i = hay.indexOf(term, i + 1)) {
    if (i === 0 || !WORD_CHAR.test(hay[i - 1])) return i;
  }
  return -1;
}

/** Words of `hay` starting with `term` (counted up to `cap`), and whether one of them is exactly `term`. */
function wordMatches(hay: string, term: string, cap: number): { count: number; exact: boolean } {
  let count = 0;
  let exact = false;
  for (let i = wordStart(hay, term); i !== -1 && count < cap; i = wordStart(hay, term, i + 1)) {
    count++;
    const after = hay[i + term.length];
    if (after === undefined || !WORD_CHAR.test(after)) exact = true;
  }
  return { count, exact };
}

function scoreSection(s: DocSection, terms: string[], phrase: string): number {
  const l = lowerOf(s);
  let score = 0;
  let own = false;
  for (const t of terms) {
    const h = wordMatches(l.heading, t, 1);
    const d = wordMatches(l.doc, t, 1);
    const x = wordMatches(l.text, t, 6);
    if (!h.count && !d.count && !x.count) return 0;
    if (h.count || x.count) own = true;
    if (h.count) score += 12 + (h.exact ? 6 : 0);
    if (d.count) score += 3;
    if (x.count) score += x.count + (x.exact ? 2 : 0);
  }
  // The document title alone must not pull in every section of that document.
  if (!own) return 0;
  if (terms.length > 1) {
    if (l.heading.includes(phrase)) score += 15;
    if (l.text.includes(phrase)) score += 8;
  }
  return score;
}

/** `text` split into pieces, marking the words that start with one of the terms. */
export function highlight(text: string, terms: string[]): Segment[] {
  const l = lowered(text);
  const ranges: [number, number][] = [];
  for (const t of terms) {
    for (let i = wordStart(l, t); i !== -1; i = wordStart(l, t, i + t.length)) ranges.push([i, i + t.length]);
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged: [number, number][] = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  const out: Segment[] = [];
  let at = 0;
  for (const [a, b] of merged) {
    if (a > at) out.push({ text: text.slice(at, a), hit: false });
    out.push({ text: text.slice(a, b), hit: true });
    at = b;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out;
}

/** An excerpt of the section text around the first match, cut at word boundaries. */
function excerpt(s: DocSection, terms: string[], phrase: string, length: number): string {
  const text = s.text;
  const l = lowerOf(s).text;
  let pos = terms.length > 1 ? l.indexOf(phrase) : -1;
  if (pos === -1) {
    for (const t of terms) {
      const i = wordStart(l, t);
      if (i !== -1 && (pos === -1 || i < pos)) pos = i;
    }
  }
  if (pos === -1) pos = 0;
  let start = Math.max(0, pos - Math.floor(length / 4));
  if (start > 0) {
    const sp = text.indexOf(' ', start);
    if (sp !== -1 && sp < pos) start = sp + 1;
  }
  let end = Math.min(text.length, start + length);
  if (end < text.length) {
    const sp = text.lastIndexOf(' ', end);
    if (sp > pos) end = sp;
  }
  return `${start > 0 ? '… ' : ''}${text.slice(start, end).trim()}${end < text.length ? ' …' : ''}`;
}

/**
 * Sections matching every word of `query` (as the start of a word, any case), best first. A word in a
 * heading counts most, then in the text; a phrase match adds to that. Returns nothing for an empty query.
 */
export function searchDocs(index: DocsIndex, query: string, o: SearchOptions = {}): SearchHit[] {
  const terms = tokenize(query);
  if (!terms.length) return [];
  const phrase = terms.join(' ');
  const limit = o.limit ?? 20;
  const perDoc = o.perDoc ?? 4;
  const scored = index.sections
    .map((section, order) => ({ section, order, score: scoreSection(section, terms, phrase) }))
    .filter((h) => h.score > 0)
    .sort((a, b) => b.score - a.score || a.order - b.order);
  const perDocCount = new Map<string, number>();
  const hits: SearchHit[] = [];
  for (const h of scored) {
    const n = perDocCount.get(h.section.slug) ?? 0;
    if (n >= perDoc) continue;
    perDocCount.set(h.section.slug, n + 1);
    hits.push({
      section: h.section,
      score: h.score,
      docSegments: highlight(h.section.doc, terms),
      headingSegments: h.section.heading === null ? null : highlight(h.section.heading, terms),
      snippet: highlight(excerpt(h.section, terms, phrase, o.snippetLength ?? 160), terms),
    });
    if (hits.length >= limit) break;
  }
  return hits;
}

/** Where a hit links to. */
export function hitHref(s: DocSection): string {
  return `/docs/${s.slug}${s.anchor ? `#${s.anchor}` : ''}`;
}

/** The parts of a keyboard event that decide whether `/` should focus the search box. */
export interface ShortcutEvent {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  defaultPrevented: boolean;
  target: { tagName?: string; isContentEditable?: boolean } | null;
}

/** `/` focuses the search unless a modifier is held or the key is going to a text field. */
export function isFocusShortcut(e: ShortcutEvent): boolean {
  if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return false;
  const t = e.target;
  return !(t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes((t.tagName ?? '').toUpperCase())));
}
