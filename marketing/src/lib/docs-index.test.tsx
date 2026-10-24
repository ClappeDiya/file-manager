// The published docs, the page renderer and the search index must agree: every search hit has to land on a
// heading that exists on the rendered page.
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DocMarkdown } from '@/components/doc-markdown';
import { DOCS_REPO_BLOB_URL, listDocs, loadAllDocs, resolveDocHref } from '@/lib/docs';
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

const hrefsOf = (html: string) => [...html.matchAll(/<a\s[^>]*?href="([^"]*)"/g)].map((m) => m[1].replace(/&amp;/g, '&'));

describe('resolveDocHref', () => {
  it('maps relative links to published docs onto their site pages, keeping the fragment', () => {
    expect(resolveDocHref('backup-safety.md')).toBe('/docs/backup-safety');
    expect(resolveDocHref('./user-guide.md')).toBe('/docs/user-guide');
    expect(resolveDocHref('backup-policies-and-schedules.md#time-zone')).toBe('/docs/backup-policies-and-schedules#time-zone');
  });

  it('sends links to unpublished repo documents to GitHub', () => {
    expect(resolveDocHref('api-reference.md#ufop-server-rest-api-apiv1')).toBe(`${DOCS_REPO_BLOB_URL}/docs/api-reference.md#ufop-server-rest-api-apiv1`);
    expect(resolveDocHref('./release-checklist.md')).toBe(`${DOCS_REPO_BLOB_URL}/docs/release-checklist.md`);
    expect(resolveDocHref('../README.md#build')).toBe(`${DOCS_REPO_BLOB_URL}/README.md#build`);
    expect(resolveDocHref('duplicati-parity/notes.md')).toBe(`${DOCS_REPO_BLOB_URL}/docs/duplicati-parity/notes.md`);
  });

  it('leaves external, fragment-only, absolute and non-Markdown links alone', () => {
    for (const h of ['https://example.com/a.md', 'mailto:a@b.test', '#install', '/docs/user-guide', 'diagram.png', '../../outside.md'])
      expect(resolveDocHref(h)).toBe(h);
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

  it('has no link left pointing at a repo .md file, and every internal link lands on a published page and heading', async () => {
    const docs = await loadAllDocs();
    const idsBySlug = new Map<string, Set<string>>();
    const htmlBySlug = new Map<string, string>();
    for (const d of docs) {
      const html = renderToStaticMarkup(<DocMarkdown content={d.content} />);
      htmlBySlug.set(d.entry.slug, html);
      idsBySlug.set(d.entry.slug, new Set(idsOf(html)));
    }
    let internal = 0;
    for (const [slug, html] of htmlBySlug) {
      for (const href of hrefsOf(html)) {
        const where = `${slug} -> ${href}`;
        // Only absolute GitHub URLs may name a .md file; nothing on the site itself may.
        if (!/^https:\/\/github\.com\//.test(href)) expect(href.split('#')[0], where).not.toMatch(/\.md$/i);
        const m = /^\/docs\/([^#/]+)(?:#(.+))?$/.exec(href);
        if (m) {
          internal++;
          expect(idsBySlug.has(m[1]), `${where}: unpublished slug`).toBe(true);
          if (m[2]) expect(idsBySlug.get(m[1])!.has(m[2]), `${where}: missing heading`).toBe(true);
        } else if (href.startsWith('#')) {
          expect(idsBySlug.get(slug)!.has(href.slice(1)), `${where}: missing heading`).toBe(true);
        } else {
          expect(href, where).toMatch(/^(https?:|mailto:)/);
        }
      }
    }
    expect(internal).toBeGreaterThan(20);
  });
});
