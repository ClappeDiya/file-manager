import { describe, expect, it } from 'vitest';
import { isFocusShortcut, buildSearchIndex, createSlugger, highlight, hitHref, markdownToText, searchDocs, slugify, splitSections, stripInline, tokenize, type DocsIndex } from './docs-search';

const GUIDE = `# Guide

Intro paragraph about **transfers** and the [admin console](https://example.test/console).

## Table of Contents

- [Install](#install)
- [Install](#install-1)

## Install

Run \`ufop install --all\` to set up.

\`\`\`bash
# not a heading
ufop sync --dry-run
\`\`\`

### Install

Second heading with the same text.

## Sync *and* backup

| Command | Meaning |
|---------|---------|
| \`ufop sync\` | two-way sync |

> Quoted advice: keep snake_case names and *.log patterns.
`;

describe('stripInline', () => {
  it('keeps the text of links, code and emphasis and drops markup, HTML and entities', () => {
    expect(stripInline('See [the **guide**](http://x.test) and `a*b*c` &amp; <b>bold</b> ~~old~~')).toBe('See the guide and a*b*c & bold old');
    expect(stripInline('![alt text](pic.png) and [ref][1]')).toBe('alt text and ref');
    expect(stripInline('keep snake_case_names and 2 * 3 * 4')).toBe('keep snake_case_names and 2 * 3 * 4');
    expect(stripInline('escaped \\*stars\\* and _under_ score')).toBe('escaped *stars* and under score');
  });
});

describe('markdownToText', () => {
  it('flattens lists, quotes, tables and code blocks, dropping fences and rules', () => {
    const md = '- one\n- [x] two\n1. three\n\n> quote\n\n| a | b |\n|---|---|\n| c | d |\n\n---\n\n```sh\n# comment\nls -la\n```\n~~~\ntilde\n~~~';
    expect(markdownToText(md)).toBe('one two three quote a b c d # comment ls -la tilde');
  });

  it('closes a fence only with the same marker', () => {
    expect(markdownToText('```\n~~~\nstill code\n```\nafter')).toBe('~~~ still code after');
  });
});

describe('anchors', () => {
  it('slugifies like GitHub and numbers repeats', () => {
    expect(slugify('Hello, World! (v2)')).toBe('hello-world-v2');
    expect(slugify('Sync & Backup')).toBe('sync--backup');
    expect(slugify('Überblick')).toBe('überblick');
    expect(slugify('???')).toBe('section');
    const s = createSlugger();
    expect([s('Install'), s('Install'), s('Install'), s('Other')]).toEqual(['install', 'install-1', 'install-2', 'other']);
  });
});

describe('splitSections', () => {
  const sections = splitSections('guide', 'Guide', GUIDE);

  it('makes one section per heading with its anchor and plain text', () => {
    expect(sections.map((s) => [s.heading, s.anchor])).toEqual([
      ['Guide', 'guide'],
      ['Install', 'install'],
      ['Install', 'install-1'],
      ['Sync and backup', 'sync-and-backup'],
    ]);
    expect(sections[0].text).toBe('Intro paragraph about transfers and the admin console.');
    expect(sections[3].text).toContain('ufop sync two-way sync');
    expect(sections[3].text).toContain('keep snake_case names and *.log patterns');
  });

  it('skips the table of contents but still counts its heading and the links it holds are not headings', () => {
    expect(sections.some((s) => s.heading === 'Table of Contents')).toBe(false);
  });

  it('indexes code but does not read comments in code blocks as headings', () => {
    const install = sections[1];
    expect(install.text).toContain('ufop sync --dry-run');
    expect(install.text).toContain('# not a heading');
    expect(sections.some((s) => s.heading === 'not a heading')).toBe(false);
  });

  it('keeps text before the first heading as a section without a heading', () => {
    const s = splitSections('d', 'Doc', 'Lead text\n\n## First\nBody');
    expect(s).toEqual([
      { slug: 'd', doc: 'Doc', heading: null, anchor: null, text: 'Lead text' },
      { slug: 'd', doc: 'Doc', heading: 'First', anchor: 'first', text: 'Body' },
    ]);
    expect(hitHref(s[0])).toBe('/docs/d');
    expect(hitHref(s[1])).toBe('/docs/d#first');
    expect(splitSections('e', 'Empty', '')).toEqual([]);
  });

  it('builds one index over several documents', () => {
    const index = buildSearchIndex([
      { slug: 'a', title: 'A', content: '# One\ntext' },
      { slug: 'b', title: 'B', content: '# Two\ntext' },
    ]);
    expect(index.version).toBe(1);
    expect(index.sections.map((s) => s.slug)).toEqual(['a', 'b']);
  });
});

const idx = (docs: Record<string, string>, titles: Record<string, string> = {}): DocsIndex => buildSearchIndex(Object.entries(docs).map(([slug, content]) => ({ slug, title: titles[slug] ?? slug, content })));

describe('tokenize', () => {
  it('lower-cases, splits on punctuation, drops repeats and caps the number of words', () => {
    expect(tokenize('  Ufop-Server  ufop SYNC? ')).toEqual(['ufop', 'server', 'sync']);
    expect(tokenize('')).toEqual([]);
    expect(tokenize('a b c d e f g h i j')).toHaveLength(8);
  });
});

describe('searchDocs', () => {
  const index = idx(
    {
      cli: '# CLI\n## Copy files\nUse ufop copy to copy a folder.\n## Backups\nRun a backup with ufop backup run.',
      guide: '# Guide\n## Backups\nBackups are encrypted. A backup repository keeps versions.\n## Mounting\nMount a remote as a drive.',
      misc: '# Misc\n## Notes\nNothing relevant here, only the word copying once.',
    },
    { cli: 'CLI reference', guide: 'User guide', misc: 'Misc' },
  );

  it('matches the start of words, ignoring case, and requires every word', () => {
    expect(searchDocs(index, 'BACKUP').map((h) => h.section.heading)).toEqual(expect.arrayContaining(['Backups']));
    expect(searchDocs(index, 'mount').map((h) => h.section.heading)).toEqual(['Mounting']);
    expect(searchDocs(index, 'ufop mount')).toEqual([]);
    expect(searchDocs(index, 'ount')).toEqual([]); // not a word start
    expect(searchDocs(index, '')).toEqual([]);
    expect(searchDocs(index, '  ?! ')).toEqual([]);
  });

  it('ranks a heading match above a text match, and a phrase above scattered words', () => {
    const hits = searchDocs(index, 'copy');
    expect(hits[0].section).toMatchObject({ slug: 'cli', heading: 'Copy files' });
    const phrase = idx({
      a: '## Setup\nBackup the repository first, then verify.',
      b: '## Setup\nThe repository backup runs nightly.',
    });
    expect(searchDocs(phrase, 'repository backup')[0].section.slug).toBe('b');
  });

  it('does not let a title match alone pull in every section of a document', () => {
    const hits = searchDocs(index, 'reference');
    expect(hits).toEqual([]);
    const withBody = searchDocs(idx({ cli: '## One\nthe reference\n## Two\nother' }, { cli: 'CLI reference' }), 'cli reference');
    expect(withBody.map((h) => h.section.heading)).toEqual(['One']);
  });

  it('caps hits per document and in total', () => {
    const many = idx({ big: Array.from({ length: 10 }, (_, i) => `## Item ${i}\nbackup item`).join('\n'), small: '## One\nbackup once' });
    const hits = searchDocs(many, 'backup');
    expect(hits.filter((h) => h.section.slug === 'big')).toHaveLength(4);
    expect(hits.some((h) => h.section.slug === 'small')).toBe(true);
    expect(searchDocs(many, 'backup', { limit: 2, perDoc: 10 })).toHaveLength(2);
  });

  it('returns a snippet around the first match with the matches marked', () => {
    const long = idx({ d: `## Topic\n${'filler words '.repeat(40)}the needle sits here ${'more filler '.repeat(40)}` });
    const [hit] = searchDocs(long, 'needle', { snippetLength: 80 });
    const text = hit.snippet.map((s) => s.text).join('');
    expect(text.startsWith('… ')).toBe(true);
    expect(text.endsWith(' …')).toBe(true);
    expect(text.length).toBeLessThan(100);
    expect(hit.snippet.filter((s) => s.hit).map((s) => s.text)).toEqual(['needle']);
    expect(hit.headingSegments).toEqual([{ text: 'Topic', hit: false }]);
  });

  it('marks matches in the heading and document title too, and starts a heading-only match at the top of the text', () => {
    // "guide" is only in the title and "mount" in the heading: every word is found, so the section matches.
    const [hit] = searchDocs(idx({ d: '## Mounting drives\nAnything at all about disks.' }, { d: 'User guide' }), 'mount guide');
    expect(hit.docSegments.filter((s) => s.hit).map((s) => s.text)).toEqual(['guide']);
    expect(hit.headingSegments!.filter((s) => s.hit).map((s) => s.text)).toEqual(['Mount']);
    const [h2] = searchDocs(idx({ d: '## Mounting drives\nAnything at all about disks.' }, { d: 'User guide' }), 'mounting');
    expect(h2.headingSegments).toEqual([
      { text: 'Mounting', hit: true },
      { text: ' drives', hit: false },
    ]);
    expect(h2.snippet.map((s) => s.text).join('')).toBe('Anything at all about disks.');
    expect(h2.docSegments).toEqual([{ text: 'User guide', hit: false }]);
  });
});

describe('highlight', () => {
  it('splits text into marked and plain pieces that rebuild the original', () => {
    const text = 'Back up with backups and Backup tools';
    const segs = highlight(text, ['backup', 'back']);
    expect(segs.map((s) => s.text).join('')).toBe(text);
    expect(segs.filter((s) => s.hit).map((s) => s.text)).toEqual(['Back', 'backup', 'Backup']);
    expect(highlight('nothing here', ['zzz'])).toEqual([{ text: 'nothing here', hit: false }]);
    expect(highlight('', ['a'])).toEqual([]);
  });

  it('merges overlapping matches and keeps non-ASCII letters', () => {
    expect(highlight('Überblick über alles', ['über']).filter((s) => s.hit).map((s) => s.text)).toEqual(['Über', 'über']);
  });
});

describe('isFocusShortcut', () => {
  const ev = (o: Partial<Parameters<typeof isFocusShortcut>[0]> = {}) => ({ key: '/', ctrlKey: false, metaKey: false, altKey: false, defaultPrevented: false, target: { tagName: 'BODY' }, ...o });
  it('focuses the search on a bare slash', () => {
    expect(isFocusShortcut(ev())).toBe(true);
    expect(isFocusShortcut(ev({ target: null }))).toBe(true);
  });
  it('leaves slashes typed into fields, with modifiers, handled elsewhere or other keys alone', () => {
    expect(isFocusShortcut(ev({ target: { tagName: 'INPUT' } }))).toBe(false);
    expect(isFocusShortcut(ev({ target: { tagName: 'textarea' } }))).toBe(false);
    expect(isFocusShortcut(ev({ target: { tagName: 'DIV', isContentEditable: true } }))).toBe(false);
    expect(isFocusShortcut(ev({ ctrlKey: true }))).toBe(false);
    expect(isFocusShortcut(ev({ metaKey: true }))).toBe(false);
    expect(isFocusShortcut(ev({ altKey: true }))).toBe(false);
    expect(isFocusShortcut(ev({ defaultPrevented: true }))).toBe(false);
    expect(isFocusShortcut(ev({ key: '?' }))).toBe(false);
  });
});
