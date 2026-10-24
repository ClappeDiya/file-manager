// The published docs, the page renderer and the search index must agree: every search hit has to land on a
// heading that exists on the rendered page.
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DocMarkdown } from '@/components/doc-markdown';
import { listDocs, loadAllDocs } from '@/lib/docs';
import { buildSearchIndex, searchDocs, splitSections } from '@/lib/docs-search';
import { GET } from '@/app/docs/search-index.json/route';

const idsOf = (html: string) => [...html.matchAll(/<h[1-6][^>]*\sid="([^"]*)"/g)].map((m) => m[1]);

describe('DocMarkdown', () => {
  it('gives headings the anchors the indexer computes, numbering repeats alike', () => {
    const md = '# Title\n\n## Install\n\ntext\n\n## `ufop` **sync** & [more](http://x.test)\n\n### Install\n\n```\n# not a heading\n```\n\n## Install\n';
    const html = renderToStaticMarkup(<DocMarkdown content={md} />);
    expect(idsOf(html)).toEqual(['title', 'install', 'ufop-sync--more', 'install-1', 'install-2']);
    expect(splitSections('d', 'D', md).map((s) => s.anchor)).toEqual(idsOf(html));
  });
});

describe('published docs index', () => {
  it('lists every published document and a section for each', async () => {
    const published = await listDocs();
    const docs = await loadAllDocs();
    expect(published.length).toBeGreaterThan(0);
    // A published file that cannot be read would silently vanish from search: fail instead.
    expect(docs.map((d) => d.entry.slug)).toEqual(published.map((d) => d.slug));
    const index = buildSearchIndex(docs.map((d) => ({ slug: d.entry.slug, title: d.entry.title, content: d.content })));
    for (const d of docs) expect(index.sections.filter((s) => s.slug === d.entry.slug).length, d.entry.slug).toBeGreaterThan(0);
  });

  it('only links to headings that exist on the rendered pages', async () => {
    const docs = await loadAllDocs();
    for (const d of docs) {
      const ids = new Set(idsOf(renderToStaticMarkup(<DocMarkdown content={d.content} />)));
      for (const s of splitSections(d.entry.slug, d.entry.title, d.content)) {
        if (s.anchor) expect(ids.has(s.anchor), `${d.entry.slug}#${s.anchor}`).toBe(true);
      }
    }
  });

  it('is served by the static index route and finds real pages', async () => {
    const res = await GET();
    expect(res.status).toBe(200);
    const index = await res.json();
    expect(index.version).toBe(1);
    expect(index.sections.length).toBeGreaterThan(100);
    const top = searchDocs(index, 'ransomware')[0];
    expect(top.section.slug).toBe('backup-ransomware-hardening');
    const cli = searchDocs(index, 'completions');
    expect(cli.length).toBeGreaterThan(0);
    expect(cli[0].snippet.some((s) => s.hit)).toBe(true);
  });
});
