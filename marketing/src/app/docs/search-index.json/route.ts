import { NextResponse } from 'next/server';
import { loadAllDocs } from '@/lib/docs';
import { buildSearchIndex } from '@/lib/docs-search';

// Built once at `next build` from the same published Markdown the docs pages render, and served as a
// static file; the search box downloads it on first use.
export const dynamic = 'force-static';

export async function GET() {
  const docs = await loadAllDocs();
  const index = buildSearchIndex(docs.map((d) => ({ slug: d.entry.slug, title: d.entry.title, content: d.content })));
  return NextResponse.json(index, { headers: { 'cache-control': 'public, max-age=3600' } });
}
